//! cloud-hypervisor's external REST API over its Unix-domain API socket —
//! the endpoints documented in
//! `../cloud-hypervisor-build/cloud-hypervisor/docs/api.md`
//! (`PUT /api/v1/vm.create`, `PUT /api/v1/vm.boot`, `GET /api/v1/vm.info`,
//! `PUT /api/v1/vm.shutdown`). Hand-rolled HTTP/1.1 on
//! `std::os::unix::net::UnixStream`: four verbs do not justify an HTTP
//! crate, and the offline workspace forbids new dependencies
//! (PLAN_avm_pkg §4.6).
//!
//! Launch honesty (PLAN_avm_pkg §4.7 applied to VMs): [`find_vmm`] probes
//! for a cloud-hypervisor binary and [`refuse_reason`] explains precisely
//! what is missing — this crate never pretends a VM was created.

use crate::config::VmConfig;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// Socket connect/read failure (e.g. no VMM listening).
    Io(String),
    /// Non-2xx HTTP status from the API.
    Http { status: u16, body: String },
    /// Unparseable response.
    BadResponse(String),
    /// The cloud-hypervisor binary is not on this machine.
    NoVmm(String),
}

impl From<std::io::Error> for LaunchError {
    fn from(e: std::io::Error) -> Self {
        LaunchError::Io(e.to_string())
    }
}

pub struct Launcher {
    socket: PathBuf,
}

impl Launcher {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Launcher {
            socket: socket.into(),
        }
    }

    pub fn socket(&self) -> &std::path::Path {
        &self.socket
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
    ) -> Result<String, LaunchError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.socket)?;
        let body = body.unwrap_or_default();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n"
        );
        if !body.is_empty() {
            req.push_str("Content-Type: application/json\r\n");
            req.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        req.push_str("\r\n");
        req.push_str(&body);
        stream.write_all(req.as_bytes())?;
        stream.flush()?;
        // Backstop: never block forever on a wedged VMM.
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        // Read the headers, then exactly Content-Length body bytes. (A
        // blind `read_to_string` hangs against cloud-hypervisor's
        // keep-alive HTTP — found when the first real VMM binary became
        // available; the mock server always closed after one reply.)
        let mut head_bytes: Vec<u8> = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = stream.read(&mut byte)?;
            if n == 0 {
                break;
            }
            head_bytes.push(byte[0]);
            if head_bytes.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&head_bytes).to_string();
        let clen = head
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        let mut body = vec![0u8; clen];
        stream.read_exact(&mut body)?;
        // `head` already carries the terminating `\r\n\r\n`.
        let resp = format!("{head}{}", String::from_utf8_lossy(&body));
        parse_response(&resp)
    }

    /// `PUT /api/v1/vm.create` with the documented payload.
    pub fn create(&self, cfg: &VmConfig) -> Result<String, LaunchError> {
        self.request("PUT", "/api/v1/vm.create", Some(cfg.to_json()))
    }

    /// `PUT /api/v1/vm.boot`.
    pub fn boot(&self) -> Result<String, LaunchError> {
        self.request("PUT", "/api/v1/vm.boot", None)
    }

    /// `GET /api/v1/vm.info` — returns the info JSON.
    pub fn info(&self) -> Result<String, LaunchError> {
        self.request("GET", "/api/v1/vm.info", None)
    }

    /// `PUT /api/v1/vm.shutdown`.
    pub fn shutdown(&self) -> Result<String, LaunchError> {
        self.request("PUT", "/api/v1/vm.shutdown", None)
    }

    /// Poll `vm.info` until the API **answers** (the socket comes up after
    /// process spawn). Only I/O errors (no listener yet) are retried: a
    /// non-2xx *answer* — e.g. cloud-hypervisor's 500 `["Error from API",
    /// "VM is not created"]` before the first `vm.create` — proves the API
    /// is up and is returned immediately as `Err(Http)`; callers that just
    /// wait for readiness accept `Err(Http)` as "up".
    pub fn wait_ready(&self, attempts: u32) -> Result<String, LaunchError> {
        let mut last = LaunchError::BadResponse("wait_ready: no attempts".to_string());
        for i in 0..attempts.max(1) {
            match self.info() {
                Ok(body) => return Ok(body),
                Err(e @ LaunchError::Http { .. }) => return Err(e),
                Err(e) => last = e,
            }
            if i + 1 < attempts {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Err(last)
    }
}

fn parse_response(resp: &str) -> Result<String, LaunchError> {
    let (head, body) = resp.split_once("\r\n\r\n").ok_or_else(|| {
        LaunchError::BadResponse("response has no header/body separator".to_string())
    })?;
    let status_line = head.lines().next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let _version = parts.next().unwrap_or("");
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| LaunchError::BadResponse(format!("bad status line: {status_line}")))?;
    if (200..300).contains(&status) {
        Ok(body.to_string())
    } else {
        Err(LaunchError::Http {
            status,
            body: body.to_string(),
        })
    }
}

