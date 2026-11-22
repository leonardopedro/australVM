//! G23b: two distinct physical adapters, one process, independent dispatches.
//!
//! The card's questions are (a) which two cards are actually distinct, given
//! that one card can be enumerated once per backend, (b) whether each device
//! produces the right answer, and (c) whether the two are demonstrably busy at
//! the same time. (a) is decided from the adapter facts wgpu reports, (b) by
//! comparing each device's words with the KernelIR reference, and (c) by host
//! timestamps around each submit/completion pair plus a serial phase to
//! compare against — never by asserting that "two threads ran".
//!
//! Placement goes through the G23a coordinator, so the hardware lane exercises
//! the same one-worker-per-device admission path the mock tests pin.

#![cfg(all(feature = "device", feature = "service"))]

use avm_gpu::coordinator::{Budget, Coordinator, Identity};
use avm_gpu::device::{self, AdapterEvidence};
use avm_gpu::run::RunFailure;
use avm_gpu::schema::*;
use avm_gpu::service::{COMPLETE, PENDING};
use avm_gpu::{reference, validate};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ITERATIONS: u32 = 1500;

const ADD_WGSL: &str = r#"
@group(0) @binding(0) var<storage, read> input0: array<u32>;
@group(0) @binding(1) var<storage, read> input1: array<u32>;
@group(0) @binding(2) var<storage, read_write> output0: array<u32>;

