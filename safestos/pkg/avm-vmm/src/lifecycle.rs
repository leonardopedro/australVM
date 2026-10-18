//! The CAS-backed driver-domain lifecycle: kernel cell manifest →
//! hydrated boot file → validated `VmConfig` → launch-or-refuse.
//!
//! Hydration uses the assembled read path (`ChunkStore::read_assembled`)
//! because cloud-hypervisor takes a *file path* for the payload; bridging
//! the hot store's zero-copy `acquire()` mapping straight into a VMM fd is
//! the recorded follow-up (docs/spectrum_plan.md §6, boot-latency row) —
//! the honest current shape, not a pretended zero-copy.
//!
//! Availability is **probed, never assumed**: [`kvm_available`] opens
//! `/dev/kvm` for real, exactly as `avm-exec/src/runner.rs` probes
//! `unshare` flags before claiming isolation.

use crate::config::{ConfigError, VmConfig};
use crate::launcher::{refuse_reason, LaunchError, Launcher};
use avm_cas::{ChunkStore, Manifest};
use std::path::{Path, PathBuf};

/// What a driver domain needs to be planned: which kernel cell to boot and
/// with what cmdline, plus where the hydrated boot file lands.
#[derive(Debug, Clone)]
pub struct DomainSpec {
    pub kernel_name: String,
    pub kernel_version: String,
    pub cmdline: String,
    pub workdir: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LifecycleError {
    /// No manifest in the store matches the requested kernel cell.
    KernelNotFound {
        name: String,
        version: String,
    },
    Cas(String),
    Io(String),
    /// `vm.info` answered something that is not the documented JSON.
    BadInfo(String),
    Config(ConfigError),
    Launch(LaunchError),
}

impl From<std::io::Error> for LifecycleError {
    fn from(e: std::io::Error) -> Self {
        LifecycleError::Io(e.to_string())
    }
}

impl From<ConfigError> for LifecycleError {
    fn from(e: ConfigError) -> Self {
        LifecycleError::Config(e)
    }
}

/// Find the kernel cell manifest by (name, version) in the store.
pub fn find_manifest(
    store: &ChunkStore,
    name: &str,
    version: &str,
) -> Result<Manifest, LifecycleError> {
    for id in store
        .list_manifests()
        .map_err(|e| LifecycleError::Cas(e.to_string()))?
    {
        let m = store
            .get_manifest(&id)
            .map_err(|e| LifecycleError::Cas(e.to_string()))?;
        if m.name == name && m.version == version {
            return Ok(m);
        }
    }
    Err(LifecycleError::KernelNotFound {
        name: name.to_string(),
        version: version.to_string(),
    })
}

/// Hydrate the kernel cell from the store into `spec.workdir/vmlinux` and
/// return the bootable path.
pub fn hydrate_kernel(store: &ChunkStore, spec: &DomainSpec) -> Result<PathBuf, LifecycleError> {
    let manifest = find_manifest(store, &spec.kernel_name, &spec.kernel_version)?;
    let bytes = store
        .read_assembled(&manifest)
        .map_err(|e| LifecycleError::Cas(e.to_string()))?;
    std::fs::create_dir_all(&spec.workdir)?;
    let path = spec.workdir.join("vmlinux");
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Assemble the cloud-hypervisor config for this driver domain.
pub fn plan_config(store: &ChunkStore, spec: &DomainSpec) -> Result<VmConfig, LifecycleError> {
    let kernel = hydrate_kernel(store, spec)?;
    let cfg = VmConfig::driver_domain(kernel.to_string_lossy(), spec.cmdline.clone());
    cfg.validate()?;
    Ok(cfg)
}

/// Probe `/dev/kvm` for real; the variant records *why* it is unavailable
/// so a report never claims more than the host grants.
#[derive(Debug, PartialEq, Eq)]
pub enum KvmProbe {
    Available,
    Absent(String),
    Denied(String),
}

pub fn kvm_available() -> KvmProbe {
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
    {
        Ok(_) => KvmProbe::Available,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => KvmProbe::Absent(e.to_string()),
        Err(e) => KvmProbe::Denied(e.to_string()),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct LaunchOutcome {
    /// True only when the VMM API actually acknowledged create+boot.
    pub created: bool,
    pub detail: String,
}

/// Launch-or-refuse: `None` (no VMM binary on this machine) yields the
/// honest refusal; `Some` runs create → boot against the API socket and
/// any failure propagates as an error — never a fake success.
pub fn launch_or_refuse(
    launcher: Option<&Launcher>,
    cfg: &VmConfig,
) -> Result<LaunchOutcome, LifecycleError> {
    match launcher {
        None => Ok(LaunchOutcome {
            created: false,
            detail: refuse_reason(),
        }),
        Some(l) => {
            l.create(cfg).map_err(LifecycleError::Launch)?;
            l.boot().map_err(LifecycleError::Launch)?;
            let info = l.info().map_err(LifecycleError::Launch)?;
            Ok(LaunchOutcome {
                created: true,
                detail: info,
            })
        }
    }
}

/// The full planned-but-honest path used by reports: probe KVM, find the
/// VMM, plan the config, then launch or refuse with the precise reason.
pub fn run(store: &ChunkStore, spec: &DomainSpec) -> Result<LaunchOutcome, LifecycleError> {
    let kvm = kvm_available();
    let cfg = plan_config(store, spec)?;
    let bin = crate::launcher::find_vmm();
    match (&bin, &kvm) {
        (None, _) => Ok(LaunchOutcome {
            created: false,
            detail: format!("{} [kvm: {:?}]", refuse_reason(), kvm),
        }),
        (Some(_), KvmProbe::Absent(e)) => Ok(LaunchOutcome {
            created: false,
            detail: format!("refusing to launch: /dev/kvm absent ({e})"),
        }),
        (Some(_), KvmProbe::Denied(e)) => Ok(LaunchOutcome {
            created: false,
            detail: format!("refusing to launch: /dev/kvm denied ({e})"),
        }),
        (Some(bin), KvmProbe::Available) => {
            // Task 7.3's orchestration: spawn the VMM child, wait for its
            // API socket, then create → boot against that socket. (The
            // earlier shape mapped the *binary path* into `Launcher::new`
            // and never spawned anything — a dead socket at best; fixed
            // when the first real VMM binary became available.)
            let socket = spec.workdir.join("vmm-api.sock");
            let _child =
                crate::launcher::spawn_vmm(bin, &socket).map_err(LifecycleError::Launch)?;
            let l = Launcher::new(&socket);
            // Readiness: a non-2xx answer (cloud-hypervisor's 500 "VM is
            // not created" pre-create) still proves the API is up.
            if let Err(e) = l.wait_ready(100) {
                if !matches!(e, LaunchError::Http { .. }) {
                    return Err(LifecycleError::Launch(e));
                }
            }
            launch_or_refuse(Some(&l), &cfg)
        }
    }
}

/// Where the kernel cell should live for the stock bootstrap plan. The
/// (name, version) pair is what `stdlib/pkg/LinuxDriverKernel.aui` exports
/// into plan.json — keep the two in lockstep.
pub fn bootstrap_kernel_spec(workdir: impl Into<PathBuf>) -> DomainSpec {
    DomainSpec {
        kernel_name: "linux-driver-kernel".to_string(),
        kernel_version: "0.1.0".to_string(),
        cmdline: "console=hvc0".to_string(),
        workdir: workdir.into(),
    }
}

/// Boot-latency measurement (docs/spectrum_plan.md task 8.3): poll
/// `vm.info` until the guest reports `Running`, recording how many polls
/// and how much wall time it took. The real < 50 ms number is host-gated
/// (§6); this harness is what a capable host runs, and offline it is
/// exercised against the mock API server. Never a fake success: if the
/// guest never reports running, `running` is false and the caller says so.
#[derive(Debug, PartialEq, Eq)]
pub struct BootLatency {
    pub polls: u32,
    pub elapsed: std::time::Duration,
    /// Last observed `state` string from `vm.info`.
    pub state: String,
    /// True only if the guest actually reported `Running`.
    pub running: bool,
}

pub fn measure_boot_latency(
    mut poll: impl FnMut() -> Result<String, LaunchError>,
    max_polls: u32,
    interval: std::time::Duration,
) -> Result<BootLatency, LifecycleError> {
    let start = std::time::Instant::now();
    let mut state = String::new();
    let attempts = max_polls.max(1);
    for i in 0..attempts {
        let body = poll().map_err(LifecycleError::Launch)?;
        let v: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| LifecycleError::BadInfo(format!("vm.info is not JSON: {e}")))?;
        state = v
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("<no state>")
            .to_string();
        if state == "Running" {
            return Ok(BootLatency {
                polls: i + 1,
                elapsed: start.elapsed(),
                state,
                running: true,
            });
        }
        if i + 1 < attempts {
            std::thread::sleep(interval);
        }
    }
    Ok(BootLatency {
        polls: attempts,
        elapsed: start.elapsed(),
        state,
        running: false,
    })
}

/// Convenience: is this path a bootable ELF the VMM could take?
pub fn looks_like_elf(path: &Path) -> bool {
    std::fs::read(path)
        .map(|b| b.len() >= 4 && b[0..4] == [0x7f, b'E', b'L', b'F'])
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avm_cas::ChunkStore;

    fn store_with_kernel(dir: &Path) -> ChunkStore {
        let store = ChunkStore::open(dir.join("store")).unwrap();
        store
            .put_buffer(
                "linux-driver-kernel",
                "0.1.0",
                b"\x7fELF-fake-driver-kernel-bytes",
                None,
            )
            .unwrap();
        store
    }

    #[test]
    fn kernel_resolves_hydrates_and_plans() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_kernel(tmp.path());
        let spec = bootstrap_kernel_spec(tmp.path().join("work"));
        let cfg = plan_config(&store, &spec).unwrap();
        cfg.validate().unwrap();
        // The hydrated file is byte-identical to what went in, and it is a
        // plausible payload (ELF magic) — no placeholder paths.
        let kernel = PathBuf::from(cfg.payload.kernel.clone());
        assert!(kernel.is_file());
        assert!(looks_like_elf(&kernel));
        assert_eq!(
            std::fs::read(&kernel).unwrap(),
            b"\x7fELF-fake-driver-kernel-bytes"
        );
        assert_eq!(cfg.payload.cmdline, "console=hvc0");
    }

    #[test]
    fn missing_kernel_is_an_honest_error() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ChunkStore::open(tmp.path().join("store")).unwrap();
        let spec = bootstrap_kernel_spec(tmp.path().join("work"));
        assert_eq!(
            plan_config(&store, &spec),
            Err(LifecycleError::KernelNotFound {
                name: "linux-driver-kernel".to_string(),
                version: "0.1.0".to_string(),
            })
        );
    }

    #[test]
    fn kvm_probe_runs_and_reports_consistently() {
        match kvm_available() {
            KvmProbe::Available => {
                // Claims availability ⇒ the device really opens rw.
                assert!(std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open("/dev/kvm")
                    .is_ok());
            }
            KvmProbe::Absent(e) | KvmProbe::Denied(e) => assert!(!e.is_empty()),
        }
    }

    #[test]
    fn boot_latency_measures_polls_and_time_against_the_mock_server() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;

        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("lat.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let replies = [
            "{\"state\":\"Created\"}".to_string(),
            "{\"state\":\"Running\"}".to_string(),
        ];
        let handle = std::thread::spawn(move || {
            for body in replies {
                let (mut s, _) = listener.accept().unwrap();
                // Drain the request headers (GET sends no body).
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
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                s.write_all(reply.as_bytes()).unwrap();
                s.flush().unwrap();
            }
        });

        let l = Launcher::new(&sock);
        let lat =
            measure_boot_latency(|| l.info(), 5, std::time::Duration::from_millis(1)).unwrap();
        handle.join().unwrap();
        assert!(lat.running, "mock guest reaches Running");
        assert_eq!(lat.polls, 2);
        assert_eq!(lat.state, "Running");
        assert!(
            lat.elapsed >= std::time::Duration::from_millis(1),
            "one poll interval elapsed: {:?}",
            lat.elapsed
        );
    }