/// Locate a cloud-hypervisor binary: explicit env override first, then the
/// two trees this workspace family actually builds (the sibling
/// `cloud-hypervisor-build` tree regenerated by
/// `australVM/cloud_hypervisor_vm/setup.sh`, and an in-repo build).
pub fn find_vmm() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AVM_CLOUD_HYPERVISOR") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest.join(
            "../../../../cloud-hypervisor-build/cloud-hypervisor/target/release/cloud-hypervisor",
        ),
        manifest
            .join("../../../cloud_hypervisor_vm/cloud-hypervisor/target/release/cloud-hypervisor"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// The honest refusal: what is missing and how a capable host gets it.
pub fn refuse_reason() -> String {
    "refusing to launch: no cloud-hypervisor binary found \
     (set AVM_CLOUD_HYPERVISOR, or build the pinned tree with \
     `australVM/cloud_hypervisor_vm/setup.sh`, which fetches \
     cloud-hypervisor 9a24680a + the SpectrumOS patches; \
     see docs/spectrum_plan.md §6 for the hardware-gated rows)"
        .to_string()
}

/// Spawn the VMM with its API socket (`--api-socket`, cloud-hypervisor CLI).
pub fn spawn_vmm(
    bin: &std::path::Path,
    socket: &std::path::Path,
) -> Result<std::process::Child, LaunchError> {
    std::process::Command::new(bin)
        .arg("--api-socket")
        .arg(socket)
        .spawn()
        .map_err(LaunchError::from)
}

/// Spawn the VMM with its API socket and its stdout+stderr captured to
/// `log` (docs/spectrum_plan.md §5: the guest console *is* the in-guest
/// instrumentation for the VirtIO-ready row — kernel timestamps printed
/// on `console=ttyS0` land here, in the VMM's own capture file).
pub fn spawn_vmm_logged(
    bin: &std::path::Path,
    socket: &std::path::Path,
    log: &std::path::Path,
) -> Result<std::process::Child, LaunchError> {
    let out = std::fs::File::create(log)
        .map_err(|e| LaunchError::Io(format!("create {}: {e}", log.display())))?;
    let err = out
        .try_clone()
        .map_err(|e| LaunchError::Io(format!("clone {}: {e}", log.display())))?;
    std::process::Command::new(bin)
        .arg("--api-socket")
        .arg(socket)
        .stdout(std::process::Stdio::from(out))
        .stderr(std::process::Stdio::from(err))
        .spawn()
        .map_err(LaunchError::from)
}

/// A cloud-profile Hermit guest runner (docs/spectrum_plan.md §8) — the
/// two runners `../hermit-rs/hermit/README.md` "Running" names: the
/// specialized Uhyve unikernel hypervisor, or QEMU via the Hermit loader.
#[derive(Debug, PartialEq, Eq)]
pub enum HermitRunner {
    Uhyve(PathBuf),
    QemuLoader { qemu: PathBuf, loader: PathBuf },
}

/// Locate a Hermit runner: `uhyve` first (purpose-built), then QEMU plus
/// the Hermit loader (the CI download name is `hermit-loader-<arch>`,
/// `../hermit-rs/.github/workflows/ci.yml`). Both must be present for the
/// QEMU path — a loader without QEMU boots nothing. Env overrides
/// `AVM_UHYVE`, `AVM_QEMU`, `AVM_HERMIT_LOADER` win over PATH.
pub fn find_hermit_runner() -> Option<HermitRunner> {
    find_hermit_runner_with(|name| {
        let var = match name {
            "uhyve" => "AVM_UHYVE",
            "qemu-system-x86_64" => "AVM_QEMU",
            "hermit-loader-x86_64" => "AVM_HERMIT_LOADER",
            _ => "",
        };
        std::env::var_os(var)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| which(name))
    })
}

/// The probe seam: `probe(name)` returns the path of a usable binary, if
/// any. Injected so tests pin the preference order and the both-or-neither
/// rule without touching the machine.
pub fn find_hermit_runner_with(probe: impl Fn(&str) -> Option<PathBuf>) -> Option<HermitRunner> {
    if let Some(uhyve) = probe("uhyve") {
        return Some(HermitRunner::Uhyve(uhyve));
    }
    match (probe("qemu-system-x86_64"), probe("hermit-loader-x86_64")) {
        (Some(qemu), Some(loader)) => Some(HermitRunner::QemuLoader { qemu, loader }),
        _ => None,
    }
}

