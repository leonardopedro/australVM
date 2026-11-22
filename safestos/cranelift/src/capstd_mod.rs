//! Capability-based file access for modhost and Rust modules (B10).
//!
//! Tier 1 (in-process): cap-std as declared-intent + defense-in-depth.
//! Tier 2 (VM): the cloud-hypervisor guest is the hard boundary.
//!
//! HONESTY NOTE: Tier 1 cap-std is NOT a sandbox for untrusted Rust code.
//! A module with `unsafe` or `std::fs` can bypass cap-std entirely. cap-std
//! defines the *coupling* (which capabilities cross the boundary); the VM
//! provides the *isolation* for genuinely untrusted code.

#[cfg(feature = "capstd")]
use cap_std::ambient_authority;
#[cfg(feature = "capstd")]
use cap_std::fs::Dir;
#[cfg(feature = "capstd")]
use std::path::Path;

#[cfg(feature = "capstd")]
#[derive(Debug)]
pub struct CapFs {
    root: Dir,
}

#[cfg(feature = "capstd")]
impl CapFs {
    pub fn open(root_path: &Path) -> Result<Self, String> {
        let root = Dir::open_ambient_dir(root_path, ambient_authority())
            .map_err(|e| format!("cannot open root dir {}: {e}", root_path.display()))?;
        Ok(Self { root })
    }

    pub fn read_file(&self, relative: &str) -> Result<Vec<u8>, String> {
        self.root
            .read(relative)
            .map_err(|e| format!("read '{relative}': {e}"))
    }

    pub fn write_file(&self, relative: &str, data: &[u8]) -> Result<(), String> {
        self.root
            .write(relative, data)
            .map_err(|e| format!("write '{relative}': {e}"))
    }

    pub fn list_dir(&self, relative: &str) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for entry in self
            .root
            .read_dir(relative)
            .map_err(|e| format!("list '{relative}': {e}"))?
        {
            let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        Ok(names)
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.root.exists(relative)
    }
}

#[cfg(not(feature = "capstd"))]
#[derive(Debug)]
pub struct CapFs;

#[cfg(not(feature = "capstd"))]
impl CapFs {
    pub fn open(_root_path: &std::path::Path) -> Result<Self, String> {
        Err("capstd feature not enabled — rebuild with --features capstd".to_string())
    }
}

// ── D09: instance capabilities ──────────────────────────────────────────────
//
// The host constructs one set of capabilities per loaded instance and hands the
// adapter **handles**, never a path it could re-root and never the right to dial
// an arbitrary host. Everything below is the Tier 1 coupling described at the
// top of this file: a module that can call `std::fs` directly is not contained
// by this, and the VM tier is what contains it.