    #[test]
    fn boot_latency_never_fakes_a_running_guest() {
        let mut n = 0u32;
        let lat = measure_boot_latency(
            || {
                n += 1;
                Ok(r#"{"state":"Created"}"#.to_string())
            },
            3,
            std::time::Duration::from_millis(0),
        )
        .unwrap();
        assert!(!lat.running);
        assert_eq!(lat.polls, 3);
        assert_eq!(lat.state, "Created");
    }

    #[test]
    fn boot_latency_propagates_api_errors_not_fake_success() {
        let err = measure_boot_latency(
            || Err(LaunchError::Io("socket dead".to_string())),
            3,
            std::time::Duration::from_millis(0),
        )
        .unwrap_err();
        assert!(matches!(err, LifecycleError::Launch(LaunchError::Io(_))));

        let err = measure_boot_latency(
            || Ok("not json".to_string()),
            3,
            std::time::Duration::from_millis(0),
        )
        .unwrap_err();
        assert!(matches!(err, LifecycleError::BadInfo(_)));
    }

    #[test]
    fn without_a_vmm_binary_the_launch_refuses_with_the_setup_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_kernel(tmp.path());
        let spec = bootstrap_kernel_spec(tmp.path().join("work"));
        // Force the no-VMM path regardless of what a future host has.
        let out = launch_or_refuse(None, &plan_config(&store, &spec).unwrap()).unwrap();
        assert!(!out.created, "refusal must not claim creation");
        assert!(
            out.detail.contains("setup.sh"),
            "refusal points at the pinned build: {}",
            out.detail
        );
    }

    #[test]
    fn with_a_vmm_config_but_no_socket_the_failure_is_an_error_not_a_success() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = VmConfig::driver_domain("/nope", "console=hvc0");
        let l = Launcher::new(tmp.path().join("no-socket.sock"));
        let err = launch_or_refuse(Some(&l), &cfg).unwrap_err();
        assert!(
            matches!(err, LifecycleError::Launch(LaunchError::Io(_))),
            "{err:?}"
        );
    }
}
