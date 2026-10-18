//! Task 7.3 evidence driver (docs/spectrum_plan.md §5/§6): the real
//! CAS → hydrate → configure → spawn → create → boot path against a real
//! cloud-hypervisor binary and a real guest kernel, with the task-8.3
//! boot-latency harness on top — plus the in-guest VirtIO-ready read-out:
//! the VMM's captured console carries the guest kernel's own timestamps
//! (`printk.time=1`), and the first `virtio` line is the guest-side ready
//! moment (§5). Not a unit test — this is the run the
//! plan records as host-gated.
//!
//! ```sh
//! cargo run --release --example boot_once -- /path/to/bzImage [workdir] [extra-cmdline]
//! ```
//!
//! The optional third argument appends to the guest cmdline; the §5
//! in-guest run uses `loglevel=8 printk.time=1` (kernel timestamps on
//! every line; `initcall_debug` is deliberately avoided — its serial
//! chatter starves the boot of the settle window).

use avm_cas::ChunkStore;
use avm_vmm::launcher::{find_vmm, refuse_reason, spawn_vmm_logged, Launcher};
use avm_vmm::lifecycle::{kvm_available, measure_boot_latency, plan_config, DomainSpec};

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("boot_once: {msg}");
    std::process::exit(1);
}

/// First guest-console line whose kernel timestamp and body mark a virtio
/// device moment: a driver line mentioning `virtio`, or the PCI bus
/// enumerating a `1af4` (Red Hat/virtio) device — the guest-side instant
/// the VirtIO device is visible. Guest-clock only (printk time since
/// kernel entry) — reported as such, never as a host-clock number.
fn first_virtio_line(log: &str) -> Option<(f64, &str)> {
    for line in log.lines() {
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((ts, tail)) = rest.split_once(']') else {
            continue;
        };
        let Ok(ts) = ts.trim().parse::<f64>() else {
            continue;
        };
        if tail.contains("virtio") || tail.contains("1af4") {
            return Some((ts, line));
        }
    }
    None
}

fn main() {
    let mut args = std::env::args().skip(1);
    let kernel_path = args
        .next()
        .unwrap_or_else(|| die("usage: boot_once <kernel-image> [workdir] [extra-cmdline]"));
    let workdir = args.next().unwrap_or_else(|| "/tmp/avm-boot".to_string());
    let extra_cmdline = args.next().unwrap_or_default();

    // 1. The kernel cell goes into the CAS as real bytes — the same
    //    `put_buffer` path the firmware ingestion of Sprint 7 uses.
    let bytes =
        std::fs::read(&kernel_path).unwrap_or_else(|e| die(format!("read {kernel_path}: {e}")));
    let store = ChunkStore::open(format!("{workdir}/store")).unwrap_or_else(|e| die(e.to_string()));
    store
        .put_buffer("linux-driver-kernel", "0.1.0", &bytes, None)
        .unwrap_or_else(|e| die(e.to_string()));

    let spec = DomainSpec {
        kernel_name: "linux-driver-kernel".to_string(),
        kernel_version: "0.1.0".to_string(),
        cmdline: format!("console=ttyS0 panic=-1 {extra_cmdline}"),
        workdir: format!("{workdir}/work").into(),
    };

    println!("[kvm] {:?}", kvm_available());
    let bin = match find_vmm() {
        Some(b) => b,
        None => die(refuse_reason()),
    };
    println!("[vmm] {}", bin.display());

    // 2. Hydrate the cell back out of the store and validate the config.
    let cfg = plan_config(&store, &spec).unwrap_or_else(|e| die(format!("{e:?}")));
    // The in-guest instrumentation sink: guest `ttyS0` (printk timestamps)
    // to a file — cloud-hypervisor's serial device defaults to Null.
    let serial_log = spec.workdir.join("serial.log");
    let cfg = cfg.with_serial_file(serial_log.display().to_string());

    // 3. Spawn the VMM with the guest console captured, wait for the API
    //    socket, create + boot. A socket path left behind by a killed VMM
    //    run makes cloud-hypervisor fail with AddrInUse — clear it first.
    let socket = spec.workdir.join("vmm-api.sock");
    let _ = std::fs::remove_file(&socket);
    let console_log = spec.workdir.join("console.log");
    let mut child = spawn_vmm_logged(&bin, &socket, &console_log)
        .unwrap_or_else(|e| die(format!("spawn: {e:?}")));
    let l = Launcher::new(&socket);
    // Readiness: the API answers 500 "VM is not created" before the first
    // create — an answer, so only I/O errors are fatal here.
    if let Err(e) = l.wait_ready(200) {
        if !matches!(e, avm_vmm::launcher::LaunchError::Http { .. }) {
            die(format!("wait_ready: {e:?}"));
        }
    }

    let t0 = std::time::Instant::now();
    l.create(&cfg)
        .unwrap_or_else(|e| die(format!("create: {e:?}")));
    let create_done = t0.elapsed();
    l.boot().unwrap_or_else(|e| die(format!("boot: {e:?}")));

    // 4. Task 8.3's harness: poll vm.info until the guest reports Running.
    let lat = measure_boot_latency(|| l.info(), 400, std::time::Duration::from_millis(5))
        .unwrap_or_else(|e| die(format!("latency: {e:?}")));
    println!(
        "[boot] running={} state={} polls={} vm_info_running_after={:?} since_create={:?}",
        lat.running,
        lat.state,
        lat.polls,
        lat.elapsed,
        create_done + lat.elapsed
    );
    println!("[info] {}", l.info().unwrap_or_default());

    // 4b. Let the guest reach the in-guest moment before pulling the plug:
    //     poll the serial sink for the first virtio line, bounded at 5 s
    //     (a guest without virtio drivers settles on the bound and the
    //     read-out below says so honestly).
    let settle_until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let seen = std::fs::read_to_string(&serial_log)
            .map(|s| first_virtio_line(&s).is_some())
            .unwrap_or(false);
        if seen || std::time::Instant::now() >= settle_until {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    l.shutdown()
        .unwrap_or_else(|e| die(format!("shutdown: {e:?}")));
    println!("[shutdown] ok");
    // Reap the VMM child so repeated runs can reuse the workdir/socket.
    let _ = child.kill();
    let _ = child.wait();

    // 5. In-guest read-out: the guest kernel's own timestamps on the
    //    captured serial console. The first `virtio` line is the
    //    guest-side VirtIO-ready moment (guest clock, i.e. since kernel
    //    entry).
    let log = std::fs::read_to_string(&serial_log)
        .unwrap_or_else(|e| die(format!("read {}: {e}", serial_log.display())));
    match first_virtio_line(&log) {
        Some((ts, line)) => println!(
            "[virtio] first in-guest virtio log line at guest-clock +{ts:.6} s: {}",
            line.trim()
        ),
        None => println!(
            "[virtio] no virtio line in {} bytes of guest console (no virtio driver in the guest?)",
            log.len()
        ),
    }
    println!("[console] {} bytes captured", log.len());
}
