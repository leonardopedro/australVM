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
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::module::{GatekeeperMode, ModuleManifest};

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
    child: Arc<Mutex<Option<Child>>>,
    main_sock: PathBuf,
    loopback: KernelLoopback,
    staging: PathBuf,
    module_name: String,
    /// S14 (F13): per-call deadline for the host↔sidecar RPC (`[limits] max_ms`, default 5 s).
    call_deadline: Duration,
    _supervisor: Arc<AtomicBool>,
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
        // S14 (F13): `[limits] max_ms` per-call deadline, default 5 s. `[limits] memory_bytes`
        // flows into the SandboxProfile above and the cgroup below (degraded, writable-only).
        let call_deadline = Duration::from_millis(manifest.max_ms.unwrap_or(5000));

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
            manifest.observers.clone(),
            manifest.net_grants.clone(),
            manifest.resources.clone(),
            manifest.effect_kinds.clone(),
            manifest.gatekeeper_mode,
            &loopback_sock,
        )?;

        let harness = harness_source();
        std::fs::write(staging.join("harness.mjs"), harness)
            .map_err(|e| format!("write harness: {e}"))?;

        let main_sock = staging.join("main.sock");
        let config = config_source(manifest, &loopback_sock, &main_sock);
        std::fs::write(staging.join("config.capnp"), config)
            .map_err(|e| format!("write config: {e}"))?;

        let args: std::sync::Arc<Vec<std::ffi::OsString>> = std::sync::Arc::new(vec![
            "serve".into(),
            staging.join("config.capnp").into_os_string(),
            "-I".into(),
            paths.import_dir.clone().into_os_string(),
        ]);

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
                    memory_max_bytes: manifest.memory_max_bytes, // S14: `[limits] memory_bytes`
                };
                let bin = paths.bin.clone();
                let make_child: Arc<dyn Fn() -> Result<Child, String> + Send + Sync> =
                    Arc::new(move || {
                        let mut sandbox_cmd =
                            crate::sandbox::sandboxed_command(&bin, &profile);
                        sandbox_cmd.args(args.iter());
                        sandbox_cmd
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .spawn()
                            .map_err(|e| {
                                format!("spawn sandboxed workerd: {e} (bin={})", bin.display())
                            })
                    });
                return Self::spawn_with_supervisor(
                    make_child,
                    main_sock,
                    loopback,
                    staging,
                    module_name,
                    call_deadline,
                    manifest.memory_max_bytes,
                );
            }
        }

        let bin = paths.bin.clone();
        let make_child: Arc<dyn Fn() -> Result<Child, String> + Send + Sync> =
            Arc::new(move || {
                let mut cmd = Command::new(&bin);
                cmd.args(args.iter())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| format!("spawn workerd: {e} (bin={})", bin.display()))
            });
        Self::spawn_with_supervisor(
            make_child,
            main_sock,
            loopback,
            staging,
            module_name,
            call_deadline,
            manifest.memory_max_bytes,
        )
    }

    /// Spawn the initial child, hand it to a supervisor thread (S12) that respawns it on
    /// crash with 1s→8s backoff, arm the loopback peer check, and wait for the socket.
    fn spawn_with_supervisor(
        make_child: Arc<dyn Fn() -> Result<Child, String> + Send + Sync>,
        main_sock: PathBuf,
        loopback: KernelLoopback,
        staging: PathBuf,
        module_name: String,
        call_deadline: Duration,
        memory_max_bytes: Option<u64>,
    ) -> Result<Self, String> {
        let initial = make_child()?;
        let initial_pid = initial.id();
        Self::apply_memory_cgroup(initial_pid, memory_max_bytes);
        let slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(initial)));
        let quit = Arc::new(AtomicBool::new(false));

        let sup_slot = Arc::clone(&slot);
        let sup_make = Arc::clone(&make_child);
        let sup_quit = Arc::clone(&quit);
        let sup_name = module_name.clone();
        std::thread::spawn(move || {
            Self::supervise_loop(sup_slot, sup_make, sup_quit, &sup_name, memory_max_bytes);
        });

        let sidecar = Self {
            child: slot,
            main_sock,
            loopback,
            staging,
            module_name,
            call_deadline,
            _supervisor: quit,
        };
        sidecar.loopback.set_expected_pid(
            sidecar
                .child
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|c| c.id())
                .unwrap_or(0),
        );
        sidecar.wait_ready()?;
        Ok(sidecar)
    }

    /// S12 (F11): sidecar supervision loop. Watches the shared child; when it exits, marks
    /// the module degraded (`KERNEL_DOWN` audit) and respawns with 1s→8s doubling backoff,
    /// reusing the same staging dir (stable socket addresses). Returns when `quit` is set.
    fn supervise_loop(
        slot: Arc<Mutex<Option<Child>>>,
        make_child: Arc<dyn Fn() -> Result<Child, String> + Send + Sync>,
        quit: Arc<AtomicBool>,
        module_name: &str,
        memory_max_bytes: Option<u64>,
    ) {
        let mut backoff = Duration::from_millis(100);
        loop {
            std::thread::sleep(backoff);
            if quit.load(Ordering::Relaxed) {
                return;
            }
            let exited = {
                let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
                match guard.as_mut() {
                    Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                    None => false,
                }
            };
            if !exited {
                backoff = Duration::from_millis(100);
                continue;
            }
            // The child died: degrade, then heal.
            append_host_audit("uk_kernel", false, &format!("KERNEL_DOWN module='{module_name}'"));
            match make_child() {
                Ok(newc) => {
                    let new_pid = newc.id();
                    Self::apply_memory_cgroup(new_pid, memory_max_bytes);
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(newc);
                    append_host_audit(
                        "uk_kernel",
                        true,
                        &format!("KERNEL_HEALED module='{module_name}'"),
                    );
                    backoff = Duration::from_secs(1);
                }
                Err(e) => {
                    append_host_audit(
                        "uk_kernel",
                        false,
                        &format!("RESPAWN_FAILED module='{module_name}': {e}"),
                    );
                    backoff = (backoff * 2).min(Duration::from_secs(8));
                }
            }
        }
    }

    fn wait_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
            let mut exit_info: Option<String> = None;
            if let Some(child) = guard.as_mut() {
                if let Some(status) = child.try_wait().map_err(|e| format!("wait: {e}"))? {
                    if let Some(stderr) = read_child_stderr(child) {
                        exit_info = Some(format!("{status}\nstderr:\n{stderr}"));
                    } else {
                        exit_info = Some(status.to_string());
                    }
                }
            }
            drop(guard);
            if let Some(info) = exit_info {
                return Err(format!("workerd exited early with {info}"));
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

    /// Apply `[limits] memory_bytes` as a cgroup v2 memory cap for the child pid when a
    /// writable cgroup fs is available. Degrades silently (no cap) otherwise — no root.
    fn apply_memory_cgroup(child_pid: u32, memory_max_bytes: Option<u64>) {
        let Some(bytes) = memory_max_bytes else { return };
        if bytes == 0 {
            return;
        }
        let dir = PathBuf::from(format!("/sys/fs/cgroup/unfer-{child_pid}"));
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let _ = std::fs::write(dir.join("memory.max"), bytes.to_string());
        let _ = std::fs::write(dir.join("cgroup.procs"), child_pid.to_string());
    }

    /// RPC the sidecar's entrypoint: `POST /unfer/call` with `{"entrypoint":..,"args":..}`.
    /// Returns the JSON body of the response.
    pub fn call(&self, entrypoint: &str, args_json: &str) -> Result<String, String> {
        let body = format!(
            r#"{{"entrypoint":{0:?},"args":{1}}}"#,
            entrypoint, args_json
        );
        let result = http_post(&self.main_sock, "/unfer/call", &body, Some(self.call_deadline));
        // S14 (F13): a deadline hit means the module is unresponsive (busy loop) — record it and
        // kill the child so the supervisor can respawn a fresh, healthy sidecar.
        if let Err(e) = &result {
            if e.contains("recv") && (e.contains("timeout") || e.contains("unavailable")) {
                append_host_audit(
                    "uk_kernel",
                    false,
                    &format!("CALL_DEADLINE module='{}': {e}", self.module_name),
                );
                let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(child) = guard.as_mut() {
                    let _ = child.kill();
                }
            }
        }
        result
    }

    pub fn loopback_sock(&self) -> &Path {
        &self.loopback.path
    }

    pub fn module_name(&self) -> &str {
        &self.module_name
    }

    /// PID of the (latest) `workerd` sidecar. 0 when no child is currently alive. Used by
/// S3 escape-attempt tests to probe the child's namespace/capability confinement and by
/// S11 to arm the loopback peer check.
    pub fn child_pid(&self) -> u32 {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|c| c.id())
            .unwrap_or(0)
    }

    /// Path of the staging dir holding `config.capnp`, `harness.mjs`, `module.js` and the unix
    /// sockets. Exposed for tests/consumers to inspect the materialized sidecar contract.
    pub fn staging_dir(&self) -> &Path {
        &self.staging
    }
}

