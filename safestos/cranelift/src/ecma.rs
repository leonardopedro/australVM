//! ECMAScript module backend (S1): workerd sidecar supervisor.
//!
//! Runs a module's `.js` entry file inside a [`workerd`](https://github.com/cloudflare/workerd)
//! sidecar process driven by a generated `config.capnp`. The host talks to the sidecar over an
//! HTTP socket; the worker talks back to the host's `uk_*` kernel through a capability loopback:
//! each granted symbol is a workerd `service` binding pointing at a host-side [`KernelLoopback`]
//! HTTP server that re-validates the grant against the module's own grants snapshot (defense in
//! depth) and marshals JSON args onto the
//! `uk_*` C ABI (probe-then-copy buffers).
//!
//! Threat model: web-browser-equivalent. The sidecar is an untrusted V8 isolate with no ambient
//! network/fs; the capability object only ever contains the module's own `[grants]` (F5).

use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::module::ModuleManifest;

/// Locations of the workerd runtime, resolved in order:
/// 1. `UNFER_WORKERD` — explicit path to the `workerd` binary.
/// 2. `workerd` on `$PATH` (global npm install, fnm shim, Nix profile, ...).
/// 3. fnm-managed Node installations (`~/.local/share/fnm/node-versions/*`).
///
/// The capnp import dir is `UNFER_WORKERD_IMPORT` when set, else derived from the resolved
/// binary via the npm package layout (`<pkg>/workerd.capnp`).
#[derive(Debug, Clone)]
pub struct WorkerdPaths {
    pub bin: PathBuf,
    pub import_dir: PathBuf,
}

