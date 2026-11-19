//! G12: one bundle end to end through `cargo test --features device`.
//!
//! This is a *real* dispatch: it builds a two-input add bundle in a scratch
//! directory, validates it, evaluates the KernelIR reference, and requires the
//! device output to agree element for element. On a host with no physical
//! adapter the test fails unless `AVM_GPU_ALLOW_NO_DEVICE=1` is set, in which
//! case it prints a `[gap]` line and returns; a skip is never reported as a
//! hardware pass.

#![cfg(feature = "device")]

use avm_gpu::schema::*;
use avm_gpu::run::RunFailure;
use avm_gpu::{reference, validate};
use std::path::{Path, PathBuf};

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

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "avm-gpu-e2e-{}-{}-{}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn add_bundle() -> KernelBundle {
    let mut bindings = Vec::new();
    for (i, role) in ["input", "input", "output"].iter().enumerate() {
        bindings.push(serde_json::json!({
            "group": 0,
            "binding": i,
            "role": role,
            "element": "u32",
            "stride_bytes": 4,
        }));
    }
    KernelBundle {
        schema: SCHEMA.to_string(),
        backend: BACKEND.to_string(),
        numeric_profile: NUMERIC_PROFILE.to_string(),
        entry: "Add:k".to_string(),
        input_arity: 2,
        compiler: "g12-device-test".to_string(),
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
    }
}

fn write_bundle(dir: &Path) {
    let mut bundle = add_bundle();
    bundle.ir_sha256 = sha256_hex(&bundle.kernel_ir.canonical_bytes().unwrap());
    bundle.wgsl_sha256 = sha256_hex(ADD_WGSL.as_bytes());
    std::fs::write(
        dir.join("kernel.json"),
        serde_json::to_vec_pretty(&bundle).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("kernel.wgsl"), ADD_WGSL).unwrap();
}

#[test]
fn add_bundle_round_trips_through_the_device() {
    let scratch = Scratch::new("add");
    write_bundle(&scratch.0);
    let loaded = avm_gpu::load_bundle(&scratch.0).expect("bundle loads");
    validate::validate_bundle(&loaded).expect("bundle validates");

    let inputs = vec![
        vec![0, 1, 2, u32::MAX, 65, 64],
        vec![0, 2, 3, 1, 1, 0],
    ];
    let expected = reference::Evaluator::new(&loaded.bundle.kernel_ir, reference::Limits::default())
        .unwrap()
        .run(&inputs)
        .unwrap();
    assert_eq!(expected, vec![0, 3, 5, 0, 66, 64]);

    let selection = avm_gpu::device::Selection::default();
    match avm_gpu::run::run(&loaded, &inputs, true, &selection) {
        Ok(result) => {
            assert!(result.adapter.physical, "device must be a physical GPU");
            println!(
                "[pass] G12 device add: {} ({}, {}), {} elements, dispatch {}, {:?}",
                result.adapter.name,
                result.adapter.backend,
                result.adapter.device_type,
                result.preflight.n,
                result.dispatch_count,
                result.output
            );
            assert_eq!(result.dispatch_count, 1);
            assert_eq!(result.output, expected);
        }
        Err(RunFailure::Unavailable(message)) => {
            if std::env::var_os("AVM_GPU_ALLOW_NO_DEVICE").is_some() {
                eprintln!("[gap] G12 device add not run: {message}");
            } else {
                panic!(
                    "no physical adapter: {message} (set AVM_GPU_ALLOW_NO_DEVICE=1 only to \
                     record an explicit gap, never a hardware pass)"
                );
            }
        }
        Err(RunFailure::Refused(message)) => panic!("device refused the bundle: {message}"),
    }
}

/// The service publishes a job's completion from this hook, so it must fire
/// with the same readback the blocking path returns — and it is the queue's
/// submitted-work-done event, not a post-return step.
#[test]
fn completion_hook_receives_the_readback_from_the_work_done_event() {
    let scratch = Scratch::new("hook");
    write_bundle(&scratch.0);
    let loaded = avm_gpu::load_bundle(&scratch.0).expect("bundle loads");
    validate::validate_bundle(&loaded).expect("bundle validates");

    let inputs = vec![vec![1, 2, 3, 4], vec![10, 20, 30, 40]];
    let selection = avm_gpu::device::Selection::default();
    let (sender, receiver) = std::sync::mpsc::channel();
    match avm_gpu::run::run_with_completion(
        &loaded,
        &inputs,
        true,
        &selection,
        move |words| {
            let _ = sender.send(words.to_vec());
        },
    ) {
        Ok(result) => {
            let published = receiver
                .try_recv()
                .expect("the work-done hook fired before run returned");
            assert_eq!(published, result.output);
            assert_eq!(published, vec![11, 22, 33, 44]);
            assert!(receiver.try_recv().is_err(), "the hook fires once");
            println!(
                "[pass] G15 completion hook: {} ({})",
                result.adapter.name, result.adapter.backend
            );
        }
        Err(RunFailure::Unavailable(message)) => {
            if std::env::var_os("AVM_GPU_ALLOW_NO_DEVICE").is_some() {
                eprintln!("[gap] G15 completion hook not run: {message}");
            } else {
                panic!("no physical adapter: {message}");
            }
        }
        Err(RunFailure::Refused(message)) => panic!("device refused the bundle: {message}"),
    }
}

#[test]
fn explicit_adapter_selection_refuses_bad_selectors() {
    let mut selection = avm_gpu::device::Selection {
        name: Some("  ".to_string()),
        index: None,
    };
    assert!(selection.validate().is_err());
    selection.name = None;
    selection.index = Some(usize::MAX);
    // The selector is syntactically valid; asking the adapter layer must fail
    // with an out-of-range error rather than silently falling back.
    match avm_gpu::device::probe(&selection) {
        Err(avm_gpu::device::ProbeError::Failed(message)) => {
            assert!(message.contains("out of range"), "{message}")
        }
        other => panic!("out-of-range selector was not refused: {other:?}"),
    }
}