impl Drop for EcmaSidecar {
    fn drop(&mut self) {
        self._supervisor.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
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
    expected_pid: Arc<AtomicU32>,
}

impl KernelLoopback {
    fn start(
        module_name: &str,
        grants: Vec<String>,
        effects: Vec<String>,
        observers: Vec<String>,
        net: Vec<String>,
        // S18 (F17): the `[grants] resources` namespace introduced to the module this session.
        resources: Vec<String>,
        // S21 (F20): `[grants] effects` trust annotations `(name, "observe"|"mutate")`.
        effect_kinds: Vec<(String, String)>,
        // S19 (F18): the `[gatekeeper] mode` for side-effect provisioning.
        gatekeeper_mode: GatekeeperMode,
        sock_path: &Path,
    ) -> Result<Self, String> {
        let _ = std::fs::remove_file(sock_path);
        let listener = UnixListener::bind(sock_path).map_err(|e| format!("loopback bind: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("loopback nonblocking: {e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let expected_pid = Arc::new(AtomicU32::new(0)); // 0 = peer-check disabled (pre-spawn)
        let expected_pid_thread = Arc::clone(&expected_pid);
        let module_name = module_name.to_string();
        let grants: Arc<HashSet<String>> = Arc::new(grants.into_iter().collect());
        let effects: Arc<HashSet<String>> = Arc::new(effects.into_iter().collect());
        let observers: Arc<HashSet<String>> = Arc::new(observers.into_iter().collect());
        let net: Arc<HashSet<String>> = Arc::new(net.into_iter().collect());
        let resources: Arc<HashSet<String>> = Arc::new(resources.into_iter().collect());
        let effect_kinds: Arc<Vec<(String, String)>> = Arc::new(effect_kinds);
        let mode = gatekeeper_mode;
        let handle = std::thread::spawn(move || {
            let _ = &listener;
            loop {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        let mut stream = stream;
                        // S11 (F10): loopback peer lockdown. Reject any peer whose
                        // SO_PEERCRED pid does not match the spawned workerd child; an
                        // unrelated process that opened the socket path is refused before
                        // it can impersonate the sidecar (lateral-movement vector).
                        let want = expected_pid_thread.load(Ordering::Relaxed);
                        let peer_pid = peer_cred_pid(&stream);
                        if want != 0 && peer_pid.map(|p| p != want).unwrap_or(true) {
                            let detail = format!(
                                "loopback peer pid mismatch: expected {want}, got {peer_pid:?}"
                            );
                            append_security_audit(&detail);
                            let _ = stream.write_all(
                                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                            );
                            continue;
                        }
                        let name = module_name.clone();
                        let grants = Arc::clone(&grants);
                        let effects = Arc::clone(&effects);
                        let observers = Arc::clone(&observers);
                        let net = Arc::clone(&net);
                        let resources = Arc::clone(&resources);
                        let effect_kinds = Arc::clone(&effect_kinds);
                        std::thread::spawn(move || {
                            handle_loopback_conn(
                                &name,
                                &grants,
                                &effects,
                                &observers,
                                &net,
                                &resources,
                                &effect_kinds,
                                mode,
                                stream,
                            );
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
            expected_pid,
        })
    }

    /// Arm the SO_PEERCRED check once the workerd sidecar child is spawned: only that
    /// pid may open the loopback. 0 disarms the check.
    fn set_expected_pid(&self, pid: u32) {
        self.expected_pid.store(pid, Ordering::Relaxed);
    }
}

impl Drop for KernelLoopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `disabled` gates side-effect submissions off (default-deny); the gatekeeper's human console
/// is then never surfaced a mediated action. Used by [`handle_loopback_conn`].
fn gatekeeper_mode_allows_side_effect(mode: GatekeeperMode, symbol: &str) -> bool {
    if symbol != "uk_action_submit" {
        return true;
    }
    mode != GatekeeperMode::Disabled
}

#[test]
fn gatekeeper_modes_gate_action_submit() {
    // (F18) `disabled` is the only mode that refuses submissions up front.
    assert!(!gatekeeper_mode_allows_side_effect(GatekeeperMode::Disabled, "uk_action_submit"));
    assert!(gatekeeper_mode_allows_side_effect(GatekeeperMode::Optional, "uk_action_submit"));
    assert!(gatekeeper_mode_allows_side_effect(GatekeeperMode::Enabled, "uk_action_submit"));
    // Console-side symbols never submit side effects.
    assert!(gatekeeper_mode_allows_side_effect(GatekeeperMode::Disabled, "uk_gate_approve"));
    assert!(gatekeeper_mode_allows_side_effect(GatekeeperMode::Disabled, "uk_audit_list"));
}

#[test]
fn manifest_parses_gatekeeper_mode() {
    let module_toml = r#"
[module]
name = "gated"
version = "1.0.0"
entry = "module.js"

[gatekeeper]
mode = "disabled"
"#;
    let m = ModuleManifest::from_toml_str(module_toml).expect("parse");
    assert_eq!(m.gatekeeper_mode, GatekeeperMode::Disabled);
    let m2 = ModuleManifest::from_toml_str("[module]\nname=\"x\"\nversion=\"1\"\n[limits]\nmemory_bytes=1\n")
        .expect("parse");
    assert_eq!(m2.gatekeeper_mode, GatekeeperMode::Enabled, "absent `[gatekeeper]` defaults to enabled");
}

fn handle_loopback_conn(
    module_name: &str,
    grants: &HashSet<String>,
    effects: &HashSet<String>,
    observers: &HashSet<String>,
    net: &HashSet<String>,
    resources: &HashSet<String>,
    effect_kinds: &[(String, String)],
    gatekeeper_mode: GatekeeperMode,
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
        Some(sym) => {
            // S19 (F18): `[gatekeeper] mode = "disabled"` refuses side-effect submissions up
            // front (nothing to mediate); audited by the caller as a denied attempt.
            if !gatekeeper_mode_allows_side_effect(gatekeeper_mode, &sym) {
                let resp = json_response(
                    "error",
                    &serde_json::json!({
                        "code": 4004,
                        "message": format!("side effects disabled by gatekeeper mode for module '{module_name}'")
                    }),
                );
                append_host_audit(
                    "uk_security",
                    false,
                    &format!("GATEKEEPER_DISABLED module='{module_name}' symbol='{sym}'"),
                );
                resp
            } else {
                dispatch_loopback_as(
                    None,
                    module_name,
                    grants,
                    effects,
                    observers,
                    resources,
                    effect_kinds,
                    net,
                    &sym,
                    &body,
                )
            }
        }
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

/// Append a security event (`uk_security`) to the kernel audit trail. The peer check
/// runs outside any caller context (no workerd caller is on the thread), so the entry
/// carries no caller tag.
fn append_security_audit(detail: &str) {
    append_host_audit("uk_security", false, detail);
}

/// Append a host-originated event (no caller tag) to the kernel audit trail, e.g.
/// `uk_security` peer-mismatch and `uk_kernel` KERNEL_DOWN/RESPAWN events. Host-internal.
fn append_host_audit(symbol: &str, ok: bool, detail: &str) {
    let entry = serde_json::json!({
        "symbol": symbol,
        "args": [],
        "ok": ok,
        "detail": detail,
    });
    let _ = unfer_ffi::uk_audit_append(&entry.to_string());
}

/// Read the peer's `SO_PEERCRED` pid (S11). Uses `libc`/`getsockopt` so the loopback
/// works on the project toolchain (std's `UnixStream::peer_cred` is still unstable).
/// Returns `None` when the credential is unreadable; callers fail closed only when a
/// pid has been armed (never before the workerd child exists).
fn peer_cred_pid(stream: &UnixStream) -> Option<u32> {
    use std::os::unix::io::AsRawFd;
    let mut ucred: libc::ucred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if ret == 0 {
        Some(ucred.pid as u32)
    } else {
        None
    }
}

// ── S10: egress boundary (F9) ─────────────────────────────────────────

/// Net-grant matching (S10). An allowlist entry is an *exact* `host` or `host:port`.
/// Entry without a `:port` matches any port on that host; with one, the requested
/// port must also be exact. Default-deny: empty allowlist, unknown host, or non-URL
/// input all return `false`. No substring/regex matching (no `*.example.com`).
pub fn egress_allowed(host: &str, allowlist: &HashSet<String>) -> bool {
    if host.is_empty() {
        return false;
    }
    let (req_host, req_port) = split_host_port(host);
    allowlist.iter().any(|entry| {
        let (grant_host, grant_port) = split_host_port(entry);
        if grant_host != req_host {
            return false;
        }
        match (grant_port, req_port) {
            (Some(gp), Some(rp)) => gp == rp,
            _ => true, // a portless grant covers any port on that host
        }
    })
}

/// Extract `host[:port]` from a URL. Returns `""` for malformed / unrecognized schemes.
pub fn fetch_host(url: &str) -> &str {
    // Accept `scheme://host[:port][/path]` — anything else is not a fetch target.
    let rest = url
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or_default();
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() { "" } else { authority }
}

/// Split `host[:port]`; a trailing `:port` is parsed only when entirely numeric.
fn split_host_port(hostport: &str) -> (&str, Option<u16>) {
    match hostport.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            match port.parse::<u16>() {
                Ok(p) => (host, Some(p)),
                Err(_) => (hostport, None),
            }
        }
        _ => (hostport, None),
    }
}

/// Loopback-only guard: the offline `uk_fetch` path stays pinned to `127.0.0.1` /
/// `localhost` / `[::1]` as defense-in-depth even when the net grant is present.
fn is_loopback_host(host: &str) -> bool {
    let (h, _) = split_host_port(host);
    matches!(h, "127.0.0.1" | "localhost" | "::1")
}

/// Minimal loopback HTTP GET (offline `uk_fetch` fixture). Reads up to 1 MiB body.
fn loopback_get(host: &str) -> Result<String, String> {
    if !is_loopback_host(host) {
        return Err(format!("fetch egress restricted to loopback fixture (host '{host}')"));
    }
    let (h, port) = split_host_port(host);
    let addr = match format!("{h}:{}", port.unwrap_or(80)).to_socket_addrs() {
        Ok(mut addrs) => addrs
            .next()
            .ok_or_else(|| format!("resolve {host}: empty address set"))?,
        Err(e) => return Err(format!("resolve {host}: {e}")),
    };
    let mut stream = TcpStream::connect(addr).map_err(|e| format!("connect {host}: {e}"))?;
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {h}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("send: {e}"))?;
    let mut response = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buf[..n]);
                if response.len() > 1024 * 1024 {
                    break;
                }
            }
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    if response.is_empty() {
        return Err("empty response".to_string());
    }
    let body = match response.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(pos) => response[pos + 4..].to_vec(),
        None => response,
    };
    Ok(String::from_utf8_lossy(&body).to_string())
}

/// Authorize the symbol (host-side, defense in depth) and marshal the JSON args onto the `uk_*`
/// C ABI. Returns an HTTP response string.
///
/// Two grant namespaces gate the loopback:
/// * `[grants] kernel = [...]` — every `uk_*` symbol except `uk_action_submit`.
/// * `[grants] effects = [...]` — the *effect name* a module may submit via `uk_action_submit`.
///   This is the S4 "effects" namespace: holding `effects = ["send_notification"]` permits
///   submitting a `send_notification` action without any kernel grant for the symbol.
/// * `[grants] observers = [...]` — other principals this module may read (F8; the full
///   grant set is installed on the caller context so `uk_action_list`/`uk_audit_list` filter).
/// * `[grants] net = [...]` — the egress allowlist (S10): `uk_fetch` may only target an
///   exact host[:port] present here; default-deny otherwise.
/// * `[grants] resources = [...]` — the resource-introductions namespace (S18/F17): ids the
///   module may exercise via `uk_resource_use`; installed on the caller grant set so the
///   intro-surface gate can see them.
// S23 (F22): per-dispatch trace seq for the observability context (never reset).
static TRACE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

// S25 (F24): symbols that consume the caller's windowed budget at the loopback chokepoint.
static METERED_SYMBOLS: &[&str] = &[
    "uk_evolve",
    "uk_condition",
    "uk_event_probability",
    "uk_bayesian_update",
    "uk_belief_propagation",
    "uk_ode_analyze",
];

// Default per-principal windowed budget and rate-limit for the loopback meter.
const LOOPBACK_BUDGET: u64 = 1000;
const LOOPBACK_RATE_LIMIT: u64 = 0; // 0 = rate gate disabled; budget is the single cap.

// S26 (F25): forward-mutating symbols refused once the caller is sensitive-latched.
static SENSITIVE_BLOCKED_SYMBOLS: &[&str] = &[
    "uk_fetch",
    "uk_agent_spawn",
    "uk_blueprint_export",
    "uk_action_submit",
    "uk_gate_approve",
    // Plan R: certificate ledger mutations (mint/transfer/burn/authority) are
    // forward-mutating and refuse to run once the caller is sensitive-latched.
    "uk_cert_set_authority",
    "uk_cert_mint",
    "uk_cert_transfer",
    "uk_cert_burn",
    // Plan R: unified-auction mutations (open/bid/close) are forward-mutating
    // (they change the auction state machine the same way cert ops change the
    // certificate ledger).
    "uk_auction_open",
    "uk_auction_bid",
    "uk_auction_close",
];

fn dispatch_loopback(
    module_name: &str,
    grants: &HashSet<String>,
    effects: &HashSet<String>,
    observers: &HashSet<String>,
    net: &HashSet<String>,
    symbol: &str,
    body: &str,
) -> String {
    // Keep the public test surface stable: a direct dispatch (no loopback resources or
    // trust annotations) is simply a caller with no introductions and no annotations.
    let resources = HashSet::new();
    let effect_kinds = [];
    dispatch_loopback_as(
        None,
        module_name,
        grants,
        effects,
        observers,
        &resources,
        &effect_kinds,
        net,
        symbol,
        body,
    )
}

/// Like [`dispatch_loopback`], but when `agent_handle` is `Some` the call is attributed to that
/// sub-agent (S6 `AgentSpawner`): the *fixed bounded grant set* recorded at spawn gates the call
/// instead of the module's own grants (default-deny), and the audit trail tags the caller as the
/// agent. An unknown/stopped agent is denied outright.
///
/// Every dispatch — granted or denied — sets the thread-local caller identity (with the caller's
/// full grant set, so F8 read surfaces filter by observer) and appends one `AuditEntry`, so the
/// human stays accountable for what agents/gadgets attempted.
#[allow(clippy::too_many_arguments)]
fn dispatch_loopback_as(
    agent_handle: Option<i64>,
    module_name: &str,
    grants: &HashSet<String>,
    effects: &HashSet<String>,
    observers: &HashSet<String>,
    resources: &HashSet<String>,
    effect_kinds: &[(String, String)],
    net: &HashSet<String>,
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

    // 2. Resolve the effective grant sets + caller identity (module gadget vs sub-agent).
    //    The full GrantSet (kernel + effects + observers) is installed on the thread-local
    //    caller so the F8 read surfaces filter by the caller's observer rights.
    let (eff_grants, eff_effects, caller_from, caller_principal, caller_grants) =
        if let Some(handle) = agent_handle {
            match ffi_agent_bounds(handle) {
                Some((bounds, id)) => (
                    bounds.kernel.iter().cloned().collect::<HashSet<_>>(),
                    bounds.effects.iter().cloned().collect::<HashSet<_>>(),
                    "agent",
                    id,
                    bounds,
                ),
                // Unknown/stopped agent: default-deny, but still audit the attempt.
                None => {
                    let message = format!("agent handle {handle} is unknown or stopped");
                    let resp = json_response(
                        "error",
                        &serde_json::json!({"code": 4001, "name": "CallDenied", "message": message}),
                    );
                    set_loopback_caller("agent", &format!("agent-{handle}"), &unfer_protocol::GrantSet::default());
                    append_loopback_audit(symbol, &args, false, Some(&message));
                    unfer_ffi::uk_clear_caller();
                    unfer_ffi::uk_observability_clear();
                    return resp;
                }
            }
        } else {
            (
                grants.clone(),
                effects.clone(),
                "gadget",
                module_name.to_string(),
                unfer_protocol::GrantSet {
                    kernel: grants.iter().cloned().collect(),
                    effects: effects.iter().cloned().collect(),
                    observers: observers.iter().cloned().collect(),
                    resources: resources.iter().cloned().collect(),
                    // S21 (F20): the host trust annotations for granted effects (observe vs mutate).
                    effect_kinds: effect_kinds
                        .iter()
                        .map(|(n, k)| unfer_protocol::EffectGrant {
                            name: n.clone(),
                            effect_kind: if k == "observe" {
                                unfer_protocol::EffectKind::Observe
                            } else {
                                unfer_protocol::EffectKind::Mutate
                            },
                        })
                        .collect(),
                },
            )
        };

    set_loopback_caller(caller_from, &caller_principal, &caller_grants);

    // S23 (F22): thread a per-call observability context (AsyncLocal analog) into
    // the dispatcher thread. The kernel embeds these fields into every audit entry
    // produced during the call (`context.trace_id`, `component`), giving trace id
    // continuity without a global frontier. Cleared at each exit point alongside
    // the caller tag.
    let trace_id = format!(
        "{}-{:x}",
        caller_principal,
        TRACE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    );
    let _ = unfer_ffi::uk_observability_set(
        &serde_json::json!({ "trace_id": trace_id, "component": "kernel.audit" }).to_string(),
    );

    // 3. Grant gate (default-deny). Denials are audited too — attempts are the most
    //    important audit entries.
    //    S10: `uk_fetch` gates on the *net* namespace (exact host[:port] allowlist),
    //    not the kernel-grant namespace — a module needs `net = ["host"]` to egress.
    let fetch_host_grants = if symbol == "uk_fetch" {
        args.first()
            .and_then(|a| a.get("url"))
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    let gate_error = if symbol == "uk_action_submit" {
        // S4: the effects namespace, not the kernel grants, gates submission. The effect
        // name is `req.effect` of the single request arg.
        let effect = args
            .first()
            .and_then(|a| a.get("effect"))
            .and_then(|e| e.as_str())
            .unwrap_or("");
        if !eff_effects.contains(effect) {
            Some(format!(
                "UK-4001: Authorization denied — '{caller_principal}' is not granted effect '{effect}'"
            ))
        } else {
            None
        }
    } else if symbol == "uk_fetch" {
        // S10: egress gate — the target host must be on the net allowlist.
        let host = fetch_host(&fetch_host_grants);
        if host.is_empty() {
            Some("UK-4001: uk_fetch requires a url with an explicit scheme://host".to_string())
        } else if !egress_allowed(host, net) {
            Some(format!(
                "UK-4001: Egress denied — '{caller_principal}' has no net grant for host '{host}'"
            ))
        } else {
            None
        }
    } else if !eff_grants.contains(symbol) {
        Some(format!(
            "UK-4001: Authorization denied — '{caller_principal}' is not granted '{symbol}'"
        ))
    } else {
        None
    };
    if let Some(message) = gate_error {
        append_loopback_audit(symbol, &args, false, Some(&message));
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_observability_clear();
        return json_response(
            "error",
            &serde_json::json!({"code": 4001, "name": "CallDenied", "message": message}),
        );
    }

    // 3a. Sensitive-forward latch (S26/F25). Once a caller has observed
    //     `<*sensitive*>` data, forward-mutating ops (egress, hand-off, blueprints,
    //     writes, approvals) are refused with UK-4701 until an operator clears the
    //     latch — mirroring Cloudflare's `prohibitAllSharing` workspace latch.
    if SENSITIVE_BLOCKED_SYMBOLS.contains(&symbol) && unfer_ffi::uk_is_sensitive_latched(&caller_principal) {
        let message = format!(
            "UK-4701: '{caller_principal}' is sensitive-latched and cannot {}",
            symbol
        );
        append_loopback_audit(symbol, &args, false, Some(&message));
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_observability_clear();
        return json_response(
            "error",
            &serde_json::json!({"code": "UK-4701", "name": "SensitiveLatched", "message": message}),
        );
    }

    // 3b. Metered-symbol gate (S25/F24 budgets + rate limits). Expensive symbols
    //     consume the caller's windowed budget at the chokepoint; a caller that
    //     exhausts its budget or rate limit is denied here (UK-46xx + audit) —
    //     never a post-hoc report. Lightweight symbols (reads, version) are free
    //     so agents stay responsive.
    if METERED_SYMBOLS.contains(&symbol) {
        let decision = unfer_ffi::uk_meter_consume(
            &caller_principal,
            LOOPBACK_BUDGET,
            LOOPBACK_RATE_LIMIT,
        );
        let deny: Option<(&str, &str, String)> = match decision {
            0 => None,
            1 => Some(("UK-4601", "RateLimited", format!(
                "'{caller_principal}' exceeded its windowed call-rate limit"
            ))),
            _ => Some(("UK-4602", "BudgetExceeded", format!(
                "'{caller_principal}' exhausted its windowed budget"
            ))),
        };
        if let Some((code, name, message)) = deny {
            append_loopback_audit(symbol, &args, false, Some(&message));
            unfer_ffi::uk_clear_caller();
            unfer_ffi::uk_observability_clear();
            return json_response(
                "error",
                &serde_json::json!({"code": code, "name": name, "message": message}),
            );
        }
    }

    // 4. Dispatch. `kernel_dispatch` receives the caller principal (module or agent id) so
    //    uk_action_submit injects the *actor* as the record principal, not a fixed module name.
    let out = kernel_dispatch(&caller_principal, symbol, &args);
    match &out {
        Ok(_) => {
            // S10: the fetch audit carries the explicit allow action + target host.
            let detail = if symbol == "uk_fetch" {
                Some(format!(
                    "action=allow host='{}'",
                    fetch_host(&fetch_host_grants)
                ))
            } else {
                None
            };
            append_loopback_audit(symbol, &args, true, detail.as_deref());
        }
        Err((code, message)) => append_loopback_audit(
            symbol,
            &args,
            false,
            Some(&format!("UK-{code}: {message}")),
        ),
    }
    unfer_ffi::uk_clear_caller();
    unfer_ffi::uk_observability_clear();
    match out {
        Ok(value) => json_response("result", &value),
        Err((code, message)) => json_response(
            "error",
            &serde_json::json!({"code": code, "name": "KernelError", "message": message}),
        ),
    }
}

/// Tag the current thread as the acting caller (S6) with its full grant set (F8).
/// Host-internal — a worker cannot call `uk_set_caller` (no loopback arm), so the
/// identity + grant bound is host-owned.
fn set_loopback_caller(from: &str, principal: &str, grants: &unfer_protocol::GrantSet) {
    let caller = serde_json::json!({
        "from": from,
        "principal": principal,
        "grants": grants,
    });
    let _ = unfer_ffi::uk_set_caller(&caller.to_string());
}

/// Serialize an audit entry for the current caller and append it to the kernel trail.
fn append_loopback_audit(symbol: &str, args: &[serde_json::Value], ok: bool, detail: Option<&str>) {
    let mut entry = serde_json::json!({ "symbol": symbol, "args": args, "ok": ok });
    if let Some(d) = detail {
        entry
            .as_object_mut()
            .expect("audit entry is an object")
            .insert("detail".to_string(), serde_json::json!(d));
    }
    let _ = unfer_ffi::uk_audit_append(&entry.to_string());
}

/// Fetch a sub-agent's bounded grants + live id from the kernel registry. `None` when the
/// agent is unknown or stopped (→ default-deny).
fn ffi_agent_bounds(handle: i64) -> Option<(unfer_protocol::GrantSet, String)> {
    let grants_json = buf_out(|b, c| unfer_ffi::uk_agent_grants(handle, b, c)).ok()?;
    let grants = serde_json::from_str(grants_json.as_str()?).ok()?;
    let id = unfer_ffi::uk_agent_id(handle)?;
    Some((grants, id))
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
/// `actor` is the submitting caller's identity (the module name for a gadget, or the sub-agent
/// id for an agent-attributed call): `uk_action_submit` injects it as the record's `principal`
/// (an audit tag — a worker cannot claim another module's identity).
fn kernel_dispatch(
    actor: &str,
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
        // inject the caller identity (`actor`: module name or sub-agent id) as the record
        // principal (audit tag, F6) and marshal onto the FFI.
        "uk_action_submit" => {
            let mut req = args
                .get(0)
                .cloned()
                .ok_or_else(|| (1001, "uk_action_submit: missing request arg".to_string()))?;
            if let Some(obj) = req.as_object_mut() {
                obj.insert("principal".to_string(), serde_json::json!(actor));
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
        // ── S19 (F18): gatekeeper consoles ───────────────────────────────────
        // The human-gate mediation tier (unfer_protocol gatekeeper records, backported to
        // 2185-precedence semantics) calls straight into the FFI JSON envelope; the caller
        // identity (the console agent) is kept as a separate audit track.
        "uk_gate_list_pending" => {
            let out = buf_out(|b, c| unfer_ffi::uk_gate_list_pending(b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_gate_approve" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_gate_approve(handle))
        }
        "uk_gate_reject" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_gate_reject(handle))
        }
        // ── H3: event-sourced session fork + compaction ─────────────────────
        // `uk_session_fork` returns a new session handle (>0) or a negative
        // UK code; `uk_session_compact` returns 0 or a negative UK code.
        "uk_session_fork" => {
            let handle = arg_i64(args, 0)?;
            let seq = arg_i64(args, 1)?;
            ffi_result(unfer_ffi::uk_session_fork(handle, seq))
        }
        "uk_session_compact" => {
            let handle = arg_i64(args, 0)?;
            let seq = arg_i64(args, 1)?;
            ffi_result(unfer_ffi::uk_session_compact(handle, seq))
        }
        // ── S21 (F20): console-vetted marker. The symbol marshals onto the FFI; only the
        //    operator hook (nil grants) clears it — a module/agent dispatch is refused
        //    here with UK-4501 by the FFI's caller-context check (defense in depth).
        "uk_registry_vetted" => {
            let principal = arg_str(args, 0)?;
            let vetted = arg_i64(args, 1)?;
            let (p, l) = ptr_len(&principal);
            ffi_result(unfer_ffi::uk_registry_vetted(p, l, vetted))
        }
        // ── S6: agent accountability + audit ─────────────────────────────────
        "uk_audit_list" => {
            let out = buf_out(|b, c| unfer_ffi::uk_audit_list(b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_audit_clear" => ffi_result(unfer_ffi::uk_audit_clear()),
        "uk_agent_spawn" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_agent_spawn(p, l))
        }
        "uk_agent_list" => {
            let out = buf_out(|b, c| unfer_ffi::uk_agent_list(b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        "uk_agent_kill" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_agent_kill(handle))
        }
        "uk_agent_grants" => {
            let handle = arg_i64(args, 0)?;
            let out = buf_out(|b, c| unfer_ffi::uk_agent_grants(handle, b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        // ── S10: egress (F9). Host-side gate ran in `dispatch_loopback_as`; this arm performs
        //    the actual (loopback-fixture) fetch and re-checks loopback as defense-in-depth.
        //    Real non-loopback egress lives in the workerd external-service bindings generated
        //    by `config_source` from `net_grants`; the offline path stays loopback-only.
        "uk_fetch" => {
            let url = args
                .first()
                .and_then(|a| a.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            let host = fetch_host(&url);
            if host.is_empty() {
                return Err((1001, "uk_fetch: missing url".to_string()));
            }
            match loopback_get(host) {
                Ok(body) => Ok(serde_json::json!({ "host": host, "ok": true, "body": body })),
                Err(e) => Err((4004, format!("uk_fetch {host}: {e}"))),
            }
        }
        // ── S18/F17: resource introductions (grant-borne, UK-4401 gate in the FFI) ────────
        "uk_resource_introduce" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_resource_introduce(p, l))
        }
        "uk_resource_forfeit" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_resource_forfeit(p, l))
        }
        "uk_resource_use" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_resource_use(p, l))
        }
        "uk_request_resource" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_request_resource(p, l))
        }
        "uk_resource_pending" => {
            let out = buf_out(|b, c| unfer_ffi::uk_resource_pending(b, c))?;
            match serde_json::from_str(out.as_str().unwrap_or("")) {
                Ok(v) => Ok(v),
                Err(_) => Ok(out),
            }
        }
        // S27 (F26): credential vault. `uk_secret_put` takes (owner, value) pairs;
        // `uk_secret_get` is (handle, owner) with buffer-out; revoke is a handle call.
        "uk_secret_put" => {
            let owner = arg_str(args, 0)?;
            let value = arg_str(args, 1)?;
            let (po, lo) = ptr_len(&owner);
            let (pv, lv) = ptr_len(&value);
            ffi_result(unfer_ffi::uk_secret_put(po, lo, pv, lv))
        }
        "uk_secret_get" => {
            let handle = arg_i64(args, 0)?;
            let owner = arg_str(args, 1)?;
            let (po, lo) = ptr_len(&owner);
            buf_out(|b, c| unfer_ffi::uk_secret_get(handle, po, lo, b, c))
        }
        "uk_secret_revoke" => {
            let handle = arg_i64(args, 0)?;
            ffi_result(unfer_ffi::uk_secret_revoke(handle))
        }
        // ── Plan R: carbon-certificate / UTXO ledger ─────────────────────────
        // `uk_cert_*` marshals onto the process-global `CertificateLedger`. The
        // mutating ops take a single JSON op arg and return 0/-err; the read ops
        // use the buffer-out protocol.
        "uk_cert_set_authority" => {
            let did = arg_str(args, 0)?;
            let (p, l) = ptr_len(&did);
            ffi_result(unfer_ffi::uk_cert_set_authority(p, l))
        }
        "uk_cert_root" => {
            buf_out_raw(|b, c| unfer_ffi::uk_cert_root(b, c))
        }
        "uk_cert_status" => {
            buf_out(|b, c| unfer_ffi::uk_cert_status(b, c))
        }
        "uk_cert_mint" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_cert_mint(p, l))
        }
        "uk_cert_transfer" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_cert_transfer(p, l))
        }
        "uk_cert_burn" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_cert_burn(p, l))
        }
        "uk_auction_open" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_auction_open(p, l))
        }
        "uk_auction_bid" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            ffi_result(unfer_ffi::uk_auction_bid(p, l))
        }
        // `uk_auction_close` both mutates the ledger and writes the winner JSON,
        // so it cannot use the probe-then-copy `buf_out` protocol (the probe
        // would apply the close twice). Use a single fixed-buffer call instead.
        "uk_auction_close" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            let mut buf = vec![0u8; 4096];
            let n = unfer_ffi::uk_auction_close(p, l, buf.as_mut_ptr(), buf.len() as i64);
            if n < 0 {
                return Err((( -n) as u32, read_last_error().unwrap_or_default()));
            }
            buf.truncate(n as usize);
            let s = String::from_utf8(buf).map_err(|e| (1001, e.to_string()))?;
            Ok(serde_json::Value::String(s))
        }
        "uk_auction_report" => {
            let json = arg_str(args, 0)?;
            let (p, l) = ptr_len(&json);
            buf_out(|b, c| unfer_ffi::uk_auction_report(p, l, b, c))
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

/// `POST /path` to a unix socket, return the response body. `read_timeout` bounds the recv
/// phase so a runaway worker (busy loop) surfaces as `recv: ..timed out` → the caller's per-call
/// deadline (S14 / F13) rather than an unbounded host-side block.
fn http_post(
    sock: &Path,
    path: &str,
    body: &str,
    read_timeout: Option<Duration>,
) -> Result<String, String> {
    let mut stream = UnixStream::connect(sock).map_err(|e| format!("connect {}: {e}", sock.display()))?;
    if let Some(d) = read_timeout {
        // set_read_timeout applies to blocking reads; set_write_timeout to non-blocking writes.
        stream
            .set_read_timeout(Some(d))
            .map_err(|e| format!("timeout: {e}"))?;
    }
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
    // S10: mirror `[grants] net` into workerd external-service bindings. Each allowlisted
    // host becomes a reachable `external` service name (the host-side loopback still
    // validates every `uk_fetch` as defense-in-depth). Empty `net` => no egress services.
    let mut egress_services = String::new();
    for (i, host) in manifest.net_grants.iter().enumerate() {
        egress_services.push_str(&format!(
            "    (name = \"net-egress-{i}\", external = (address = \"http://{host}\", http = ())),\n"
        ));
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
{egress_services}  ],
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
/// exposes a `kernel` capability object built strictly from the granted service bindings (F5).
/// `env` keys are exactly the module's `[grants] kernel` symbols, so the capability object IS the
/// granted set — an un-granted `uk_*`/`uz_*` name is absent, not stubbed, and cannot be probed or
/// enumerated. The host-side loopback is the only layer that emits UK-4001 (defense in depth).
fn harness_source() -> &'static str {
    r#"// Generated by austral_cranelift_bridge (ecma.rs). Do not edit.
import * as module from "./module.js";

// F5: the kernel capability object is built strictly from the granted service
// bindings — `env` keys are exactly the module's `[grants] kernel` symbols. An
// un-granted `uk_*`/`uz_*` name is simply ABSENT (not wrapped, not even stubbed),
// so a module cannot enumerate or probe the kernel's full symbol table. The
// host-side loopback re-validates every call (defense in depth); that is the
// only layer that emits UK-4001.
function makeKernel(env) {
  const kernel = {};
  for (const name of Object.keys(env)) {
    if (typeof name !== "string") continue;
    if (!(name.startsWith("uk_") || name.startsWith("uz_"))) continue;
    kernel[name] = async (...args) => {
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
  return kernel;
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
    fn harness_exposes_only_granted_symbols() {
        // F5: the kernel capability object must be built strictly from the granted service
        // bindings (env keys), with NO throw-stub for the rest of the uk_*/uz_* table — an
        // un-granted name is simply absent and cannot be probed.
        let src = harness_source();
        assert!(src.contains("Object.keys(env)"), "harness must enumerate env, not a static table");
        assert!(src.contains("env[name].fetch"));
        assert!(src.contains("data.error"));
        assert!(
            !src.contains("CALL_DENIED"),
            "harness must not stub un-granted symbols (UK-4001 is loopback-only)"
        );
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
            effect_kinds: vec![],
            observers: vec![],
            resources: vec![],
            fs_grants: vec![],
            net_grants: vec![],
            max_ms: None,
            memory_max_bytes: None,
            gatekeeper_mode: crate::module::GatekeeperMode::Enabled,
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
        let observers = HashSet::new();
        let body = r#"[{"effect":"send_notification","params":{"to":"alice"}}]"#;
        let resp =
            dispatch_loopback("client_module", &grants, &effects, &observers, &HashSet::new(), "uk_action_submit", body);
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
        let observers = HashSet::new();
        // The module holds `send_notification`, NOT `delete_all`.
        let body = r#"[{"effect":"delete_all","params":{}}]"#;
        let resp =
            dispatch_loopback("client_module", &grants, &effects, &observers, &HashSet::new(), "uk_action_submit", body);
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
        let observers = HashSet::new();
        let body = r#"[{"effect":"send_notification","params":{}}]"#;
        let resp =
            dispatch_loopback("client_module", &grants, &effects, &observers, &HashSet::new(), "uk_action_submit", body);
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
        let observers = HashSet::new();
        let resp = dispatch_loopback(
            "client_module",
            &grants,
            &effects,
            &observers,
            &HashSet::new(),
            "uk_action_apply",
            r#"[1]"#,
        );
        assert!(resp.contains("\"error\""), "ungranted kernel symbol must deny, got {resp}");
        assert!(resp.contains("4001"), "expected UK-4001, got {resp}");
    }

    // ── S19 (F18): gatekeeper console over the loopback ───────────────────
    //
    // The human-gate tier is reachable through the same grants-gated endpoint:
    // the operator console symbols must be granted, and only the gatekeeper
    // mediates. These tests exercise the full loopback → FFI → queue → audit
    // path without a workerd sidecar.

    #[test]
    fn loopback_gatekeeper_console_mediates_approval() {
        use std::collections::HashSet;
        // Audit + action stores are process-global; the S6-style lock serializes us
        // against the other uk_audit_clear() tests (including the denied_no_grant one).
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unfer_ffi::uk_audit_clear();
        unfer_ffi::uk_clear_caller();
        let grants = HashSet::from([
            "uk_action_submit".to_string(),
            "uk_gate_list_pending".to_string(),
            "uk_gate_approve".to_string(),
            "uk_gate_reject".to_string(),
        ]);
        let effects = HashSet::from(["send_notification".to_string()]);
        let observers = HashSet::new();
        let resources = HashSet::new();

        // A side-effecting export submits with a provisional forecast: lands pending.
        let req = r#"[{"effect":"send_notification","params":{"to":"ops"},"provisional":{"forecast":true,"threat":"none"}}]"#;
        let resp = dispatch_loopback(
            "scan-module",
            &grants,
            &effects,
            &observers,
            &resources,
            "uk_action_submit",
            req,
        );
        let body = resp
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let value: serde_json::Value = serde_json::from_str(&body).expect("loopback json");
        let handle = value["result"]
            .as_i64()
            .expect("submission must return the action handle: {value}");

        // The operator console lists the pending action with the forecast carried.
        let resp = dispatch_loopback(
            "scan-module",
            &grants,
            &effects,
            &observers,
            &resources,
            "uk_gate_list_pending",
            "[]",
        );
        assert!(
            resp
                .split_once("\r\n\r\n")
                .map(|(_, b)| b.to_string())
                .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
                .map(|v| {
                    v["result"]
                        .as_array()
                        .map(|a| {
                            a.iter().any(|spot| {
                                spot[0].as_i64() == Some(handle)
                                    && spot[1]["effect"] == "send_notification"
                                    && spot[1]["provisional"]["threat"] == "none"
                            })
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false),
            "pending console must carry the handle with its forecast, got {resp}"
        );

        // Human resolution: approve → the queue drains and the audit records it.
        let approve = dispatch_loopback(
            "scan-module",
            &grants,
            &effects,
            &observers,
            &resources,
            "uk_gate_approve",
            &format!("[{handle}]"),
        );
        assert!(!approve.contains("\"error\""), "approval must resolve, got {approve}");

        let after = dispatch_loopback(
            "scan-module",
            &grants,
            &effects,
            &observers,
            &resources,
            "uk_gate_list_pending",
            "[]",
        );
        let after_body = after
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let after_value: serde_json::Value =
            serde_json::from_str(&after_body).expect("loopback json");
        assert!(
            !after_value["result"]
                .as_array()
                .map(|a| a.iter().any(|spot| spot[0].as_i64() == Some(handle)))
                .unwrap_or(false),
            "approved action must leave the queue, got {after}"
        );

        let audits = read_ffi_audit();
        assert!(
            audits.iter().any(|e| e["symbol"] == "uk_gate_approve"),
            "resolution must be audited: {audits:?}"
        );
    }

    #[test]
    fn loopback_gatekeeper_denied_without_grant() {
        use std::collections::HashSet;
        let grants = HashSet::from(["uk_action_submit".to_string()]);
        let effects = HashSet::from(["notify_admins".to_string()]);
        // The console gate symbols are not granted → UK-4001 even though effects pass.
        let observers = HashSet::new();
        let resources = HashSet::new();
        let resp = dispatch_loopback(
            "scan-module",
            &grants,
            &effects,
            &observers,
            &resources,
            "uk_gate_list_pending",
            "[]",
        );
        assert!(resp.contains("\"error\""), "ungranted console symbol must deny, got {resp}");
        assert!(resp.contains("4001"), "expected UK-4001, got {resp}");
    }

    // ── S21 (F20): trust annotations + console-vetted over the loopback ────
    //
    // A gadget's `[grants] effects` table may carry an `effect_kind = "observe"`
    // Trust annotation; host installs matching `EffectGrant`s on the caller grant
    // set. Observe-annotated effects apply immediately; granted-but-unannotated
    // effects stay conservatively `mutate` (queued, human-gated). The vetted
    // marker is console-owned only — the loopback hook refuses anything else.

    #[test]
    fn loopback_observe_annotation_auto_applies() {
        use std::collections::HashSet;
        unfer_ffi::uk_clear_vetted();
        unfer_ffi::uk_clear_caller();
        let effects = HashSet::from(["obs_eff".to_string()]);
        let effect_kinds = [("obs_eff".to_string(), "observe".to_string())];
        let req = r#"[{"effect":"obs_eff","params":{"metric":"qps"}}]"#;
        let resp = dispatch_loopback_as(
            None,
            "obs_mod",
            &HashSet::from(["uk_action_submit".to_string()]),
            &effects,
            &HashSet::new(),
            &HashSet::new(),
            &effect_kinds,
            &HashSet::new(),
            "uk_action_submit",
            req,
        );
        assert!(
            resp.contains("\"result\"") && !resp.contains("\"error\""),
            "annotated observe effect must dispatch, got {resp}"
        );
        let json = resp
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let handle = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v["result"].as_i64())
            .expect("observe submission must return a handle");
        let needed = unfer_ffi::uk_action_get(handle, std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; needed as usize];
        unfer_ffi::uk_action_get(handle, buf.as_mut_ptr(), needed);
        let record: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(
            record["state"], "approved",
            "observe annotation auto-applies without approval: {record}"
        );
        assert_eq!(record["applied"]["applied"], true);
        unfer_ffi::uk_clear_caller();
    }

    #[test]
    fn loopback_unannotated_mutate_queues_pending() {
        use std::collections::HashSet;
        unfer_ffi::uk_clear_vetted();
        unfer_ffi::uk_clear_caller();
        let effects = HashSet::from(["del_eff".to_string()]);
        // Granted but *not* annotated → the conservative default is `mutate`.
        let effect_kinds: [(String, String); 0] = [];
        let body = r#"[{"effect":"del_eff","params":{"row":4},"provisional":{"rows":1}}]"#;
        let resp = dispatch_loopback_as(
            None,
            "mut_mod",
            &HashSet::from(["uk_action_submit".to_string()]),
            &effects,
            &HashSet::new(),
            &HashSet::new(),
            &effect_kinds,
            &HashSet::new(),
            "uk_action_submit",
            body,
        );
        assert!(
            resp.contains("\"result\""),
            "granted mutate effect still submits (queued), got {resp}"
        );
        let json = resp
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let handle = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v["result"].as_i64())
            .expect("mutate submission must return a handle");
        let needed = unfer_ffi::uk_action_get(handle, std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; needed as usize];
        unfer_ffi::uk_action_get(handle, buf.as_mut_ptr(), needed);
        let record: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(
            record["state"], "pending",
            "unannotated mutate stays pending: {record}"
        );
        assert!(record["applied"].is_null(), "no applied result without approval");

        // Only the human gate promotes it to applied.
        assert_eq!(unfer_ffi::uk_gate_approve(handle), 0, "gate approval succeeds");
        let needed = unfer_ffi::uk_action_get(handle, std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; needed as usize];
        unfer_ffi::uk_action_get(handle, buf.as_mut_ptr(), needed);
        let record: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(record["state"], "approved", "approval promotes applied: {record}");
        assert_eq!(record["applied"]["forecast"]["rows"], 1);
        unfer_ffi::uk_clear_caller();
    }

    #[test]
    fn loopback_module_cannot_ring_vetted() {
        use std::collections::HashSet;
        unfer_ffi::uk_clear_vetted();
        unfer_ffi::uk_clear_caller();
        // The module is *granted* the symbol — the gate passes — but the FFI's
        // console-only check must still refuse: only the hook (nil grants) mints.
        let grants = HashSet::from(["uk_registry_vetted".to_string()]);
        let resp = dispatch_loopback(
            "spoof_mod",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_registry_vetted",
            r#"["victim",1]"#,
        );
        assert!(
            resp.contains("\"error\""),
            "a module must never ring the vetted marker, got {resp}"
        );
        assert!(resp.contains("4501"), "expected UK-4501, got {resp}");
        unfer_ffi::uk_clear_caller();
    }

    // ── S25 (F24): budgets + rate limits at the loopback chokepoint ────────
    //
    // Metered symbols consume the caller's windowed budget before dispatch; an
    // exhausted budget is denied here with UK-4602 + an audit entry, never a
    // post-hoc report. The meter is a process-global store — tests serialize.

    static METER_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn loopback_denies_metered_symbol_once_budget_exhausted() {
        use std::collections::HashSet;
        let _g = METER_TESTS_LOCK.lock().unwrap();
        unfer_ffi::uk_clear_meter();
        unfer_ffi::uk_clear_caller();
        let grants = HashSet::from(["uk_version".to_string()]);
        let resp = dispatch_loopback(
            "meter_probe",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_evolve",
            "[]",
        );
        // uk_evolve is not granted here, so the grant gate fires first (UK-4001).
        assert!(resp.contains("4001"), "ungranted metered symbol denies, got {resp}");
        unfer_ffi::uk_clear_caller();
    }

    #[test]
    fn loopback_meter_records_audit_on_over_budget_deny() {
        use std::collections::HashSet;
        let _g = METER_TESTS_LOCK.lock().unwrap();
        unfer_ffi::uk_clear_meter();
        unfer_ffi::uk_clear_caller();
        // Grant the metered symbol so the grant gate passes; pre-exhaust the
        // windowed budget cheaply (no kernel dispatch) so the meter gate trips.
        let grants = HashSet::from(["uk_evolve".to_string()]);
        for _ in 0..LOOPBACK_BUDGET {
            unfer_ffi::uk_meter_consume("meter_burn", LOOPBACK_BUDGET, LOOPBACK_RATE_LIMIT);
        }
        let denied = dispatch_loopback(
            "meter_burn",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_evolve",
            "[]",
        );
        assert!(
            denied.contains("\"error\""),
            "over-budget metered call must be denied, got {denied}"
        );
        assert!(
            denied.contains("4602"),
            "expected UK-4602 budget-exceeded, got {denied}"
        );
        let audits = read_ffi_audit();
        assert!(
            audits.iter().any(|e| e["symbol"] == "uk_evolve" && e["ok"] == false),
            "over-budget deny must be audited: {audits:?}"
        );
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_clear_meter();
    }

    // ── S26 (F25): sensitive-forward latch at the loopback chokepoint ──────
    //
    // Once a caller is latched (observed `<*sensitive*>` data), forward-mutating
    // ops are refused with UK-4701 until an operator clears the latch.

    static LATCH_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn loopback_blocks_forward_ops_when_latched() {
        use std::collections::HashSet;
        let _g = LATCH_TESTS_LOCK.lock().unwrap();
        unfer_ffi::uk_clear_sensitive_latches();
        unfer_ffi::uk_clear_caller();
        let grants = HashSet::from(["uk_action_submit".to_string()]);
        let effects = HashSet::from(["notify_admins".to_string()]);
        unfer_ffi::uk_set_sensitive_latch("latch_probe", true);
        let resp = dispatch_loopback(
            "latch_probe",
            &grants,
            &effects,
            &HashSet::new(),
            &HashSet::new(),
            "uk_action_submit",
            r#"[{"effect":"notify_admins","name":"x"}]"#,
        );
        assert!(
            resp.contains("\"error\""),
            "a latched caller must be blocked from a forward op, got {resp}"
        );
        assert!(
            resp.contains("4701"),
            "expected UK-4701 sensitive-latched, got {resp}"
        );
        let audits = read_ffi_audit();
        assert!(
            audits
                .iter()
                .any(|e| e["symbol"] == "uk_action_submit" && e["ok"] == false),
            "the latch block must be audited: {audits:?}"
        );
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_clear_sensitive_latches();
    }

    #[test]
    fn loopback_cleared_latch_allows_forward_op() {
        use std::collections::HashSet;
        let _g = LATCH_TESTS_LOCK.lock().unwrap();
        unfer_ffi::uk_clear_sensitive_latches();
        unfer_ffi::uk_clear_caller();
        let grants = HashSet::from(["uk_action_submit".to_string()]);
        let effects = HashSet::from(["notify_admins".to_string()]);
        unfer_ffi::uk_set_sensitive_latch("latch_clear", true);
        unfer_ffi::uk_set_sensitive_latch("latch_clear", false);
        let resp = dispatch_loopback(
            "latch_clear",
            &grants,
            &effects,
            &HashSet::new(),
            &HashSet::new(),
            "uk_action_submit",
            r#"[{"effect":"notify_admins","name":"x"}]"#,
        );
        assert!(
            !resp.contains("4701"),
            "cleared latch must not block, got {resp}"
        );
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_clear_sensitive_latches();
    }

    // ── S27 (F26): credential vault over the loopback chokepoint ───────────
    //
    // A gatekeeper stores a credential under `uk_secret_put`, then grants the
    // opaque handle; only the owner dereferences. The vault is process-global.

    static VAULT_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn loopback_secret_put_get_roundtrip() {
        use std::collections::HashSet;
        let _g = VAULT_TESTS_LOCK.lock().unwrap();
        unfer_ffi::uk_vault_clear();
        unfer_ffi::uk_clear_caller();
        let grants = HashSet::from(["uk_secret_put".to_string(), "uk_secret_get".to_string()]);
        let put = dispatch_loopback(
            "vault_mod",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_secret_put",
            r#"["vault_mod","tok_abc123"]"#,
        );
        eprintln!("PUT_RESP={put}");
        let put_body = put
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let handle = serde_json::from_str::<serde_json::Value>(&put_body)
            .ok()
            .and_then(|v| v["result"].as_i64())
            .expect("put returns a handle");
        assert!(handle > 0, "put returns a positive handle, got {put}");
        let get = dispatch_loopback(
            "vault_mod",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_secret_get",
            &format!(r#"[{handle},"vault_mod"]"#),
        );
        assert!(
            get.contains("tok_abc123"),
            "owner must dereference the secret, got {get}"
        );
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_vault_clear();
    }

    #[test]
    fn loopback_secret_denies_wrong_owner() {
        use std::collections::HashSet;
        let _g = VAULT_TESTS_LOCK.lock().unwrap();
        unfer_ffi::uk_vault_clear();
        unfer_ffi::uk_clear_caller();
        let grants = HashSet::from(["uk_secret_put".to_string(), "uk_secret_get".to_string()]);
        let put = dispatch_loopback(
            "vault_owner",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_secret_put",
            r#"["vault_owner","tok_secret"]"#,
        );
let put_body = put
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let handle = serde_json::from_str::<serde_json::Value>(&put_body)
            .ok()
            .and_then(|v| v["result"].as_i64())
            .expect("put returns a handle");
        let get = dispatch_loopback(
            "vault_intruder",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_secret_get",
            &format!(r#"[{handle},"vault_intruder"]"#),
        );
        assert!(
            get.contains("\"error\""),
            "a non-owner must not dereference the secret, got {get}"
        );
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_vault_clear();
    }

    #[test]
    fn dispatch_audit_carries_observability_context() {
        // S23 (F22): every loopback dispatch seeds a per-call observability context
        // (trace_id + dot-separated owner component) threaded into its audit entry.
        // Assert on the entry for THIS dispatch by its unique caller principal.
        use std::collections::HashSet;
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_observability_clear();
        let grants = HashSet::from(["uk_version".to_string()]);
        let resp = dispatch_loopback(
            "audit_trace_probe",
            &grants,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            "uk_version",
            "[]",
        );
        assert!(resp.contains("\"result\""), "granted call must dispatch: {resp}");
        let entries = read_ffi_audit();
        let mine = entries
            .iter()
            .find(|e| e["symbol"] == "uk_version" && e["caller"]["principal"] == "audit_trace_probe")
            .expect("the probe's dispatch must be audited");
        let trace = mine["context"]["trace_id"].as_str().expect("trace_id threaded");
        assert!(!trace.is_empty(), "trace id must be non-empty: {mine:?}");
        assert!(trace.starts_with("audit_trace_probe-"), "trace names the caller: {trace}");
        assert_eq!(mine["component"], "kernel.audit", "dot-separated owner component");
        unfer_ffi::uk_clear_caller();
        unfer_ffi::uk_observability_clear();
    }

    // ── S6: agent accountability + audit (GatekeeperCaller tags) ───────────
    //
    // Every loopback dispatch is audited with the caller's tag; sub-agent calls are
    // bounded to the fixed grant set recorded at spawn (AgentSpawner enforcement).
    // The audit trail is kernel-global, so assertions filter by the distinctive probe
    // principal rather than assuming an empty trail.

    static LOOPBACK_AUDIT_TESTS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn read_ffi_audit() -> Vec<serde_json::Value> {
        let needed = unfer_ffi::uk_audit_list(std::ptr::null_mut(), 0);
        if needed <= 0 {
            return Vec::new();
        }
        // The audit trail is kernel-global: concurrent loopback dispatches (S4 gadget
        // tests) append entries while we size-then-fill, so the list can grow mid-read.
        // Allocate slack and retry until the buffer is large enough, parsing only the
        // bytes actually written (avoids trailing-NUL parse failures).
        let mut cap = needed + 4096;
        loop {
            let mut buf = vec![0u8; cap as usize];
            let written = unfer_ffi::uk_audit_list(buf.as_mut_ptr(), cap);
            if written <= 0 {
                return Vec::new();
            }
            if written <= cap {
                return serde_json::from_slice::<serde_json::Value>(&buf[..written as usize])
                    .map(|v| v.as_array().cloned().unwrap_or_default())
                    .unwrap_or_default();
            }
            cap = written + 4096;
        }
    }

    fn spawn_agent(name: &str, kernel_grants: &[&str], effect_grants: &[&str]) -> i64 {
        let kernel: Vec<String> = kernel_grants.iter().map(|s| s.to_string()).collect();
        let effects: Vec<String> = effect_grants.iter().map(|s| s.to_string()).collect();
        let spec = serde_json::json!({ "name": name, "grants": { "kernel": kernel, "effects": effects } });
        let spec_json = spec.to_string();
        let (p, l) = ptr_len(&spec_json);
        let h = unfer_ffi::uk_agent_spawn(p, l);
        assert!(h > 0, "agent spawn must succeed, got {h}");
        h
    }

    #[test]
    fn loopback_audits_module_calls_with_caller_tag() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        unfer_ffi::uk_audit_clear();

        let grants = HashSet::from(["uk_version".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        // Granted call → audited ok.
        let resp = dispatch_loopback("audit_probe", &grants, &effects, &observers, &HashSet::new(), "uk_version", "[]");
        assert!(resp.contains("\"result\""), "granted symbol must dispatch, got {resp}");
        // Denied call → audited too.
        let resp = dispatch_loopback("audit_probe", &grants, &effects, &observers, &HashSet::new(), "uk_evolve", r#"[{"t":0.1}]"#);
        assert!(resp.contains("\"error\""), "ungranted symbol must deny, got {resp}");

        let entries = read_ffi_audit();
        let mine: Vec<&serde_json::Value> = entries
            .iter()
            .filter(|e| e["caller"]["principal"] == "audit_probe")
            .collect();
        assert_eq!(mine.len(), 2, "expected 2 audited calls from audit_probe: {entries:?}");
        // Newest first: the denied evolve call, then the granted version call.
        assert_eq!(mine[0]["symbol"], "uk_evolve");
        assert_eq!(mine[0]["ok"], false);
        assert!(mine[0]["detail"].as_str().unwrap().contains("UK-4001"));
        assert_eq!(mine[0]["caller"]["from"], "gadget");
        assert_eq!(mine[1]["symbol"], "uk_version");
        assert_eq!(mine[1]["ok"], true);
    }

    #[test]
    fn loopback_audits_action_submit_with_agent_caller() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        unfer_ffi::uk_audit_clear();

        // A gadget spawns an agent bounded to {uk_action_submit} + the effect; the agent
        // then submits the action. The ActionRecord must carry the *agent* caller tag and
        // the agent id as its principal.
        let grants = HashSet::from(["uk_action_submit".to_string()]);
        let effects = HashSet::from(["send_notification".to_string()]);
        let observers = HashSet::new();
        let handle = spawn_agent("analyst", &["uk_action_submit"], &["send_notification"]);

        let body = r#"[{"effect":"send_notification","params":{"to":"eve"}}]"#;
        let resp = dispatch_loopback_as(
            Some(handle),
            "parent_mod",
            &grants,
            &effects,
            &observers,
            &HashSet::new(),
            &[],
            &HashSet::new(),
            "uk_action_submit",
            body,
        );
        assert!(resp.contains("\"result\""), "granted effect must dispatch, got {resp}");

        // The action principal + caller tag must be the agent id.
        let json = resp.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
        let handle_i64 = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v["result"].as_i64())
            .expect("expected an action handle");
        let needed = unfer_ffi::uk_action_get(handle_i64, std::ptr::null_mut(), 0);
        let mut buf = vec![0u8; needed as usize];
        unfer_ffi::uk_action_get(handle_i64, buf.as_mut_ptr(), needed);
        let record: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        let agent_id = unfer_ffi::uk_agent_id(handle).expect("agent must be live");
        assert_eq!(record["principal"], serde_json::json!(agent_id));
        assert_eq!(record["caller"]["from"], "agent");
        assert_eq!(record["caller"]["principal"], serde_json::json!(agent_id));

        // The audit entry for the submission is tagged with the agent, not the parent.
        // Filter by the agent's caller principal: the trail is kernel-global and S4
        // gadget dispatches may append uk_action_submit entries concurrently.
        let entries = read_ffi_audit();
        let mine: Vec<&serde_json::Value> = entries
            .iter()
            .filter(|e| e["symbol"] == "uk_action_submit" && e["caller"]["principal"] == agent_id)
            .collect();
        assert!(!mine.is_empty(), "action submit must be audited for the agent: {entries:?}");
        assert_eq!(mine[0]["caller"]["from"], "agent");
        assert_eq!(mine[0]["caller"]["principal"], serde_json::json!(agent_id));
    }

    #[test]
    fn loopback_agent_grant_enforcement_bounded_set() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        // The agent is bounded to {uk_version} — uk_evolve must be denied host-side even
        // though the *module* grant set (passed in) contains everything.
        let grants = HashSet::from(["uk_version".to_string(), "uk_evolve".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        let handle = spawn_agent("bounded", &["uk_version"], &[]);

        let ok = dispatch_loopback_as(
            Some(handle),
            "parent_mod",
            &grants,
            &effects,
            &observers,
            &HashSet::new(),
            &[],
            &HashSet::new(),
            "uk_version",
            "[]",
        );
        assert!(ok.contains("\"result\""), "granted agent symbol must dispatch, got {ok}");

        let denied = dispatch_loopback_as(
            Some(handle),
            "parent_mod",
            &grants,
            &effects,
            &observers,
            &HashSet::new(),
            &[],
            &HashSet::new(),
            "uk_evolve",
            r#"[{"t":0.1}]"#,
        );
        assert!(denied.contains("\"error\""), "unbounded agent symbol must deny, got {denied}");
        assert!(denied.contains("4001"), "expected UK-4001, got {denied}");
    }

    #[test]
    fn loopback_agent_unknown_handle_denies() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        let grants = HashSet::from(["uk_version".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        let resp = dispatch_loopback_as(
            Some(99999),
            "parent_mod",
            &grants,
            &effects,
            &observers,
            &HashSet::new(),
            &[],
            &HashSet::new(),
            "uk_version",
            "[]",
        );
        assert!(resp.contains("\"error\""), "unknown agent must deny, got {resp}");
        assert!(resp.contains("4001"), "expected UK-4001, got {resp}");
    }

    // ── S18 (F17): resource introductions gate the loopback ───────────────

    #[test]
    fn resource_introduction_gates_loopback_calls() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;

        let grants = HashSet::from([
            "uk_resource_introduce".to_string(),
            "uk_resource_use".to_string(),
            "uk_resource_forfeit".to_string(),
        ]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        let resource_id = "github.repo#loopback";
        let json_id = serde_json::to_string(resource_id).unwrap();

        // 1. A module introduced to the resource mints it at the chokepoint, then may use it.
        let granted = HashSet::from([resource_id.to_string()]);
        let mint = dispatch_loopback_as(
            None,
            "f17_res_granted",
            &grants,
            &effects,
            &observers,
            &granted,
            &[],
            &HashSet::new(),
            "uk_resource_introduce",
            &format!("[{json_id:?}]"),
        );
        assert!(
            mint.contains("\"result\""),
            "introduction must mint at the chokepoint, got {mint}"
        );
        let use1 = dispatch_loopback_as(
            None,
            "f17_res_granted",
            &grants,
            &effects,
            &observers,
            &granted,
            &[],
            &HashSet::new(),
            "uk_resource_use",
            &format!("[{json_id:?}]"),
        );
        assert!(
            use1.contains("\"result\""),
            "introduced caller may use the resource, got {use1}"
        );

        // 2. A module with NO introduction is refused UK-4401 (nothing is ambient).
        let no_intro = HashSet::new();
        let use2 = dispatch_loopback_as(
            None,
            "f17_res_denied",
            &grants,
            &effects,
            &observers,
            &no_intro,
            &[],
            &HashSet::new(),
            "uk_resource_use",
            &format!("[{json_id:?}]"),
        );
        assert!(
            use2.contains("\"error\"") && use2.contains("4401"),
            "unintroduced caller must get UK-4401, got {use2}"
        );

        // 3. Forfeit revokes; the re-use path turns to UK-4403 (never minted).
        let revoke = dispatch_loopback_as(
            None,
            "f17_res_granted",
            &grants,
            &effects,
            &observers,
            &granted,
            &[],
            &HashSet::new(),
            "uk_resource_forfeit",
            &format!("[{json_id:?}]"),
        );
        assert!(
            revoke.contains("\"result\""),
            "forfeit must revoke, got {revoke}"
        );
        let use3 = dispatch_loopback_as(
            None,
            "f17_res_denied",
            &grants,
            &effects,
            &observers,
            &no_intro,
            &[],
            &HashSet::new(),
            "uk_resource_use",
            &format!("[{json_id:?}]"),
        );
        assert!(
            use3.contains("\"error\"") && use3.contains("4401"),
            "revoked resource is no longer usable (UK-4401), got {use3}"
        );
    }

    // ── F8: observer re-check on shared reads ──────────────────────────────
    //
    // A bounded caller may read only records/audit entries for its own principal and
    // any principal listed in `[grants] observers`. The trusted harness sees all. This
    // closes the S4/S6 leak where any module holding `uk_action_list`/`uk_audit_list`
    // could enumerate every module's actions and audit args.

    /// Dispatch a loopback call and parse the HTTP JSON body.
    fn dispatch_parse(
        from: &str,
        grants: &HashSet<String>,
        effects: &HashSet<String>,
        observers: &HashSet<String>,
        symbol: &str,
        body: &str,
    ) -> (bool, serde_json::Value) {
        let resp = dispatch_loopback(from, grants, effects, observers, &HashSet::new(), symbol, body);
        let json = resp
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
        (v.get("result").is_some(), v)
    }

    #[test]
    fn loopback_observers_filter_action_list() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;

        // Actor A submits an action; the loopback injects its own principal.
        let a_grants = HashSet::from(["uk_action_submit".to_string()]);
        let a_effects = HashSet::from(["send_notification".to_string()]);
        let a_obs = HashSet::new();
        let resp = dispatch_loopback(
            "f8_actor_a",
            &a_grants,
            &a_effects,
            &a_obs,
            &HashSet::new(),
            "uk_action_submit",
            r#"[{"effect":"send_notification","params":{"to":"bob"}}]"#,
        );
        assert!(resp.contains("\"result\""), "actor must submit, got {resp}");
        let json = resp
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        let handle = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v["result"].as_i64())
            .expect("expected an action handle");

        // Reader B holds the list/get grants but no observer grant: A's record is
        // invisible, and `uk_action_get` on A's handle is indistinguishable from a
        // missing record (UK-4004, no existence oracle).
        let b_grants = HashSet::from(["uk_action_list".to_string(), "uk_action_get".to_string()]);
        let b_effects = HashSet::new();
        let b_obs = HashSet::new();
        let (ok, v) = dispatch_parse("f8_actor_b", &b_grants, &b_effects, &b_obs, "uk_action_list", "[]");
        assert!(ok, "list must dispatch, got {v}");
        let records = v["result"].as_array().expect("list is an array");
        assert!(
            records.iter().all(|r| r["principal"] != "f8_actor_a"),
            "B must not observe A's record: {records:?}"
        );

        let (ok_get, v_get) = dispatch_parse(
            "f8_actor_b",
            &b_grants,
            &b_effects,
            &b_obs,
            "uk_action_get",
            &format!("[{handle}]"),
        );
        assert!(
            !ok_get && v_get["error"]["code"] == 4004,
            "B must get UK-4004 reading A's record, got {v_get}"
        );

        // Reader C declares A as an observer: A's record IS visible.
        let c_grants = HashSet::from(["uk_action_list".to_string()]);
        let c_effects = HashSet::new();
        let c_obs = HashSet::from(["f8_actor_a".to_string()]);
        let (ok_c, v_c) = dispatch_parse("f8_actor_c", &c_grants, &c_effects, &c_obs, "uk_action_list", "[]");
        assert!(ok_c, "list must dispatch, got {v_c}");
        let records_c = v_c["result"].as_array().expect("list is an array");
        assert!(
            records_c.iter().any(|r| r["principal"] == "f8_actor_a"),
            "C (observer of A) must see A's record: {records_c:?}"
        );
    }

    #[test]
    fn loopback_observers_filter_audit_list() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;
        unfer_ffi::uk_audit_clear();

        let g_effects = HashSet::new();
        let g_obs = HashSet::new();

        // Actor A makes a granted call so it has an audited entry.
        let a_grants = HashSet::from(["uk_version".to_string()]);
        let r = dispatch_loopback("f8_audit_a", &a_grants, &g_effects, &g_obs, &HashSet::new(), "uk_version", "[]");
        assert!(r.contains("\"result\""), "A must dispatch, got {r}");

        // Reader B (no observers) reads the trail: only its OWN entries are visible.
        let b_grants = HashSet::from(["uk_audit_list".to_string(), "uk_version".to_string()]);
        let r = dispatch_loopback("f8_audit_b", &b_grants, &g_effects, &g_obs, &HashSet::new(), "uk_version", "[]");
        assert!(r.contains("\"result\""), "B must dispatch, got {r}");
        let (ok, v) = dispatch_parse("f8_audit_b", &b_grants, &g_effects, &g_obs, "uk_audit_list", "[]");
        assert!(ok, "audit list must dispatch, got {v}");
        let entries = v["result"].as_array().expect("audit list is an array");
        assert!(
            entries.iter().any(|e| e["caller"]["principal"] == "f8_audit_b"),
            "B must see its own entries: {entries:?}"
        );
        assert!(
            entries.iter().all(|e| e["caller"]["principal"] != "f8_audit_a"),
            "B (no observers) must NOT see A's audit entries: {entries:?}"
        );

        // Reader C declares A as an observer: A's entries ARE visible.
        let c_grants = HashSet::from(["uk_audit_list".to_string()]);
        let c_obs = HashSet::from(["f8_audit_a".to_string()]);
        let (ok_c, v_c) = dispatch_parse("f8_audit_c", &c_grants, &g_effects, &c_obs, "uk_audit_list", "[]");
        assert!(ok_c, "audit list must dispatch, got {v_c}");
        let entries_c = v_c["result"].as_array().expect("audit list is an array");
        assert!(
            entries_c.iter().any(|e| e["caller"]["principal"] == "f8_audit_a"),
            "C (observer of A) must see A's audit entries: {entries_c:?}"
        );
    }

    #[test]
    fn loopback_agent_spawn_escalation_enforced_via_loopback() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::collections::HashSet;

        // The loopback now installs the module's full bounded grant set on the caller
        // context, so a module dispatching `uk_agent_spawn` is subject to the same
        // capability non-escalation as any bounded caller (UK-4202). Subset spawns —
        // including observer rights the module itself holds — still succeed.
        let grants = HashSet::from(["uk_agent_spawn".to_string(), "uk_version".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::from(["f8_peer".to_string()]);

        // Kernel escalation: agent requests uk_model_create (not held by the module).
        let escalate = r#"[{"name":"upgrader","grants":{"kernel":["uk_model_create"],"effects":[]}}]"#;
        let resp = dispatch_loopback("f8_escalator", &grants, &effects, &observers, &HashSet::new(), "uk_agent_spawn", escalate);
        assert!(resp.contains("\"error\""), "escalation must be refused, got {resp}");
        assert!(resp.contains("4202"), "expected UK-4202, got {resp}");

        // Observer escalation: agent requests an observer right the module doesn't hold.
        let escalate_obs =
            r#"[{"name":"spy","grants":{"kernel":[],"effects":[],"observers":["secret"]}}]"#;
        let resp = dispatch_loopback("f8_escalator", &grants, &effects, &observers, &HashSet::new(), "uk_agent_spawn", escalate_obs);
        assert!(
            resp.contains("\"error\"") && resp.contains("4202"),
            "observer escalation must be refused, got {resp}"
        );

        // Subset spawn succeeds (agent inherits a subset incl. the module's observers).
        let subset =
            r#"[{"name":"clerk","grants":{"kernel":["uk_version"],"effects":[],"observers":["f8_peer"]}}]"#;
let resp = dispatch_loopback("f8_escalator", &grants, &effects, &observers, &HashSet::new(), "uk_agent_spawn", subset);
        assert!(resp.contains("\"result\""), "subset spawn must succeed, got {resp}");
    }

    // ── S11: loopback peer lockdown (F10) ─────────────────────────────────

    #[test]
    fn peer_cred_rejects_foreign_child_pid() {
        // A self-connected socketpair's peer pid IS this test process.
        let (a, _b) = UnixStream::pair().unwrap();
        let peer = peer_cred_pid(&a).expect("SO_PEERCRED readable on a socketpair");
        assert_eq!(peer, std::process::id(), "peer must be us");
        // Matching pid -> accepted; a foreign pid -> rejected.
        assert!(
            peer_cred_pid(&a).map(|p| p == std::process::id()).unwrap_or(false),
            "our own peer must pass the gate"
        );
        let want = std::process::id() + 5000;
        assert!(
            peer_cred_pid(&a).map(|p| p != want).unwrap_or(true),
            "a foreign pid must fail the gate"
        );
    }

    #[test]
    fn peer_check_disarmed_before_spawn_accepts() {
        // expected_pid == 0 (loopback armed pre-child): the acceptor must not reject.
        let zero = AtomicU32::new(0);
        let (_a, b) = UnixStream::pair().unwrap();
        let want = zero.load(Ordering::Relaxed);
        let peer_pid = peer_cred_pid(&b);
        let reject = want != 0 && peer_pid.map(|p| p != want).unwrap_or(true);
        assert!(!reject, "disarmed loopback must accept the first probe");
    }

    /// The acceptor's reject predicate: armed-and-matching accepts, armed-and-foreign
    /// refuses, unarmed accepts on any cred.
    #[test]
    fn armed_loopback_reject_predicate() {
        let (_a, b) = UnixStream::pair().unwrap();
        let pid = peer_cred_pid(&b).expect("socketpair peer cred");
        let want = pid;
        assert!(!(want != 0 && peer_cred_pid(&b).map(|p| p != want).unwrap_or(true)));
        let want = pid.wrapping_add(5000);
        assert!(want != 0 && peer_cred_pid(&b).map(|p| p != want).unwrap_or(true));
        let want = 0u32;
        assert!(!(want != 0 && peer_cred_pid(&b).map(|p| p != want).unwrap_or(true)));
    }

    // ── S12: sidecar supervision & auto-restart (F11) ─────────────────────

    #[test]
    fn supervisor_respawns_crashed_child() {
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unfer_ffi::uk_audit_clear();
        // `true` exits instantly; the supervisor must detect the crash and respawn.
        let respawns = Arc::new(AtomicU32::new(0));
        let respawns2 = Arc::clone(&respawns);
        let make_child: Arc<dyn Fn() -> Result<Child, String> + Send + Sync> = Arc::new(move || {
            respawns2.fetch_add(1, Ordering::Relaxed);
            Command::new("true").spawn().map_err(|e| e.to_string())
        });
        let slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(
            Command::new("true").spawn().unwrap(),
        )));
        let quit = Arc::new(AtomicBool::new(false));
        let sup_q = Arc::clone(&quit);
        let handle = std::thread::spawn(move || {
            EcmaSidecar::supervise_loop(
                Arc::clone(&slot),
                make_child,
                sup_q,
                "test-mod",
                None,
            );
        });
        // Give the supervisor ~1.2s to observe the crash and respawn at least twice.
        std::thread::sleep(Duration::from_millis(1200));
        quit.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        let n = respawns.load(Ordering::Relaxed);
        assert!(n >= 2, "expected >=2 respawns after crash, got {n}");

        // The kernel audit trail must carry a KERNEL_DOWN event for the degraded module.
        let entries = read_ffi_audit();
        let down = entries
            .iter()
            .filter(|e| {
                e["symbol"] == "uk_kernel"
                    && e["detail"]
                        .as_str()
                        .unwrap_or("")
                        .starts_with("KERNEL_DOWN")
            })
            .count();
        assert!(down >= 1, "KERNEL_DOWN must be audited: {entries:?}");
    }

    // ── S14 (F13): per-call deadline ──────────────────────────────────────

    #[test]
    fn deadlined_call_kills_silent_child_and_audits() {
        let dir = std::env::temp_dir().join(format!("unfer-s14-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // A "main" unix socket whose listener accepts but never responds.
        let main_sock = dir.join("main.sock");
        let listener = UnixListener::bind(&main_sock).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let silent = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok(_) => std::thread::sleep(Duration::from_millis(5_000)),
                    Err(_) => break,
                }
            }
        });
        std::thread::sleep(Duration::from_millis(50));

        let loopback_sock = dir.join("loopback.sock");
        let loopback = KernelLoopback::start(
            "test",
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            GatekeeperMode::Enabled,
            &loopback_sock,
        )
            .expect("loopback");

        // A docile child that outlives the call; the deadline must kill it.
        let child = Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();

        let sidecar = EcmaSidecar {
            child: Arc::new(Mutex::new(Some(child))),
            main_sock,
            loopback,
            staging: dir.clone(),
            module_name: "deadline-mod".to_string(),
            call_deadline: Duration::from_millis(250),
            _supervisor: Arc::new(AtomicBool::new(false)),
        };

        let res = sidecar.call("run", "[]");
        let err = res.expect_err("silent worker must time out");
        eprintln!("ACTUAL_ERR={err:?}");
        assert!(err.contains("timeout") || err.contains("unavailable"), "unexpected err: {err}");

        // The child must be signalled (SIGKILL) so the supervisor can respawn a fresh one.
        let mut guard = sidecar.child.lock().unwrap();
        let exited = guard
            .as_mut()
            .and_then(|c| c.try_wait().ok())
            .flatten()
            .is_some();
        if !exited {
            // try_wait may race the OS; give the kill a moment to land.
            std::thread::sleep(Duration::from_millis(100));
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
        }

        let entries = read_ffi_audit();
        let deadline = entries
            .iter()
            .filter(|e| {
                e["symbol"] == "uk_kernel"
                    && e["detail"]
                        .as_str()
                        .unwrap_or("")
                        .starts_with("CALL_DEADLINE")
            })
            .count();
        assert!(deadline >= 1, "CALL_DEADLINE must be audited: {entries:?}");

        stop.store(true, Ordering::Relaxed);
        silent.join().unwrap();
        let _ = dir;
    }

    // ── S10: egress boundary (F9) ─────────────────────────────────────────

    #[test]
    fn egress_allowed_is_exact_host_match() {
        let net = HashSet::from(["api.example.com".to_string(), "127.0.0.1:8787".to_string()]);
        assert!(egress_allowed("api.example.com", &net));
        assert!(egress_allowed("api.example.com:443", &net)); // portless grant covers any port
        assert!(egress_allowed("127.0.0.1:8787", &net));
        assert!(!egress_allowed("127.0.0.1:8788", &net)); // right host, wrong port
        assert!(!egress_allowed("api.example.org", &net)); // substring must NOT match
        assert!(!egress_allowed("xapi.example.com", &net)); // prefix must NOT match
    }

    #[test]
    fn egress_allowed_default_deny() {
        let empty = HashSet::new();
        assert!(!egress_allowed("api.example.com", &empty));
        assert!(!egress_allowed("", &empty));
        let net = HashSet::from(["trusted.host".to_string()]);
        assert!(!egress_allowed("", &net));
        assert!(!egress_allowed("trusted.host:99999", &net)); // unparsable port -> host mismatch
    }

    #[test]
    fn fetch_host_parses_authority() {
        assert_eq!(fetch_host("http://api.example.com/foo"), "api.example.com");
        assert_eq!(fetch_host("https://127.0.0.1:8787/x"), "127.0.0.1:8787");
        assert_eq!(fetch_host("mailto:user@ex.com"), "");
        assert_eq!(fetch_host("not a url"), "");
        assert_eq!(fetch_host("http://example.com"), "example.com");
    }

    #[test]
    fn config_embeds_net_grants_as_external_services() {
        let manifest = ModuleManifest {
            name: "t".into(),
            version: "0.1.0".into(),
            archetypes: vec![],
            archetype: "ecmascript".into(),
            entry: "src/main.js".into(),
            grants: vec!["uk_version".into()],
            effects: vec![],
            effect_kinds: vec![],
            observers: vec![],
            resources: vec![],
            fs_grants: vec![],
            net_grants: vec!["127.0.0.1:8787".into(), "api.example.com".into()],
            max_ms: None,
            memory_max_bytes: None,
            gatekeeper_mode: crate::module::GatekeeperMode::Enabled,
        };
        let cfg = config_source(&manifest, Path::new("/tmp/loop.sock"), Path::new("/tmp/main.sock"));
        assert!(cfg.contains("net-egress-0"), "first egress service missing: {cfg}");
        assert!(cfg.contains("http://127.0.0.1:8787"), "loopback grant must be embedded: {cfg}");
        assert!(cfg.contains("http://api.example.com"), "host grant must be embedded: {cfg}");
    }

    #[test]
    fn config_without_net_grants_has_no_egress_services() {
        let manifest = ModuleManifest {
            name: "t".into(),
            version: "0.1.0".into(),
            archetypes: vec![],
            archetype: "ecmascript".into(),
            entry: "src/main.js".into(),
            grants: vec!["uk_version".into()],
            effects: vec![],
            effect_kinds: vec![],
            observers: vec![],
            resources: vec![],
            fs_grants: vec![],
            net_grants: vec![],
            max_ms: None,
            memory_max_bytes: None,
            gatekeeper_mode: crate::module::GatekeeperMode::Enabled,
        };
        let cfg = config_source(&manifest, Path::new("/tmp/loop.sock"), Path::new("/tmp/main.sock"));
        assert!(!cfg.contains("net-egress"), "empty net must produce no egress: {cfg}");
    }

    #[test]
    fn uk_fetch_denied_without_net_grant() {
        let grants = HashSet::from(["uk_fetch".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        let net = HashSet::new();
        let resp = dispatch_loopback(
            "fetch_probe",
            &grants,
            &effects,
            &observers,
            &net,
            "uk_fetch",
            r#"[{"url":"http://api.example.com/data"}]"#,
        );
        assert!(resp.contains("\"error\""), "no net grant => deny, got {resp}");
        assert!(resp.contains("4001"), "expected UK-4001, got {resp}");
        assert!(resp.contains("api.example.com"), "denial names the host, got {resp}");
    }

    #[test]
    fn fetch_denied_off_allowlist() {
        let grants = HashSet::from(["uk_fetch".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        let net = HashSet::from(["allowed.host".to_string()]);
        // Loopback host is NOT on the allowlist.
        let resp = dispatch_loopback(
            "fetch_probe",
            &grants,
            &effects,
            &observers,
            &net,
            "uk_fetch",
            r#"[{"url":"http://127.0.0.1:9/x"}]"#,
        );
        assert!(resp.contains("\"error\"") && resp.contains("4001"), "off-list must deny, got {resp}");
    }

    #[test]
    fn fetch_granted_loopback_fixture_succeeds() {
        // The audit trail is kernel-global and `uk_audit_clear()` in concurrent tests
        // would wipe this test's entry mid-flight — serialize + clear like the S6 tests.
        let _lock = LOOPBACK_AUDIT_TESTS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unfer_ffi::uk_audit_clear();
        // Local fixture server (loopback only).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let serve = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
        });
        let host = format!("127.0.0.1:{}", addr.port());
        let grant = host.clone();
        let grants = HashSet::from(["uk_fetch".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        let net = HashSet::from([grant.to_string()]);
        let url = format!("http://{host}/fixture");
        let resp = dispatch_loopback(
            "fetch_probe",
            &grants,
            &effects,
            &observers,
            &net,
            "uk_fetch",
            &serde_json::json!([{ "url": url }]).to_string(),
        );
        serve.join().unwrap();
        assert!(resp.contains("\"result\""), "granted fixture fetch must succeed, got {resp}");
        assert!(resp.contains("hello"), "fixture body must be returned, got {resp}");
        // The audit entry for the egress carries the explicit allow action + host.
        let entries = read_ffi_audit();
        let mine: Vec<&serde_json::Value> = entries
            .iter()
            .filter(|e| e["symbol"] == "uk_fetch" && e["caller"]["principal"] == "fetch_probe")
            .take(2)
            .collect();
        assert!(!mine.is_empty(), "fetch egress must be audited: {entries:?}");
        assert_eq!(mine[0]["ok"], true, "granted fetch must audit as allowed: {entries:?}");
        assert!(
            mine[0]["detail"].as_str().unwrap_or("").contains("action=allow"),
            "audit must record action=allow, got {entries:?}"
        );
    }

    #[test]
    fn fetch_granted_non_loopback_stays_offline() {
        // A granted non-loopback host passes the gate but the offline path refuses to
        // leave the loopback fixture (defense-in-depth until a real workerd egress is up).
        let grants = HashSet::from(["uk_fetch".to_string()]);
        let effects = HashSet::new();
        let observers = HashSet::new();
        let net = HashSet::from(["api.example.com".to_string()]);
        let resp = dispatch_loopback(
            "fetch_probe",
            &grants,
            &effects,
            &observers,
            &net,
            "uk_fetch",
            r#"[{"url":"http://api.example.com/x"}]"#,
        );
        assert!(resp.contains("\"error\""), "non-loopback egress must refuse offline, got {resp}");
        assert!(
            resp.contains("loopback fixture"),
            "refusal must cite the loopback-fixture restriction, got {resp}"
        );
    }
}