impl WorkerdPaths {
    /// Resolve the `workerd` binary and its capnp import dir, in order of preference:
    /// 1. `$UNFER_WORKERD` (explicit; error if set but missing).
    /// 2. `workerd` found on `$PATH` (e.g. a global npm install, fnm shim, or Nix profile).
    /// 3. fnm-managed Node installations (`~/.local/share/fnm/node-versions/*/...`).
    ///
    /// The capnp import dir is `$UNFER_WORKERD_IMPORT` when set, else derived from the
    /// resolved binary path via the npm layout (`<pkg>/workerd.capnp` where the binary lives
    /// at `<pkg>/bin/workerd` or `<pkg>/installation/bin/workerd`).
    pub fn from_env() -> Result<Self, String> {
        let bin = resolve_workerd_bin()?;
        let import_dir = match std::env::var("UNFER_WORKERD_IMPORT") {
            Ok(d) => PathBuf::from(d),
            Err(_) => {
                // npm layout: <pkg>/bin/workerd, <pkg>/workerd.capnp (also the fnm variant
                // <pkg>/installation/bin/workerd). Canonicalize to deref the fnm shim symlink.
                let real = std::fs::canonicalize(&bin).unwrap_or_else(|_| bin.clone());
                let pkg = real
                    .parent()
                    .and_then(|d| d.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                if pkg.join("workerd.capnp").exists() {
                    pkg
                } else {
                    return Err(
                        "cannot locate workerd.capnp; set UNFER_WORKERD_IMPORT to its directory"
                            .to_string(),
                    );
                }
            }
        };
        if !import_dir.join("workerd.capnp").exists() {
            return Err(format!(
                "workerd.capnp not found in {}",
                import_dir.display()
            ));
        }
        Ok(Self { bin, import_dir })
    }

    /// Path to the `workerd` binary.
    pub fn binary(&self) -> &Path {
        &self.bin
    }
}

/// Resolve the `workerd` binary: `$UNFER_WORKERD` → `$PATH` → fnm node installations.
fn resolve_workerd_bin() -> Result<PathBuf, String> {
    if let Ok(b) = std::env::var("UNFER_WORKERD") {
        let p = PathBuf::from(b);
        if p.exists() {
            return Ok(p);
        }
        return Err(format!("workerd binary not found: {}", p.display()));
    }

    if let Some(p) = find_on_path("workerd") {
        return Ok(p);
    }

    if let Some(p) = find_in_fnm() {
        return Ok(p);
    }

    Err(
        "ecmascript module requires the workerd runtime; set UNFER_WORKERD to its path, \
         install it via `npm install -g workerd`, or put it on $PATH"
            .to_string(),
    )
}

/// Search `$PATH` for an executable named `name`.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            if let Ok(md) = std::fs::metadata(&candidate) {
                let is_exe = {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        md.permissions().mode() & 0o111 != 0
                    }
                    #[cfg(not(unix))]
                    {
                        true
                    }
                };
                if is_exe {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Search fnm-managed Node installations for a global npm `workerd` package:
/// `~/.local/share/fnm/node-versions/<v>/installation/lib/node_modules/workerd/bin/workerd`.
fn find_in_fnm() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let root = PathBuf::from(home)
        .join(".local/share/fnm/node-versions");
    let entries = std::fs::read_dir(&root).ok()?;
    let mut versions: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    // Prefer the most recent installation when multiple exist.
    versions.sort();
    for version_dir in versions.into_iter().rev() {
        let candidate = version_dir
            .join("installation/lib/node_modules/workerd/bin/workerd");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// A running workerd sidecar for one ECMAScript module (S1: one sidecar per module).
pub struct EcmaSidecar {
    child: Child,
    main_sock: PathBuf,
    loopback: KernelLoopback,
    staging: PathBuf,
    module_name: String,
}

impl EcmaSidecar {
    /// Materialize a `config.capnp` + harness in `staging_dir`, spawn `workerd serve`, and
    /// wait until the main unix socket accepts connections.
    ///
    /// `module_dir` supplies the entry file and the fs-grant base for the OS sandbox; the
    /// staging directory is passed separately so that per-instance sidecars (S5/F3) can each
    /// own a private `config.capnp` + sockets without colliding with siblings of the same
    /// module.
    pub fn spawn(
        module_dir: &Path,
        staging_dir: &Path,
        manifest: &ModuleManifest,
        paths: &WorkerdPaths,
    ) -> Result<Self, String> {
        let module_name = manifest.name.clone();

        // Staging dir holds config.capnp, harness.mjs and a copy of the entry JS so workerd's
        // `embed "..."` directives resolve relative to the config file.
        let staging = staging_dir.to_path_buf();
        std::fs::create_dir_all(&staging).map_err(|e| format!("staging dir: {e}"))?;

        let entry_file = module_dir.join(&manifest.entry);
        if !entry_file.exists() {
            return Err(format!(
                "ecmascript entry not found: {}",
                entry_file.display()
            ));
        }
        std::fs::copy(&entry_file, staging.join("module.js"))
            .map_err(|e| format!("copy entry: {e}"))?;

        // Host-side capability loopback first (its socket path is baked into config.capnp).
        let loopback_sock = staging.join("kernel-loopback.sock");
        let loopback = KernelLoopback::start(
            &module_name,
            manifest.grants.clone(),
            manifest.effects.clone(),
            &loopback_sock,
        )?;

        let harness = harness_source();
        std::fs::write(staging.join("harness.mjs"), harness)
            .map_err(|e| format!("write harness: {e}"))?;

        let main_sock = staging.join("main.sock");
        let config = config_source(manifest, &loopback_sock, &main_sock);
        std::fs::write(staging.join("config.capnp"), config)
            .map_err(|e| format!("write config: {e}"))?;

        let args: Vec<std::ffi::OsString> = vec![
            "serve".into(),
            staging.join("config.capnp").into_os_string(),
            "-I".into(),
            paths.import_dir.clone().into_os_string(),
        ];

        let spawn_cmd = |base: &mut Command| -> Result<Child, String> {
            base.args(args.iter()).stdout(Stdio::piped()).stderr(Stdio::piped());
            base.spawn().map_err(|e| e.to_string())
        };

        // S3: wrap the sidecar in the OS sandbox (userns + netns + no_new_privs + seccomp +
        // Landlock) when the feature is on and the kernel supports it. The unix sockets the
        // sidecar materializes in `staging` are the only reachable endpoints (empty netns).
        #[cfg(feature = "sandbox")]
        {
            if crate::sandbox::supported() {
                let mut writable = vec![staging.clone(), module_dir.to_path_buf()];
                for g in &manifest.fs_grants {
                    writable.push(module_dir.join(g));
                }
                // The child needs read/exec on its own binary, its dynamic deps (Nix store /
                // system libs), the capnp import dir, and staging (config + harness + module.js).
                // Include the standard system dirs + loader because workerd is dynamically
                // linked against glibc and the dynamic linker (e.g. /lib64 on NixOS is a symlink
                // into /nix/store) — Landlock must be able to traverse them at exec time.
                let mut readable = vec![
                    paths.import_dir.clone(),
                    paths.bin.parent().unwrap_or(Path::new("/")).to_path_buf(),
                    PathBuf::from("/lib"),
                    PathBuf::from("/lib64"),
                    PathBuf::from("/usr/lib"),
                    PathBuf::from("/etc"),
                    PathBuf::from("/nix/store"),
                    PathBuf::from("/run/current-system"),
                    PathBuf::from("/proc"),
                    PathBuf::from("/dev"),
                ];
                if let Ok(real_bin) = std::fs::canonicalize(&paths.bin) {
                    if let Some(parent) = real_bin.parent() {
                        readable.push(parent.to_path_buf());
                    }
                }
                let profile = crate::sandbox::SandboxProfile {
                    writable_dirs: writable,
                    readable_dirs: readable,
                    memory_max_bytes: None,
                };
                let mut sandbox_cmd = crate::sandbox::sandboxed_command(&paths.bin, &profile);
                let child = spawn_cmd(&mut sandbox_cmd)
                    .map_err(|e| format!("spawn sandboxed workerd: {e} (bin={})", paths.bin.display()))?;

                let mut sidecar = Self {
                    child,
                    main_sock,
                    loopback,
                    staging,
                    module_name,
                };
                sidecar.wait_ready()?;
                return Ok(sidecar);
            }
        }

        let mut cmd = Command::new(&paths.bin);
        let child = spawn_cmd(&mut cmd)
            .map_err(|e| format!("spawn workerd: {e} (bin={})", paths.bin.display()))?;

        let mut sidecar = Self {
            child,
            main_sock,
            loopback,
            staging,
            module_name,
        };
        sidecar.wait_ready()?;
        Ok(sidecar)
    }

    fn wait_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| format!("wait: {e}"))? {
                let stderr = read_child_stderr(&mut self.child);
                return Err(format!(
                    "workerd exited early with {status}{}",
                    stderr.map(|s| format!("\nstderr:\n{s}")).unwrap_or_default()
                ));
            }
            if UnixStream::connect(&self.main_sock).is_ok() {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err("timed out waiting for workerd socket".to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// RPC the sidecar's entrypoint: `POST /unfer/call` with `{"entrypoint":..,"args":..}`.
    /// Returns the JSON body of the response.
    pub fn call(&self, entrypoint: &str, args_json: &str) -> Result<String, String> {
        let body = format!(
            r#"{{"entrypoint":{0:?},"args":{1}}}"#,
            entrypoint, args_json
        );
        http_post(&self.main_sock, "/unfer/call", &body)
    }

    pub fn loopback_sock(&self) -> &Path {
        &self.loopback.path
    }

    pub fn module_name(&self) -> &str {
        &self.module_name
    }

    /// PID of the sandboxed `workerd` sidecar. Used by S3 escape-attempt tests to probe the
    /// child's namespace/capability confinement (e.g. `uid_map`, `no_new_privs`).
    pub fn child_pid(&self) -> u32 {
        self.child.id()
    }

    /// Path of the staging dir holding `config.capnp`, `harness.mjs`, `module.js` and the unix
    /// sockets. Exposed for tests/consumers to inspect the materialized sidecar contract.
    pub fn staging_dir(&self) -> &Path {
        &self.staging
    }
}

impl Drop for EcmaSidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.staging);
    }
}