/// PATH lookup, std-only (offline discipline: no `which` crate).
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// The honest refusal for the cloud profile: what is missing and where a
/// capable host gets it (`hermit-os/uhyve`, `hermit-os/loader`).
pub fn hermit_refuse_reason() -> String {
    "refusing to launch: no Hermit runner found \
     (tried `uhyve`, then `qemu-system-x86_64` + `hermit-loader-x86_64`; \
     set AVM_UHYVE, or AVM_QEMU + AVM_HERMIT_LOADER, or install \
     https://github.com/hermit-os/uhyve and https://github.com/hermit-os/loader — \
     see docs/spectrum_plan.md §8 for the host-gated rows)"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// One-shot mock VMM API server: records the raw request, responds with
    /// `respond`, then closes (hence `Connection: close` on our side).
    fn mock_server(
        dir: &std::path::Path,
        respond: Vec<String>,
    ) -> (
        PathBuf,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        std::thread::JoinHandle<()>,
    ) {
        let sock = dir.join("vmm.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let seen2 = seen.clone();
        let handle = std::thread::spawn(move || {
            for reply in respond {
                let (mut stream, _) = listener.accept().unwrap();
                // Read headers, then Content-Length body.
                let mut req: Vec<u8> = Vec::new();
                let mut byte = [0u8; 1];
                loop {
                    if stream.read(&mut byte).unwrap() == 0 {
                        break;
                    }
                    req.push(byte[0]);
                    if req.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&req).to_string();
                let clen = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0u8; clen];
                stream.read_exact(&mut body).unwrap();
                let full = format!("{}\n{}", head, String::from_utf8_lossy(&body));
                seen2.lock().unwrap().push(full);
                stream.write_all(reply.as_bytes()).unwrap();
                stream.flush().unwrap();
                // Drop closes the socket: client's read_to_string ends.
            }
        });
        (sock, seen, handle)
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[test]
    fn create_boot_info_shutdown_speak_the_documented_api() {
        let dir = tempfile::tempdir().unwrap();
        let replies = vec![ok("{}"), ok("{}"), ok(r#"{"state":"Created"}"#), ok("{}")];
        let (sock, seen, handle) = mock_server(dir.path(), replies);

        let l = Launcher::new(&sock);
        let cfg = VmConfig::driver_domain("/run/avm/kernel", "console=hvc0");
        l.create(&cfg).unwrap();
        l.boot().unwrap();
        let info = l.info().unwrap();
        l.shutdown().unwrap();
        handle.join().unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert!(
            seen[0].starts_with("PUT /api/v1/vm.create HTTP/1.1\r\n"),
            "documented create verb"
        );
        assert!(seen[1].starts_with("PUT /api/v1/vm.boot HTTP/1.1\r\n"));
        assert!(seen[2].starts_with("GET /api/v1/vm.info HTTP/1.1\r\n"));
        assert!(seen[3].starts_with("PUT /api/v1/vm.shutdown HTTP/1.1\r\n"));
        // The create body is the documented payload.
        let body = seen[0].split_once("\r\n\r\n").unwrap().1;
        let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(v["payload"]["kernel"], "/run/avm/kernel");
        assert_eq!(info, r#"{"state":"Created"}"#);
    }

    #[test]
    fn non_2xx_is_an_honest_http_error() {
        let dir = tempfile::tempdir().unwrap();
        let (sock, _, handle) = mock_server(
            dir.path(),
            vec!["HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found".to_string()],
        );
        let err = Launcher::new(&sock).info().unwrap_err();
        handle.join().unwrap();
        assert_eq!(
            err,
            LaunchError::Http {
                status: 404,
                body: "not found".to_string()
            }
        );
    }

    #[test]
    fn no_server_means_io_error_not_fake_success() {
        let dir = tempfile::tempdir().unwrap();
        let err = Launcher::new(dir.path().join("nobody-home.sock"))
            .boot()
            .unwrap_err();
        assert!(
            matches!(err, LaunchError::Io(_)),
            "connection failure surfaces as Io: {err:?}"
        );
    }

    #[test]
    fn wait_ready_retries_until_the_api_answers() {
        let dir = tempfile::tempdir().unwrap();
        let (sock, _, handle) = mock_server(dir.path(), vec![ok(r#"{"ready":true}"#)]);
        let l = Launcher::new(&sock);
        let body = l.wait_ready(5).unwrap();
        assert!(body.contains("ready"));
        handle.join().unwrap();
    }

    #[test]
    fn spawn_vmm_logged_captures_the_child_console() {
        // The captured file is the in-guest instrumentation channel: any
        // child stdout/stderr (here: `/bin/sh` failing on the injected
        // `--api-socket` flag) must land in the file, not the test's stdio.
        let dir = std::env::temp_dir().join(format!("avm-vmm-logged-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let log = dir.join("console.log");
        // Real call shape against a real binary that writes on stderr.
        let sh = which("sh").expect("sh on PATH");
        let mut child =
            spawn_vmm_logged(&sh, &dir.join("sock"), &log).expect("spawn_vmm_logged spawns");
        let status = child.wait().expect("reap");
        assert!(!status.success(), "sh must fail on --api-socket");
        let text = std::fs::read_to_string(&log).expect("log file exists");
        assert!(
            !text.is_empty(),
            "child stderr must be captured into the log file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_vmm_only_returns_existing_files() {
        if let Some(p) = find_vmm() {
            assert!(p.is_file());
        }
        // In this offline workspace the pinned tree exists but was never
        // built: if a binary shows up, it must be the real file; if not,
        // refusal must name setup.sh.
        if find_vmm().is_none() {
            assert!(refuse_reason().contains("setup.sh"));
            assert!(refuse_reason().contains("AVM_CLOUD_HYPERVISOR"));
        }
    }

    #[test]
    fn wait_ready_counts_a_non_2xx_answer_as_api_up() {
        // cloud-hypervisor answers 500 "VM is not created" before the
        // first create — that is an answer; only I/O errors mean "not up
        // yet" and deserve retries.
        let dir = tempfile::tempdir().unwrap();
        let (sock, _, handle) = mock_server(
            dir.path(),
            vec!["HTTP/1.1 500 Internal Server Error\r\nContent-Length: 2\r\n\r\n{}".to_string()],
        );
        let err = Launcher::new(&sock).wait_ready(5).unwrap_err();
        handle.join().unwrap();
        assert!(
            matches!(err, LaunchError::Http { status: 500, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn keep_alive_server_is_read_by_content_length_not_read_to_end() {
        // A server that answers and *keeps the connection open* (as
        // cloud-hypervisor's HTTP does) must not stall the client.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("keep.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req: Vec<u8> = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                if s.read(&mut byte).unwrap() == 0 {
                    break;
                }
                req.push(byte[0]);
                if req.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let body = r#"{"state":"Running"}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            s.write_all(reply.as_bytes()).unwrap();
            s.flush().unwrap();
            // Deliberately keep the socket open (no drop/close) for a
            // while: the client must finish on Content-Length alone.
            std::thread::sleep(std::time::Duration::from_millis(200));
        });
        let info = Launcher::new(&sock).info().unwrap();
        assert_eq!(info, r#"{"state":"Running"}"#);
        handle.join().unwrap();
    }

    #[test]
    fn hermit_runner_prefers_uhyve_over_qemu() {
        let r = find_hermit_runner_with(|name| match name {
            "uhyve" => Some(PathBuf::from("/bin/uhyve")),
            "qemu-system-x86_64" => Some(PathBuf::from("/bin/qemu")),
            "hermit-loader-x86_64" => Some(PathBuf::from("/bin/loader")),
            _ => None,
        })
        .unwrap();
        assert_eq!(r, HermitRunner::Uhyve(PathBuf::from("/bin/uhyve")));
    }

    #[test]
    fn hermit_runner_falls_back_to_qemu_plus_loader() {
        let r = find_hermit_runner_with(|name| match name {
            "qemu-system-x86_64" => Some(PathBuf::from("/bin/qemu")),
            "hermit-loader-x86_64" => Some(PathBuf::from("/bin/loader")),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            r,
            HermitRunner::QemuLoader {
                qemu: PathBuf::from("/bin/qemu"),
                loader: PathBuf::from("/bin/loader"),
            }
        );
    }

    #[test]
    fn hermit_runner_needs_both_qemu_and_loader() {
        // QEMU alone boots no unikernel: the loader is mandatory.
        assert_eq!(
            find_hermit_runner_with(|name| match name {
                "qemu-system-x86_64" => Some(PathBuf::from("/bin/qemu")),
                _ => None,
            }),
            None
        );
        assert_eq!(
            find_hermit_runner_with(|name| match name {
                "hermit-loader-x86_64" => Some(PathBuf::from("/bin/loader")),
                _ => None,
            }),
            None
        );
    }

    #[test]
    fn hermit_refusal_names_everything_a_capable_host_needs() {
        assert_eq!(find_hermit_runner_with(|_| None), None);
        let r = hermit_refuse_reason();
        assert!(r.contains("uhyve"), "{r}");
        assert!(r.contains("hermit-os/loader"), "{r}");
        assert!(r.contains("AVM_UHYVE"), "{r}");
        assert!(r.contains("§8"), "{r}");
    }

    #[test]
    fn spawn_with_a_missing_binary_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let err =
            spawn_vmm(&dir.path().join("no-such-vm"), &dir.path().join("s.sock")).unwrap_err();
        assert!(matches!(err, LaunchError::Io(_)));
    }
}