pub mod caps {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// Largest frame one `send` will accept, and the largest response one
    /// `query` will hand back. Stated once so the check and the message agree.
    pub const MAX_FRAME: usize = 1 << 20;
    /// A directory read is bounded by the same limit: a capability that returns
    /// unbounded bytes is a way to exhaust the caller's memory.
    pub const MAX_READ: usize = 1 << 20;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        Dir,
        Net,
    }

    /// One request that was accepted for delivery. It carries owned bytes — no
    /// borrowed pointer is retained across the boundary — and lives until it is
    /// settled, cancelled or dropped.
    #[derive(Debug, Clone)]
    pub struct Request {
        pub id: u64,
        pub peer: String,
        pub payload: Vec<u8>,
        pub response: Option<Vec<u8>>,
        pub cancelled: bool,
    }

    #[derive(Debug)]
    struct Handle {
        kind: Kind,
        owner: String,
        pending: HashMap<u64, Request>,
    }

    #[derive(Debug, Default)]
    struct Table {
        next: u64,
        handles: HashMap<u64, Handle>,
    }

    /// The capabilities of one loaded instance.
    #[derive(Debug)]
    pub struct InstanceCaps {
        /// The module this set belongs to; a handle is only usable by it.
        pub module: String,
        /// The instance state directory every relative path resolves under.
        root: PathBuf,
        /// Exactly the peers this instance may address (`[module] node`).
        peers: Vec<String>,
        /// Written by `send`, so a test can observe whether a sink call happened.
        sink: PathBuf,
        table: Mutex<Table>,
    }

    impl InstanceCaps {
        /// Build the set for `module`, rooted at `state_dir`. The directory is
        /// canonicalised once, here: every later path is checked against the
        /// canonical root, so a symlinked state directory cannot widen it later.
        pub fn new(
            module: impl Into<String>,
            state_dir: &Path,
            peers: Vec<String>,
        ) -> Result<Self, String> {
            std::fs::create_dir_all(state_dir)
                .map_err(|e| format!("cannot create state dir {}: {e}", state_dir.display()))?;
            let root = std::fs::canonicalize(state_dir)
                .map_err(|e| format!("cannot resolve state dir {}: {e}", state_dir.display()))?;
            let mut allowed = Vec::new();
            for peer in peers {
                let peer = peer.trim().to_string();
                if peer.is_empty() {
                    return Err("a configured peer id must not be empty".to_string());
                }
                if !allowed.contains(&peer) {
                    allowed.push(peer);
                }
            }
            Ok(Self {
                module: module.into(),
                sink: root.join("sink.log"),
                root,
                peers: allowed,
                table: Mutex::new(Table::default()),
            })
        }

        /// The peers this instance may address. Empty means it may address none:
        /// no `[module] node` is not a permission to dial anyone.
        pub fn peers(&self) -> &[String] {
            &self.peers
        }

        pub fn root(&self) -> &Path {
            &self.root
        }

        /// Open a capability handle owned by this instance's module.
        pub fn open(&self, kind: Kind) -> Result<u64, String> {
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            table.next += 1;
            let id = table.next;
            table.handles.insert(
                id,
                Handle {
                    kind,
                    owner: self.module.clone(),
                    pending: HashMap::new(),
                },
            );
            Ok(id)
        }

        /// A handle used by another module is refused: the capability was issued
        /// to one instance, and a number is not a capability if any neighbour can
        /// present it.
        fn with_handle<T>(
            &self,
            handle: u64,
            caller: &str,
            want: Kind,
            f: impl FnOnce(&mut Handle) -> Result<T, String>,
        ) -> Result<T, String> {
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let entry = table
                .handles
                .get_mut(&handle)
                .ok_or_else(|| format!("no capability handle {handle}"))?;
            if entry.owner != caller {
                return Err(format!(
                    "capability handle {handle} belongs to '{}', not to '{caller}'",
                    entry.owner
                ));
            }
            if entry.kind != want {
                return Err(format!("capability handle {handle} is not a {want:?} handle"));
            }
            f(entry)
        }

        /// A relative path under the state directory. Absolute paths, `..` and a
        /// resolved path that escapes the root are all refused before any I/O.
        fn resolve(&self, relative: &str) -> Result<PathBuf, String> {
            if relative.trim().is_empty() {
                return Err("empty path".to_string());
            }
            if relative.starts_with('/') {
                return Err(format!("absolute path '{relative}' is not under the state dir"));
            }
            if relative.split('/').any(|part| part == "..") {
                return Err(format!("path '{relative}' escapes the state dir"));
            }
            let joined = self.root.join(relative);
            // The lexical check above is not enough for a symlink inside the
            // state dir; resolve the parent where it exists and require the same
            // root. A path that does not exist yet (a write) is checked lexically.
            if let Some(parent) = joined.parent() {
                if parent.exists() {
                    let resolved = std::fs::canonicalize(parent)
                        .map_err(|e| format!("cannot resolve {}: {e}", parent.display()))?;
                    if !resolved.starts_with(&self.root) {
                        return Err(format!("path '{relative}' escapes the state dir"));
                    }
                }
            }
            Ok(joined)
        }

        pub fn write(&self, handle: u64, caller: &str, relative: &str, data: &[u8]) -> Result<u64, String> {
            if data.len() > MAX_FRAME {
                return Err(format!("write of {} bytes exceeds the {MAX_FRAME}-byte limit", data.len()));
            }
            self.with_handle(handle, caller, Kind::Dir, |_| {
                let path = self.resolve(relative)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                std::fs::write(&path, data)
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                Ok(data.len() as u64)
            })
        }

        pub fn read(&self, handle: u64, caller: &str, relative: &str) -> Result<Vec<u8>, String> {
            self.with_handle(handle, caller, Kind::Dir, |_| {
                let path = self.resolve(relative)?;
                let meta = std::fs::metadata(&path)
                    .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
                if meta.len() as usize > MAX_READ {
                    return Err(format!(
                        "{} is {} bytes, over the {MAX_READ}-byte limit",
                        path.display(),
                        meta.len()
                    ));
                }
                std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
            })
        }

        /// Accept one bounded frame for `peer`. A peer outside the configured set
        /// is refused *here*, before anything is delivered: the adapter has no
        /// permission to dial arbitrary hosts.
        pub fn send(
            &self,
            handle: u64,
            caller: &str,
            peer: &str,
            payload: &[u8],
        ) -> Result<u64, String> {
            if payload.len() > MAX_FRAME {
                return Err(format!(
                    "frame of {} bytes exceeds the {MAX_FRAME}-byte limit",
                    payload.len()
                ));
            }
            if !self.peers.iter().any(|p| p == peer) {
                return Err(format!(
                    "peer '{peer}' is not configured for '{}' (allowed: {:?})",
                    self.module, self.peers
                ));
            }
            self.with_handle(handle, caller, Kind::Net, |entry| {
                let id = entry.pending.keys().max().copied().unwrap_or(0) + 1;
                entry.pending.insert(
                    id,
                    Request {
                        id,
                        peer: peer.to_string(),
                        // Owned copy: nothing borrowed from the caller survives
                        // the call.
                        payload: payload.to_vec(),
                        response: None,
                        cancelled: false,
                    },
                );
                Ok(id)
            })
        }

        /// Settle a pending request. The transport (or, in tests, the demo sink)
        /// is the only caller: a module cannot answer its own request.
        /// Called by the transport, not by the module: a caller presenting its
        /// own request id would otherwise be able to answer itself. That is why
        /// there is no `caller` argument to check here.
        pub fn settle(&self, handle: u64, _caller: &str, id: u64, response: &[u8]) -> Result<(), String> {
            let delivered = {
                let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
                let entry = table
                    .handles
                    .get_mut(&handle)
                    .ok_or_else(|| format!("no capability handle {handle}"))?;
                match entry.pending.get_mut(&id) {
                    None => return Err(format!("no pending request {id}")),
                    Some(request) if request.cancelled => {
                        return Err(format!("request {id} was cancelled"))
                    }
                    Some(request) => {
                        request.response = Some(response.to_vec());
                        (request.peer.clone(), request.payload.clone())
                    }
                }
            };
            self.deliver(&delivered.0, &delivered.1, response)
        }

        /// The demo sink: append the frame and the response to `sink.log` in the
        /// state directory. A refused or cancelled request never reaches it, which
        /// is what the tests observe.
        fn deliver(&self, peer: &str, payload: &[u8], response: &[u8]) -> Result<(), String> {
            use std::io::Write;
            let line = format!(
                "peer={peer} sent={} got={}\n",
                payload.len(),
                response.len()
            );
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.sink)
                .map_err(|e| format!("cannot open sink {}: {e}", self.sink.display()))?;
            file.write_all(line.as_bytes())
                .map_err(|e| format!("cannot append to sink: {e}"))
        }

        pub fn query(&self, handle: u64, caller: &str, id: u64) -> Result<Option<Vec<u8>>, String> {
            self.with_handle(handle, caller, Kind::Net, |entry| match entry.pending.get(&id) {
                None => Err(format!("no pending request {id}")),
                Some(request) if request.cancelled => Err(format!("request {id} was cancelled")),
                Some(request) => Ok(request.response.clone()),
            })
        }

        /// Wait for a request to settle, up to `timeout_ms`. Bounded on purpose:
        /// a module that can block the host forever is a denial of service.
        pub fn wait(
            &self,
            handle: u64,
            caller: &str,
            id: u64,
            timeout_ms: u64,
        ) -> Result<Option<Vec<u8>>, String> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
            loop {
                match self.query(handle, caller, id)? {
                    Some(response) => return Ok(Some(response)),
                    None if std::time::Instant::now() >= deadline => return Ok(None),
                    None => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        }

        /// Cancel a request that has not settled. Its frame is dropped: no `send`
        /// is retried behind the caller's back, so a dropped request cannot become
        /// an unobserved effect.
        pub fn cancel(&self, handle: u64, caller: &str, id: u64) -> Result<(), String> {
            self.with_handle(handle, caller, Kind::Net, |entry| match entry.pending.get_mut(&id) {
                None => Err(format!("no pending request {id}")),
                Some(request) if request.response.is_some() => {
                    Err(format!("request {id} already settled and cannot be cancelled"))
                }
                Some(request) => {
                    request.cancelled = true;
                    request.payload.clear();
                    Ok(())
                }
            })
        }

        /// Close a handle, dropping any request still pending under it.
        pub fn close(&self, handle: u64, caller: &str) -> Result<(), String> {
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            match table.handles.get(&handle) {
                None => Err(format!("no capability handle {handle}")),
                Some(entry) if entry.owner != caller => Err(format!(
                    "capability handle {handle} belongs to '{}', not to '{caller}'",
                    entry.owner
                )),
                Some(_) => {
                    table.handles.remove(&handle);
                    Ok(())
                }
            }
        }
    }

    // ── The registry the JIT resolves handles against ──────────────────────

    use std::sync::{Arc, OnceLock};

    static INSTALLED: OnceLock<Mutex<HashMap<u64, Arc<InstanceCaps>>>> = OnceLock::new();

    fn installed() -> &'static Mutex<HashMap<u64, Arc<InstanceCaps>>> {
        INSTALLED.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Give one instance its capabilities. The host calls this after the identity
    /// is bound; a module never constructs its own.
    pub fn install(instance: u64, caps: InstanceCaps) {
        installed()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(instance, Arc::new(caps));
    }

    /// Drop an instance's capabilities (failed load, revocation, unload).
    pub fn uninstall(instance: u64) {
        installed()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&instance);
    }

    pub fn caps_for(instance: u64) -> Option<Arc<InstanceCaps>> {
        installed()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&instance)
            .cloned()
    }
}