/// Minimal host-side kernel loopback: accepts `POST /kernel/<symbol>` from the workerd worker,
/// re-validates the grant against the module's own grants snapshot, and dispatches onto the
/// `uk_*` C ABI. The grants are captured per-sidecar at spawn time, NOT via the global auth
/// engine: concurrent module loads replace the global engine, so relying on it here would be a
/// race (defense in depth must be per-module).
struct KernelLoopback {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    _handle: std::thread::JoinHandle<()>,
}

impl KernelLoopback {
    fn start(
        module_name: &str,
        grants: Vec<String>,
        effects: Vec<String>,
        sock_path: &Path,
    ) -> Result<Self, String> {
        let _ = std::fs::remove_file(sock_path);
        let listener = UnixListener::bind(sock_path).map_err(|e| format!("loopback bind: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("loopback nonblocking: {e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let module_name = module_name.to_string();
        let grants: Arc<HashSet<String>> = Arc::new(grants.into_iter().collect());
        let effects: Arc<HashSet<String>> = Arc::new(effects.into_iter().collect());
        let handle = std::thread::spawn(move || {
            let _ = &listener;
            loop {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        let name = module_name.clone();
                        let grants = Arc::clone(&grants);
                        let effects = Arc::clone(&effects);
                        std::thread::spawn(move || {
                            handle_loopback_conn(&name, &grants, &effects, stream);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            path: sock_path.to_path_buf(),
            stop,
            _handle: handle,
        })
    }
}

impl Drop for KernelLoopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }
}

fn handle_loopback_conn(
    module_name: &str,
    grants: &HashSet<String>,
    effects: &HashSet<String>,
    mut stream: UnixStream,
) {
    let mut request = Vec::new();
    let mut buf = [0u8; 4096];
    let mut total = 0usize;
    let header_end = loop {
        match stream.read(&mut buf) {
            Ok(0) => break None,
            Ok(n) => {
                request.extend_from_slice(&buf[..n]);
                total += n;
                if let Some(pos) = find_header_end(&request) {
                    break Some(pos);
                }
                if total > 1024 * 1024 {
                    break None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return,
        }
    };
    let Some(header_end) = header_end else { return };
    let headers = &request[..header_end];
    let body_start = header_end + 4; // skip \r\n\r\n

    // Content-Length.
    let mut content_length = 0usize;
    for line in headers.split(|&b| b == b'\n') {
        let line = String::from_utf8_lossy(line);
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    // Read the full body.
    let mut body = request[body_start.min(request.len())..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    let body = String::from_utf8_lossy(&body[..body.len().min(content_length)]).to_string();

    // Path: /kernel/<symbol>
    let request_line = headers
        .split(|&b| b == b'\n')
        .next()
        .map(|l| String::from_utf8_lossy(l).to_string())
        .unwrap_or_default();
    let symbol = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .trim_start_matches('/')
        .strip_prefix("kernel/")
        .map(String::from);

    let response = match symbol {
        Some(sym) => dispatch_loopback(module_name, grants, effects, &sym, &body),
        None => json_response("error", &serde_json::json!({"code": 4000, "message": "bad path"})),
    };
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn json_response(key: &str, value: &serde_json::Value) -> String {
    let obj = serde_json::json!({ key: value });
    let body = obj.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

/// Authorize the symbol (host-side, defense in depth) and marshal the JSON args onto the `uk_*`
/// C ABI. Returns an HTTP response string.
///
/// Two grant namespaces gate the loopback:
/// * `[grants] kernel = [...]` — every `uk_*` symbol except `uk_action_submit`.
/// * `[grants] effects = [...]` — the *effect name* a module may submit via `uk_action_submit`.
///   This is the S4 "effects" namespace: holding `effects = ["send_notification"]` permits
///   submitting a `send_notification` action without any kernel grant for the symbol.
fn dispatch_loopback(
    module_name: &str,
    grants: &HashSet<String>,
    effects: &HashSet<String>,
    symbol: &str,
    body: &str,
) -> String {
    // 1. JSON args (a JSON array) → effect-name gate for uk_action_submit, kernel grant otherwise.
    let args: Vec<serde_json::Value> = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return json_response(
                "error",
                &serde_json::json!({"code": 4000, "message": "args must be a JSON array"}),
            )
        }
    };

    if symbol == "uk_action_submit" {
        // S4: the effects namespace, not the kernel grants, gates submission. The effect
        // name is `req.effect` of the single request arg.
        let effect = args
            .first()
            .and_then(|a| a.get("effect"))
            .and_then(|e| e.as_str())
            .unwrap_or("");
        if !effects.contains(effect) {
            return json_response(
                "error",
                &serde_json::json!({"code": 4001, "name": "CallDenied", "message": format!(
                    "UK-4001: Authorization denied — '{module_name}' is not granted effect '{effect}'"
                )}),
            );
        }
    } else if !grants.contains(symbol) {
        // Default-deny: the module's own granted capability set is authoritative host-side. This
        // is a per-sidecar snapshot, independent of the global auth engine (which concurrent
        // module loads may overwrite).
        return json_response(
            "error",
            &serde_json::json!({"code": 4001, "name": "CallDenied", "message": format!(
                "UK-4001: Authorization denied — '{module_name}' is not granted '{symbol}'"
            )}),
        );
    }

    let out = kernel_dispatch(module_name, symbol, &args);
    match out {
        Ok(value) => json_response("result", &value),
        Err((code, message)) => json_response(
            "error",
            &serde_json::json!({"code": code, "name": "KernelError", "message": message}),
        ),
    }
}

type DispatchResult = Result<serde_json::Value, (u32, String)>;

fn arg_i64(args: &[serde_json::Value], i: usize) -> Result<i64, (u32, String)> {
    args.get(i)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| (1001, format!("arg {i}: expected integer")))
}

fn arg_str(args: &[serde_json::Value], i: usize) -> Result<String, (u32, String)> {
    let v = args.get(i).ok_or_else(|| (1001, format!("arg {i}: missing")))?;
    match v {
        serde_json::Value::String(s) => Ok(s.clone()),
        other => serde_json::to_string(other).map_err(|e| (1001, e.to_string())),
    }
}

/// Call a `uk_*` function that returns `i64`; on negative return, fetch `uk_last_error`.
fn ffi_result(ret: i64) -> DispatchResult {
    if ret < 0 {
        let code = (-ret) as u32;
        let message = read_last_error().unwrap_or_else(|| format!("UK-{code}"));
        Err((code, message))
    } else {
        Ok(serde_json::json!(ret))
    }
}

/// Probe-then-copy buffer-out protocol (uk_get_result, uk_last_error, uk_snapshot, uk_poll).
fn buf_out(mut f: impl FnMut(*mut u8, i64) -> i64) -> DispatchResult {
    let needed = f(std::ptr::null_mut(), 0);
    if needed < 0 {
        return Err(((-needed) as u32, read_last_error().unwrap_or_default()));
    }
    if needed == 0 {
        return Ok(serde_json::Value::String(String::new()));
    }
    let mut buf = vec![0u8; needed as usize];
    let n = f(buf.as_mut_ptr(), needed);
    if n < 0 {
        return Err(((-n) as u32, read_last_error().unwrap_or_default()));
    }
    let s = String::from_utf8(buf).map_err(|e| (1001, e.to_string()))?;
    Ok(serde_json::Value::String(s))
}

/// Binary variant of [`buf_out`]: reads raw bytes (which are not UTF-8 — e.g. a gzip `.cell`
/// archive) and hex-encodes them so they survive the JSON transport.
fn buf_out_raw(mut f: impl FnMut(*mut u8, i64) -> i64) -> DispatchResult {
    let needed = f(std::ptr::null_mut(), 0);
    if needed < 0 {
        return Err(((-needed) as u32, read_last_error().unwrap_or_default()));
    }
    if needed == 0 {
        return Ok(serde_json::Value::String(String::new()));
    }
    let mut buf = vec![0u8; needed as usize];
    let n = f(buf.as_mut_ptr(), needed);
    if n < 0 {
        return Err(((-n) as u32, read_last_error().unwrap_or_default()));
    }
    Ok(serde_json::Value::String(hex::encode(&buf)))
}

fn read_last_error() -> Option<String> {
    let needed = unfer_ffi::uk_last_error(std::ptr::null_mut(), 0);
    if needed <= 0 {
        return None;
    }
    let mut buf = vec![0u8; needed as usize];
    unfer_ffi::uk_last_error(buf.as_mut_ptr(), needed);
    String::from_utf8(buf).ok()
}

/// Dispatch one `uk_*` symbol. Each arm encodes the C ABI signature. This is the single place
/// where JS objects are translated onto the probe-then-copy buffer protocol.
///
/// `module_name` is the submitting module's identity: `uk_action_submit` injects it as the
/// record's `principal` (an audit tag — a worker cannot claim another module's identity).
fn kernel_dispatch(
    module_name: &str,
    symbol: &str,
    args: &[serde_json::Value],
) -> DispatchResult {
    match symbol {
        "uk_version" => Ok(serde_json::json!(unfer_ffi::uk_version())),
        "uk_init" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_init(p, l))
        }
        "uk_model_create" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_model_create(p, l))
        }
        "uk_model_free" => {
            let model = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_model_free(model))
        }
        "uk_set_prior" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_set_prior(model, p, l))
        }
        "uk_set_hamiltonian" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_set_hamiltonian(model, p, l))
        }
        "uk_evolve" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_evolve(model, p, l))
        }
        "uk_condition" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_condition(model, p, l))
        }
        "uk_event_probability" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_event_probability(model, p, l))
        }
        "uk_observe" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_observe(model, p, l))
        }
        "uk_get_result" => {
            let model = arg_i64(args, 0)?;
            let out = buf_out(|b, c| unfer_ffi::uk_get_result(model, b, c))?;
            // Parse the result JSON if possible, else return the raw string.
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_last_error" => {
            let out = buf_out(|b, c| unfer_ffi::uk_last_error(b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_snapshot" => {
            let model = arg_i64(args, 0)?;
            let out = buf_out(|b, c| unfer_ffi::uk_snapshot(model, b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_restore" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_restore(p, l))
        }
        // S5 (F4): .cell blueprint archives. The archive is binary (gzip), so the loopback
        // transport hex-encodes it: export returns hex, instantiate accepts hex.
        "uk_blueprint_export" => {
            let model = arg_i64(args, 0)?;
            let out = buf_out_raw(|b, c| unfer_ffi::uk_blueprint_export(model, b, c))?;
            Ok(serde_json::json!({ "cell_hex": out.as_str().unwrap_or("") }))
        }
        "uk_blueprint_instantiate" => {
            let hexed = arg_str(args, 0)?;
            let bytes = hex::decode(hexed.trim())
                .map_err(|e| (1001, format!("blueprint hex decode: {e}")))?;
            ffi_result(unfer_ffi::uk_blueprint_instantiate(bytes.as_ptr(), bytes.len() as i64))
        }
        "uk_subscribe" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_subscribe(model, p, l))
        }
        "uk_poll" => {
            let sub = arg_i64(args, 0)?;
            let out = buf_out(|b, c| unfer_ffi::uk_poll(sub, b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_bayesian_update" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_bayesian_update(model, p, l))
        }
        "uk_belief_propagation" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_belief_propagation(model, p, l))
        }
        "uk_buf_free" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_buf_free(handle))
        }
        "uk_ode_analyze" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_ode_analyze(p, l))
        }
        "uk_ode_measure_original" => {
            let model = arg_i64(args, 0)?;
            let json = arg_str(args, 1)?;
            let (p, l) = ptr_len(&json);
            let ret = unfer_ffi::uk_ode_measure_original(model, p, l);
            Ok(serde_json::json!(ret))
        }
        // ── S4: deferred approval + local simulation ─────────────────────────
        // The effects grant gate for uk_action_submit ran in `dispatch_loopback`. Here we
        // inject the module identity as the record principal (audit tag, F6) and marshal
        // onto the FFI.
        "uk_action_submit" => {
            let mut req = args
                .get(0)
                .cloned()
                .ok_or_else(|| (1001, "uk_action_submit: missing request arg".to_string()))?;
            if let Some(obj) = req.as_object_mut() {
                obj.insert("principal".to_string(), serde_json::json!(module_name));
            }
            let json = serde_json::to_string(&req).map_err(|e| (1001, e.to_string()))?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_action_submit(p, l))
        }
        "uk_action_apply" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_action_apply(handle))
        }
        "uk_action_reject" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_action_reject(handle))
        }
        "uk_action_revert" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_action_revert(handle))
        }
        "uk_action_get" => {
            let handle = arg_i64(args, 0)?;
            let out = buf_out(|b, c| unfer_ffi::uk_action_get(handle, b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_action_list" => {
            let out = buf_out(|b, c| unfer_ffi::uk_action_list(b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        other => Err((
            4004,
            format!("kernel symbol '{other}' has no loopback marshaling"),
        )),
    }
}

fn ptr_len(s: &str) -> (*const u8, i64) {
    (s.as_ptr(), s.len() as i64)
}

fn read_child_stderr(child: &mut Child) -> Option<String> {
    let mut s = String::new();
    child.stderr.as_mut()?.read_to_string(&mut s).ok()?;
    Some(s)
}

/// `POST /path` to a unix socket, return the response body.
fn http_post(sock: &Path, path: &str, body: &str) -> Result<String, String> {
    let mut stream = UnixStream::connect(sock).map_err(|e| format!("connect {}: {e}", sock.display()))?;
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: unix\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("send: {e}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("recv: {e}"))?;
    let response = String::from_utf8_lossy(&response).to_string();
    let (_, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("bad HTTP response: {}", &response[..response.len().min(200)]))?;
    Ok(body.to_string())
}

// ── config.capnp + harness generation ─────────────────────────────────

fn config_source(manifest: &ModuleManifest, loopback_sock: &Path, main_sock: &Path) -> String {
    let mut bindings = String::new();
    for sym in &manifest.grants {
        if is_kernel_symbol(sym) {
            bindings.push_str(&format!("         (name = \"{sym}\", service = \"kernel-loopback\"),\n"));
        }
    }
    format!(
        r#"using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "main",
     worker = (
       compatibilityDate = "2025-07-01",
       modules = [
         (name = "harness.mjs", esModule = embed "harness.mjs"),
         (name = "module.js", esModule = embed "module.js"),
       ],
       bindings = [
{bindings}       ],
     )),
    (name = "kernel-loopback",
     external = (address = "unix:{loopback}", http = ())),
  ],
  sockets = [
    (name = "main", address = "unix:{main}", http = (), service = "main"),
  ],
);
"#,
        loopback = loopback_sock.display(),
        main = main_sock.display(),
    )
}

/// The generated worker entry: dispatches `/unfer/call` to a named export of the module's JS and
/// exposes a `kernel` capability object built strictly from the granted service bindings. Any
/// `uk_*`/`uz_*` symbol NOT in the environment is stubbed to throw a UK-4001 `CallDenied` — the
/// module can only ever see the symbols in its own `[grants]` (F5).
fn harness_source() -> &'static str {
    r#"// Generated by austral_cranelift_bridge (ecma.rs). Do not edit.
import * as module from "./module.js";

function makeKernel(env) {
  return new Proxy({}, {
    get(_target, name) {
      if (typeof name !== "string") return undefined;
      const isKernel = name.startsWith("uk_") || name.startsWith("uz_");
      if (!isKernel) return undefined;
      if (name in env) {
        return async (...args) => {
          const res = await env[name].fetch("http://kernel-loopback/kernel/" + name, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify(args),
          });
          const data = await res.json();
          if (data.error) {
            const e = new Error(data.error.message);
            e.ukCode = data.error.code;
            throw e;
          }
          return data.result;
        };
      }
      return async () => {
        const e = new Error(`UK-4001: CALL_DENIED — '${name}' is not granted to this module`);
        e.ukCode = 4001;
        throw e;
      };
    }
  });
}