fn scalar_entry(p0: u32, p1: u32) -> u32 {
    return p0 + p1;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&input0)) { return; }
    output0[i] = scalar_entry(input0[i], input1[i]);
}
"#;

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_bundle(tag: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!(
        "avm-gpu-dual-{}-{}-{}",
        std::process::id(),
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let bindings = (0..3)
        .map(|i| {
            serde_json::json!({
                "group": 0,
                "binding": i,
                "role": if i < 2 { "input" } else { "output" },
                "element": "u32",
                "stride_bytes": 4,
            })
        })
        .collect::<Vec<_>>();
    let mut bundle = KernelBundle {
        schema: SCHEMA.to_string(),
        backend: BACKEND.to_string(),
        numeric_profile: NUMERIC_PROFILE.to_string(),
        entry: "Add:k".to_string(),
        input_arity: 2,
        compiler: "g23b-dual-test".to_string(),
        sources: vec![Source {
            path: "Add.aum".to_string(),
            sha256: sha256_hex(b"module body Add\n"),
        }],
        kernel_ir: KernelIr {
            entry: "Add:k".to_string(),
            input_arity: 2,
            helpers: Vec::new(),
            body: Stmt::Return {
                value: Expr::Intrinsic {
                    op: "add".to_string(),
                    args: vec![Expr::Param { index: 0 }, Expr::Param { index: 1 }],
                },
            },
            memory: MemoryPlan {
                map: MapKind::Map2,
                output_index: "identity".to_string(),
                guard_before_load: true,
                workgroup_size: WORKGROUP_SIZE,
            },
            numeric_profile: NUMERIC_PROFILE.to_string(),
        },
        ir_sha256: String::new(),
        wgsl_sha256: String::new(),
        requirements: serde_json::json!({
            "bindings": bindings,
            "numeric": {
                "profile": NUMERIC_PROFILE,
                "element": "u32",
                "operations": ["add", "subtract", "multiply", "bitwiseAnd", "bitwiseOr", "bitwiseXor", "bitwiseNot"],
                "rounding": "modulo-2^32",
                "contraction": "not-applicable",
                "domain": "all-2^32",
                "subnormals": "not-applicable",
                "overflow": "defined-wraparound",
                "tolerance_ulp_per_operation": 0
            }
        }),
        evidence: Evidence {
            source: "test".to_string(),
            source_profile: None,
            shader_validation: "naga".to_string(),
            device_kind: "none".to_string(),
            execution: "reference".to_string(),
            proof: "none".to_string(),
        },
        proof_manifest: None,
    };
    bundle.ir_sha256 = sha256_hex(&bundle.kernel_ir.canonical_bytes().unwrap());
    bundle.wgsl_sha256 = sha256_hex(ADD_WGSL.as_bytes());
    std::fs::write(
        dir.join("kernel.json"),
        serde_json::to_vec_pretty(&bundle).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("kernel.wgsl"), ADD_WGSL).unwrap();
    Scratch(dir)
}

/// One card, independent of which backend enumerated it: a card that Vulkan and
/// GL both expose is one card, and selecting "one per backend" would run both
/// jobs on the same GPU. PCI vendor/device is the identity when the backend
/// reports it; the two views of one NVIDIA card spell its name differently
/// (`… with Max-Q Design` vs `… with Max-Q Design/PCIe/SSE2`), so a name key
/// alone would count one card twice.
fn card_key(e: &AdapterEvidence) -> String {
    if e.vendor != 0 && e.device != 0 {
        format!("pci:{:x}:{:x}", e.vendor, e.device)
    } else {
        format!("name:{}", e.name.to_ascii_lowercase())
    }
}

/// The first physical view of each distinct card, in enumeration order. A
/// non-physical view is never chosen while a physical one exists, and Vulkan is
/// preferred because it is the backend the runner's limits are measured on.
fn distinct_cards(all: &[AdapterEvidence]) -> Vec<(usize, AdapterEvidence)> {
    let mut chosen: Vec<(usize, AdapterEvidence)> = Vec::new();
    for (index, evidence) in all.iter().enumerate() {
        let key = card_key(evidence);
        match chosen.iter_mut().find(|(_, e)| card_key(e) == key) {
            Some((chosen_index, current)) => {
                let better = (evidence.physical && !current.physical)
                    || (evidence.physical == current.physical
                        && evidence.backend == "Vulkan"
                        && current.backend != "Vulkan");
                if better {
                    *chosen_index = index;
                    *current = evidence.clone();
                }
            }
            None => chosen.push((index, evidence.clone())),
        }
    }
    chosen
}

fn describe(all: &[AdapterEvidence]) {
    for (i, e) in all.iter().enumerate() {
        println!(
            "[detail] G23b adapter {i}: {} ({}, {}) vendor={:#x} device={:#x} physical={} key={}",
            e.name,
            e.backend,
            e.device_type,
            e.vendor,
            e.device,
            e.physical,
            card_key(e)
        );
    }
}

/// Where a job was on the host clock: the instant it was admitted and the
/// instant its terminal state was read. Two jobs whose intervals overlap were
/// in flight at the same time; that is correlation, and the serial phase is
/// what turns it into a comparison.
#[derive(Debug)]
struct Window {
    start: Instant,
    end: Instant,
}

impl Window {
    fn overlaps(&self, other: &Window) -> bool {
        self.start < other.end && other.start < self.end
    }
    fn ms(&self) -> f64 {
        self.end.duration_since(self.start).as_secs_f64() * 1000.0
    }
}

struct JobResult {
    output: Vec<u32>,
    window: Window,
    phase: Phase,
}

#[derive(Debug)]
struct Phase {
    prepare_ms: f64,
    upload_ms: f64,
    submit_ms: f64,
    readback_ms: f64,
    adapter: String,
    backend: String,
}

fn wait_for(
    coordinator: &Arc<Coordinator>,
    context: i64,
    job: i64,
    started: Instant,
) -> Window {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let state = coordinator.poll(context, job);
        if state != PENDING {
            assert_eq!(state, COMPLETE, "job {job} ended with {state}");
            return Window {
                start: started,
                end: Instant::now(),
            };
        }
        assert!(
            Instant::now() < deadline,
            "job {job} never reached a terminal state"
        );
        std::thread::sleep(Duration::from_micros(200));
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_on(
    coordinator: &Arc<Coordinator>,
    device: usize,
    context: i64,
    generation: i64,
    selection: device::Selection,
    loaded: &Arc<avm_gpu::LoadedBundle>,
    inputs: Vec<Vec<u32>>,
    bytes: usize,
) -> (i64, std::sync::mpsc::Receiver<(Vec<u32>, Phase)>) {
    let (sender, receiver) = channel::<(Vec<u32>, Phase)>();
    let loaded = Arc::clone(loaded);
    let job = coordinator.submit(context, generation, bytes, move || {
        match avm_gpu::run::run_with_iterations(&loaded, &inputs, true, &selection, ITERATIONS) {
            Ok(result) => {
                let phase = Phase {
                    prepare_ms: result.prepare_ms,
                    upload_ms: result.upload_ms,
                    submit_ms: result.submit_ms,
                    readback_ms: result.readback_ms,
                    adapter: result.adapter.name.clone(),
                    backend: result.adapter.backend.clone(),
                };
                let _ = sender.send((result.output, phase));
                avm_gpu::coordinator::Outcome::Complete
            }
            Err(RunFailure::Unavailable(message)) => {
                eprintln!("[gap] G23b device {device} unavailable: {message}");
                avm_gpu::coordinator::Outcome::Failed(77)
            }
            Err(RunFailure::Refused(message)) => {
                eprintln!("[fail] G23b device {device} refused the bundle: {message}");
                avm_gpu::coordinator::Outcome::Failed(1)
            }
        }
    });
    (job, receiver)
}

fn wait_receipt(
    coordinator: &Arc<Coordinator>,
    context: i64,
    job: i64,
    receiver: std::sync::mpsc::Receiver<(Vec<u32>, Phase)>,
    started: Instant,
) -> JobResult {
    let window = wait_for(coordinator, context, job, started);
    assert_eq!(coordinator.result(context, job), 0);
    let (output, phase) = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a completed job published its words");
    JobResult {
        output,
        window,
        phase,
    }
}

#[test]
fn two_distinct_physical_adapters_run_independent_dispatches() {
    let all = device::enumerate();
    describe(&all);
    let cards = distinct_cards(&all);
    println!(
        "[detail] G23b enumeration: {} adapter view(s), {} distinct card(s)",
        all.len(),
        cards.len()
    );
    for (index, card) in &cards {
        let views = all
            .iter()
            .enumerate()
            .filter(|(_, e)| card_key(e) == card_key(card) && e.backend != card.backend)
            .map(|(i, e)| format!("{i}:{}", e.backend))
            .collect::<Vec<_>>();
        if !views.is_empty() {
            println!(
                "[detail] G23b card {} selected at {index} ({}); duplicate backend view(s) {} not selected",
                card.name,
                card.backend,
                views.join(", ")
            );
        }
    }

    let physical = cards
        .iter()
        .filter(|(_, e)| e.physical)
        .collect::<Vec<_>>();
    if physical.len() < 2 {
        let detail = format!(
            "{} distinct physical adapter(s) visible; two-device execution needs two",
            physical.len()
        );
        if std::env::var_os("AVM_GPU_ALLOW_ONE_DEVICE").is_some() {
            eprintln!("[gap] G23b two-device run not performed: {detail}");
            return;
        }
        panic!("{detail} (set AVM_GPU_ALLOW_ONE_DEVICE=1 to record an explicit gap)");
    }
    let (first_index, first_card) = (physical[0].0, physical[0].1.clone());
    let (second_index, second_card) = (physical[1].0, physical[1].1.clone());
    assert_ne!(
        card_key(&first_card),
        card_key(&second_card),
        "the two selections must be different cards"
    );

    let scratch = write_bundle("dual");
    let loaded = Arc::new(
        avm_gpu::load_bundle(&scratch.0).expect("bundle loads"),
    );
    validate::validate_bundle(&loaded).expect("bundle validates");

    // Different tagged inputs, so a mix-up between the two devices shows up as
    // a wrong vector rather than as two matching outputs.
    let inputs_a = vec![vec![1, 2, 3, 4], vec![10, 20, 30, 40]];
    let inputs_b = vec![vec![7, 8, 9, 10], vec![1, 1, 1, 1]];
    let expected_a = reference::Evaluator::new(
        &loaded.bundle.kernel_ir,
        reference::Limits::default(),
    )
    .unwrap()
    .run(&inputs_a)
    .unwrap();
    let expected_b = reference::Evaluator::new(
        &loaded.bundle.kernel_ir,
        reference::Limits::default(),
    )
    .unwrap()
    .run(&inputs_b)
    .unwrap();
    assert_eq!(expected_a, vec![11, 22, 33, 44]);
    assert_eq!(expected_b, vec![8, 9, 10, 11]);

    let coordinator = Coordinator::new();
    let mut devices = Vec::new();
    for card in [&first_card, &second_card] {
        let identity = Identity {
            backend: card.backend.clone(),
            name: card.name.clone(),
            device_type: card.device_type.clone(),
        };
        assert!(
            identity.is_physical(),
            "{} was classified {} and must not back a hardware run",
            card.name,
            card.device_type
        );
        devices.push(coordinator.add_device(
            identity,
            Budget {
                max_in_flight: 2,
                max_bytes: 1 << 20,
            },
        ));
    }
    let first = devices[0];
    let second = devices[1];
    let first_context = 0xA0;
    let second_context = 0xB0;
    let first_generation = coordinator.open_context(first_context, first);
    let second_generation = coordinator.open_context(second_context, second);

    let bytes = 4 * inputs_a.iter().map(Vec::len).sum::<usize>();
    let selections = [
        device::Selection {
            name: None,
            index: Some(first_index),
        },
        device::Selection {
            name: None,
            index: Some(second_index),
        },
    ];

    // Serial phase: the same work with one device busy at a time. This is the
    // baseline the concurrent phase is compared against.
    let mut serial = Vec::new();
    for (i, (context, generation, selection, inputs)) in [
        (first_context, first_generation, selections[0].clone(), inputs_a.clone()),
        (second_context, second_generation, selections[1].clone(), inputs_b.clone()),
    ]
    .into_iter()
    .enumerate()
    {
        let started = Instant::now();
        let (job, receiver) = submit_on(
            &coordinator,
            i,
            context,
            generation,
            selection,
            &loaded,
            inputs,
            bytes,
        );
        serial.push(wait_receipt(&coordinator, context, job, receiver, started));
    }
    let serial_total = serial[0].window.ms() + serial[1].window.ms();

    // Concurrent phase: both submitted before either is awaited.
    let wall_start = Instant::now();
    let first_started = Instant::now();
    let (job_a, rx_a) = submit_on(
        &coordinator,
        0,
        first_context,
        first_generation,
        selections[0].clone(),
        &loaded,
        inputs_a.clone(),
        bytes,
    );
    let second_started = Instant::now();
    let (job_b, rx_b) = submit_on(
        &coordinator,
        1,
        second_context,
        second_generation,
        selections[1].clone(),
        &loaded,
        inputs_b.clone(),
        bytes,
    );
    // Await in submission order; the windows, not the await order, decide
    // whether they overlapped.
    let result_a = wait_receipt(&coordinator, first_context, job_a, rx_a, first_started);
    let result_b = wait_receipt(&coordinator, second_context, job_b, rx_b, second_started);
    let concurrent_wall = wall_start.elapsed().as_secs_f64() * 1000.0;

    assert_eq!(result_a.output, expected_a, "device 0 ({}) output", result_a.phase.adapter);
    assert_eq!(result_b.output, expected_b, "device 1 ({}) output", result_b.phase.adapter);
    assert!(
        !result_a.phase.adapter.is_empty() && !result_b.phase.adapter.is_empty(),
        "both devices must report their adapter"
    );

    println!(
        "[pass] G23b two devices: {} ({}), {} ({}), {} iterations each, outputs match the reference",
        result_a.phase.adapter,
        result_a.phase.backend,
        result_b.phase.adapter,
        result_b.phase.backend,
        ITERATIONS
    );
    for (i, r) in [(0, &result_a), (1, &result_b)] {
        println!(
            "[detail] G23b device {i} phases: prepare {:.3} ms, upload {:.3} ms, submit {:.3} ms, readback {:.3} ms",
            r.phase.prepare_ms, r.phase.upload_ms, r.phase.submit_ms, r.phase.readback_ms
        );
    }
    // Offsets from the concurrent phase's start, so the two windows can be
    // compared on one axis instead of by duration alone.
    let offset = |t: Instant| t.duration_since(wall_start).as_secs_f64() * 1000.0;
    println!(
        "[detail] G23b windows (ms from concurrent start): 0 +{:.3} for {:.3}, 1 +{:.3} for {:.3} (overlap={})",
        offset(result_a.window.start),
        result_a.window.ms(),
        offset(result_b.window.start),
        result_b.window.ms(),
        result_a.window.overlaps(&result_b.window)
    );
    println!(
        "[detail] G23b wall: serial {:.3} ms, concurrent {:.3} ms, ratio {:.2}",
        serial_total,
        concurrent_wall,
        concurrent_wall / serial_total
    );

    assert!(
        result_a.window.overlaps(&result_b.window),
        "the two jobs were never in flight together: A {:.1} ms, B {:.1} ms, concurrent wall {:.1} ms",
        result_a.window.ms(),
        result_b.window.ms(),
        concurrent_wall
    );
    if serial_total > 30.0 {
        assert!(
            concurrent_wall < serial_total,
            "concurrent wall {concurrent_wall:.1} ms is not below the serial baseline {serial_total:.1} ms"
        );
        println!(
            "[pass] G23b overlap: concurrent wall {:.1} ms < serial {:.1} ms ({:.2}x)",
            concurrent_wall,
            serial_total,
            serial_total / concurrent_wall
        );
    } else {
        // Too short to compare: report the measurement, never a pass.
        println!(
            "[gap] G23b overlap comparison skipped: serial baseline {serial_total:.1} ms is under the 30 ms measurable floor"
        );
    }

    assert_eq!(coordinator.teardown(first), 0);
    assert_eq!(coordinator.teardown(second), 0);
    assert_eq!(coordinator.reserved(first), Some((0, 0)));
    assert_eq!(coordinator.reserved(second), Some((0, 0)));
}

/// The card, exercised as far as the hardware contract allows: one card seen
/// through two backends must collapse to one selection, or "two adapters" would
/// silently mean "one GPU twice".
#[test]
fn duplicate_backend_views_collapse_to_one_card() {
    fn evidence(name: &str, backend: &str, device: u32, physical: bool) -> AdapterEvidence {
        AdapterEvidence {
            backend: backend.to_string(),
            name: name.to_string(),
            vendor: 0x8086,
            device,
            device_type: if physical {
                "integrated-gpu".to_string()
            } else {
                "cpu".to_string()
            },
            physical,
            max_workgroup_size_x: 64,
            max_buffer_size: 1 << 20,
            max_storage_buffer_binding_size: 1 << 20,
            max_compute_workgroups_per_dimension: 1024,
        }
    }
    let all = vec![
        evidence("Intel", "GL", 0x3E9B, true),
        // One card, a different name per backend — the case this host shows.
        evidence("NVIDIA GeForce GTX 1060", "Vulkan", 0x1C03, true),
        evidence("NVIDIA GeForce GTX 1060/PCIe/SSE2", "Gl", 0x1C03, true),
        evidence("llvmpipe", "Vulkan", 0xFFFF, false),
    ];
    let cards = distinct_cards(&all);
    assert_eq!(cards.len(), 3, "the two NVIDIA views are one card; llvmpipe is its own");
    let intel = cards.iter().find(|(_, e)| e.name == "Intel").unwrap();
    assert_eq!(intel.0, 0);
    let nvidia = cards.iter().find(|(_, e)| e.name.starts_with("NVIDIA")).unwrap();
    assert_eq!(nvidia.0, 1, "the Vulkan view of NVIDIA is preferred");
    // A software adapter is a card of its own and must stay out of a physical
    // pair: the physical filter runs after the collapse.
    assert!(!cards[2].1.physical);

    // With no PCI identity reported, the name is the only key left.
    let mut unnamed = evidence("Generic", "Vulkan", 0, true);
    unnamed.vendor = 0;
    let mut unnamed_gl = unnamed.clone();
    unnamed_gl.backend = "Gl".to_string();
    let cards = distinct_cards(&[unnamed, unnamed_gl]);
    assert_eq!(cards.len(), 1, "same name and no ids is one card");
    assert_eq!(cards[0].0, 0);
}