// ── D09 step 3: the `rs_cap_*` boundary ─────────────────────────────────────
//
// Every one of these is a C-ABI function an Austral module reaches through the
// manifest gate. The rules they all keep:
//
//   * the caller is the *current* instance context (D08), never an argument, so
//     a module cannot name another module's identity;
//   * a revoked or replaced instance is refused before any handle is looked up;
//   * lengths are checked against fixed bounds before allocation;
//   * buffers are copied in and out — no borrowed pointer is retained;
//   * a panic is contained and reported as a negative code, because unwinding
//     into JIT'd code is undefined.

/// Negative return codes, so a caller can tell the refusals apart.
pub const CAP_ERR_NO_INSTANCE: i64 = -1;
pub const CAP_ERR_NO_CAPS: i64 = -2;
pub const CAP_ERR_STALE: i64 = -3;
pub const CAP_ERR_BAD_ARG: i64 = -4;
pub const CAP_ERR_REFUSED: i64 = -5;
pub const CAP_ERR_TOO_LONG: i64 = -6;
pub const CAP_ERR_PANIC: i64 = -101;

/// Run `f`, turning a panic into [`CAP_ERR_PANIC`].
fn contained(f: impl FnOnce() -> Result<i64, i64>) -> i64 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => v,
        Ok(Err(code)) => code,
        Err(_) => CAP_ERR_PANIC,
    }
}