function body(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export default {
  async fetch(request, env) {
    const kernel = makeKernel(env);
    const url = new URL(request.url);
    if (url.pathname !== "/unfer/call") {
      return body({ error: { code: 4040, message: "not found" } }, 404);
    }
    let payload;
    try {
      payload = await request.json();
    } catch (e) {
      return body({ error: { code: 4000, message: "bad request body" } });
    }
    const { entrypoint, args } = payload;
    const fn = typeof module[entrypoint] === "function"
      ? module[entrypoint]
      : (module.default && typeof module.default[entrypoint] === "function"
          ? module.default[entrypoint]
          : null);
    if (!fn) {
      return body({ error: { code: 4040, message: `entrypoint '${entrypoint}' not found` } });
    }
    try {
      const result = await fn(kernel, args);
      return body({ result: result ?? null });
    } catch (e) {
      return body({ error: { code: e.ukCode || 5000, message: String(e && e.message || e) } });
    }
  },
};
"#
}

fn is_kernel_symbol(sym: &str) -> bool {
    crate::registered_unfer_symbols().contains(&sym)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the `WorkerdPaths::from_env` tests: they mutate the global
    /// `UNFER_WORKERD` / `UNFER_WORKERD_IMPORT` env vars, which must never run
    /// concurrently (parallel tests would observe each other's state).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn harness_denies_ungranted_symbol() {
        // The generated harness must only expose granted uk_* symbols; un-granted ones throw
        // UK-4001. We assert the source contains the deny stub and the grant-wrapping branch.
        let src = harness_source();
        assert!(src.contains("UK-4001: CALL_DENIED"));
        assert!(src.contains("env[name].fetch"));
        assert!(src.contains("data.error"));
    }

    #[test]
    fn config_embeds_only_granted_bindings() {
        let manifest = ModuleManifest {
            name: "t".into(),
            version: "0.1.0".into(),
            archetypes: vec![],
            archetype: "ecmascript".into(),
            entry: "src/main.js".into(),
            grants: vec![
                "uk_version".into(),
                "uk_model_create".into(),
                "uk_nonexistent".into(),
            ],
            effects: vec![],
            fs_grants: vec![],
            net_grants: vec![],
            max_ms: None,
        };
        let cfg = config_source(&manifest, Path::new("/tmp/loop.sock"), Path::new("/tmp/main.sock"));
        assert!(cfg.contains("(name = \"uk_version\", service = \"kernel-loopback\")"));
        assert!(cfg.contains("(name = \"uk_model_create\", service = \"kernel-loopback\")"));
        assert!(!cfg.contains("uk_nonexistent"), "un-granted symbols must be absent");
        assert!(cfg.contains("unix:/tmp/loop.sock"));
        assert!(cfg.contains("unix:/tmp/main.sock"));
    }

    #[test]
    fn from_env_resolves_unfer_workerd_env() {
        // $UNFER_WORKERD + $UNFER_WORKERD_IMPORT set: used verbatim.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("unfer_wd_env_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/workerd"), "#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("workerd.capnp"), "@0xdeadbeef;\n").unwrap();

        unsafe {
            std::env::set_var("UNFER_WORKERD", dir.join("bin/workerd"));
            std::env::set_var("UNFER_WORKERD_IMPORT", &dir);
        }
        let paths = WorkerdPaths::from_env().expect("env vars must resolve");
        assert_eq!(paths.bin, dir.join("bin/workerd"));
        assert_eq!(paths.import_dir, dir);

        unsafe {
            std::env::remove_var("UNFER_WORKERD");
            std::env::remove_var("UNFER_WORKERD_IMPORT");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_env_derives_import_dir_from_npm_layout() {
        // $UNFER_WORKERD points at <pkg>/bin/workerd, no $UNFER_WORKERD_IMPORT:
        // import dir must be derived as <pkg>.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("unfer_wd_npm_layout_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/workerd"), "#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("workerd.capnp"), "@0xdeadbeef;\n").unwrap();

        unsafe {
            std::env::set_var("UNFER_WORKERD", dir.join("bin/workerd"));
            std::env::remove_var("UNFER_WORKERD_IMPORT");
        }
        let paths = WorkerdPaths::from_env().expect("npm layout must resolve");
        assert_eq!(paths.bin, dir.join("bin/workerd"));
        assert_eq!(paths.import_dir, dir);

        unsafe {
            std::env::remove_var("UNFER_WORKERD");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_env_rejects_missing_binary() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("UNFER_WORKERD", "/nonexistent/workerd");
            std::env::set_var("UNFER_WORKERD_IMPORT", "/nonexistent");
        }
        let err = WorkerdPaths::from_env().expect_err("missing binary must error");
        assert!(err.contains("not found"));

        unsafe {
            std::env::remove_var("UNFER_WORKERD");
            std::env::remove_var("UNFER_WORKERD_IMPORT");
        }
    }

    // ── S4: effects-grant gate on the kernel loopback ──────────────────────
    //
    // `dispatch_loopback` gates `uk_action_submit` by the module's `effects` namespace
    // (not its kernel grants), and every other symbol by the kernel grants. These tests
    // hit the loopback function directly (no workerd needed).

    #[test]
    fn loopback_effect_grant_allows_submission() {
        use std::collections::HashSet;
        let grants = HashSet::from(["uk_action_list".to_string(), "uk_action_apply".to_string()]);
        let effects = HashSet::from(["send_notification".to_string()]);
        let body = r#"[{"effect":"send_notification","params":{"to":"alice"}}]"#;
        let resp =
            dispatch_loopback("client_module", &grants, &effects, "uk_action_submit", body);
        // The gate lets the granted effect through: the loopback returns a `result`
        // (the action handle), not an error.
        assert!(
            resp.contains("\"result\"") && !resp.contains("\"error\""),
            "granted effect must dispatch, got {resp}"
        );
        // The module identity is injected as the record principal (audit tag).
        let json = resp.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
        let handle = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v["result"].as_i64());
        assert!(handle.is_some(), "expected a result handle, got {resp}");
        // Cross-check via uk_action_get: principal must be the module name.
        let out = read_last_error(); // ensure clean
        let _ = out;
        let needed = unfer_ffi::uk_action_get(handle.unwrap(), std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; needed as usize];
        unfer_ffi::uk_action_get(handle.unwrap(), buf.as_mut_ptr(), needed);
        let record: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(record["principal"], "client_module");
        assert_eq!(record["effect"], "send_notification");
        assert_eq!(record["state"], "pending");
    }

    #[test]
    fn loopback_effect_grant_denies_unlisted_effect() {
        use std::collections::HashSet;
        let grants = HashSet::from(["uk_action_submit".to_string()]);
        let effects = HashSet::from(["send_notification".to_string()]);
        // The module holds `send_notification`, NOT `delete_all`.
        let body = r#"[{"effect":"delete_all","params":{}}]"#;
        let resp =
            dispatch_loopback("client_module", &grants, &effects, "uk_action_submit", body);
        assert!(resp.contains("\"error\""), "unlisted effect must be denied, got {resp}");
        assert!(resp.contains("4001"), "expected UK-4001, got {resp}");
        assert!(resp.contains("delete_all"), "denial must name the effect, got {resp}");
    }

    #[test]
    fn loopback_effect_grant_denies_without_effects_namespace() {
        use std::collections::HashSet;
        let grants = HashSet::from(["uk_action_submit".to_string()]);
        // No `effects` grant at all: even a listed effect is denied.
        let effects = HashSet::new();
        let body = r#"[{"effect":"send_notification","params":{}}]"#;
        let resp =
            dispatch_loopback("client_module", &grants, &effects, "uk_action_submit", body);
        assert!(resp.contains("\"error\""), "missing effects grant must deny, got {resp}");
        assert!(resp.contains("4001"), "expected UK-4001, got {resp}");
    }

    #[test]
    fn loopback_kernel_gate_still_applies_to_other_action_symbols() {
        use std::collections::HashSet;
        // `uk_action_apply` is NOT in the kernel grants → UK-4001 even though the module
        // may submit effects.
        let grants = HashSet::from(["uk_action_submit".to_string()]);
        let effects = HashSet::from(["send_notification".to_string()]);
        let resp = dispatch_loopback(
            "client_module",
            &grants,
            &effects,
            "uk_action_apply",
            r#"[1]"#,
        );
        assert!(resp.contains("\"error\""), "ungranted kernel symbol must deny, got {resp}");
        assert!(resp.contains("4001"), "expected UK-4001, got {resp}");
    }
}