/// The current instance and its capabilities, or the code explaining why not.
fn current() -> Result<(std::sync::Arc<caps::InstanceCaps>, String), i64> {
    let context = crate::auth::current_caller().ok_or(CAP_ERR_NO_INSTANCE)?;
    if crate::auth::context_refusal(&context).is_some() {
        return Err(CAP_ERR_STALE);
    }
    let caps = caps::caps_for(context.instance).ok_or(CAP_ERR_NO_CAPS)?;
    Ok((caps, context.module.clone()))
}

unsafe fn bytes<'a>(ptr: *const u8, len: i64) -> Result<&'a [u8], i64> {
    if len < 0 || len as usize > caps::MAX_FRAME {
        return Err(CAP_ERR_TOO_LONG);
    }
    if ptr.is_null() {
        if len == 0 {
            return Ok(&[]);
        }
        return Err(CAP_ERR_BAD_ARG);
    }
    Ok(std::slice::from_raw_parts(ptr, len as usize))
}

unsafe fn text(ptr: *const u8, len: i64) -> Result<String, i64> {
    let raw = bytes(ptr, len)?;
    std::str::from_utf8(raw)
        .map(|s| s.to_string())
        .map_err(|_| CAP_ERR_BAD_ARG)
}

/// Copy `data` into the caller's buffer. Returns the length written, or the
/// required length as a negative number when the buffer is too small, so a
/// caller never receives a truncated frame it might mistake for a full one.
unsafe fn copy_out(data: &[u8], out: *mut u8, cap: i64) -> i64 {
    if cap < 0 {
        return CAP_ERR_BAD_ARG;
    }
    if data.len() > cap as usize {
        return -(data.len() as i64);
    }
    if data.is_empty() {
        return 0;
    }
    if out.is_null() {
        return CAP_ERR_BAD_ARG;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), out, data.len());
    data.len() as i64
}

#[no_mangle]
pub extern "C" fn rs_cap_dir_open() -> i64 {
    contained(|| {
        let (caps, _caller) = current()?;
        caps.open(caps::Kind::Dir)
            .map(|h| h as i64)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_net_open() -> i64 {
    contained(|| {
        let (caps, _caller) = current()?;
        caps.open(caps::Kind::Net)
            .map(|h| h as i64)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_close(handle: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        caps.close(handle as u64, &caller)
            .map(|()| 0)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}

#[no_mangle]
pub unsafe extern "C" fn rs_cap_write(
    handle: i64,
    path_ptr: *const u8,
    path_len: i64,
    data_ptr: *const u8,
    data_len: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let path = text(path_ptr, path_len)?;
        let data = bytes(data_ptr, data_len)?;
        caps.write(handle as u64, &caller, &path, data)
            .map(|n| n as i64)
            .map_err(|e| {
                if e.contains("exceeds") {
                    CAP_ERR_TOO_LONG
                } else {
                    CAP_ERR_REFUSED
                }
            })
    })
}

#[no_mangle]
pub unsafe extern "C" fn rs_cap_read(
    handle: i64,
    path_ptr: *const u8,
    path_len: i64,
    out_ptr: *mut u8,
    out_cap: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let path = text(path_ptr, path_len)?;
        let data = caps
            .read(handle as u64, &caller, &path)
            .map_err(|e| {
                if e.contains("over the") {
                    CAP_ERR_TOO_LONG
                } else {
                    CAP_ERR_REFUSED
                }
            })?;
        Ok(copy_out(&data, out_ptr, out_cap))
    })
}

#[no_mangle]
pub unsafe extern "C" fn rs_cap_send(
    handle: i64,
    peer_ptr: *const u8,
    peer_len: i64,
    data_ptr: *const u8,
    data_len: i64,
) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        let peer = text(peer_ptr, peer_len)?;
        let data = bytes(data_ptr, data_len)?;
        caps.send(handle as u64, &caller, &peer, data)
            .map(|id| id as i64)
            .map_err(|e| {
                if e.contains("exceeds") {
                    CAP_ERR_TOO_LONG
                } else if e.contains("not configured") {
                    CAP_ERR_BAD_ARG
                } else {
                    CAP_ERR_REFUSED
                }
            })
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_query(handle: i64, id: i64, out_ptr: *mut u8, out_cap: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        match caps.query(handle as u64, &caller, id as u64).map_err(|_| CAP_ERR_REFUSED)? {
            // Not settled yet is 0, distinct from an empty response (-0 is still 0),
            // so a caller polls rather than mis-reading a pending request.
            None => Ok(0),
            Some(response) => unsafe { Ok(copy_out(&response, out_ptr, out_cap)) },
        }
    })
}

#[no_mangle]
pub extern "C" fn rs_cap_cancel(handle: i64, id: i64) -> i64 {
    contained(|| {
        let (caps, caller) = current()?;
        caps.cancel(handle as u64, &caller, id as u64)
            .map(|()| 0)
            .map_err(|_| CAP_ERR_REFUSED)
    })
}
