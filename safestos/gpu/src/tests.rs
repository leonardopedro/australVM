//! CPU-only tests for the checker: strict loading, digest reproduction,
//! wrapper mutations, and reference evaluation.

use crate::schema::*;
use crate::validate::validate_wgsl;
use std::path::PathBuf;

fn bundle_for_arity(input_arity: u32) -> KernelBundle {
    let helpers = Vec::new();
    let body = Stmt::Return {
        value: Expr::Param { index: 0 },
    };
    let map = if input_arity == 1 {
        MapKind::Map1
    } else {
        MapKind::Map2
    };
    let mut bindings = Vec::new();
    for i in 0..input_arity + 1 {
        bindings.push(serde_json::json!({
            "group": 0,
            "binding": i,
            "role": if i < input_arity { "input" } else { "output" },
        }));
    }
    KernelBundle {
        schema: SCHEMA.to_string(),
        backend: BACKEND.to_string(),
        numeric_profile: NUMERIC_PROFILE.to_string(),
        entry: "k".to_string(),
        input_arity,
        compiler: "test".to_string(),
        sources: vec![Source {
            path: "k.aum".to_string(),
            sha256: "0".repeat(64),
        }],
        kernel_ir: KernelIr {
            entry: "k".to_string(),
            input_arity,
            helpers,
            body,
            memory: MemoryPlan {
                map,
                output_index: "identity".to_string(),
                guard_before_load: true,
                workgroup_size: WORKGROUP_SIZE,
            },
        },
        ir_sha256: "0".repeat(64),
        wgsl_sha256: "0".repeat(64),
        requirements: serde_json::json!({ "bindings": bindings }),
        evidence: Evidence {
            source: "compiled".to_string(),
            shader_validation: "naga".to_string(),
            device_kind: "none".to_string(),
            execution: "reference".to_string(),
            proof: "none".to_string(),
        },
        proof_manifest: None,
    }
}

const BASE_WGSL: &str = r#"
@group(0) @binding(0) var<storage, read> input0: array<u32>;
@group(0) @binding(1) var<storage, read_write> output0: array<u32>;

fn scalar_entry(p0: u32) -> u32 {
    return p0;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&input0)) { return; }
    output0[i] = scalar_entry(input0[i]);
}
"#;

fn audit(wgsl: &str, arity: u32) -> Result<(), String> {
    validate_wgsl(wgsl, &bundle_for_arity(arity))
}

#[test]
fn strict_json_rejects_duplicate_keys() {
    let err = crate::artifact::strict_json_check(br#"{"a":1,"a":2}"#).unwrap_err();
    assert!(err.contains("duplicate JSON key"), "{err}");
}

#[test]
fn strict_json_rejects_unknown_bundle_fields() {
    let mut value = serde_json::to_value(bundle_for_arity(1)).unwrap();
    value["extra"] = serde_json::json!(1);
    let bundle: Result<KernelBundle, _> = serde_json::from_value(value);
    assert!(bundle.is_err());
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "avm-gpu-test-{}-{}-{}",
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

fn write_bundle(dir: &PathBuf, mut bundle: KernelBundle, wgsl: &str) {
    bundle.ir_sha256 = sha256_hex(&bundle.kernel_ir.canonical_bytes().unwrap());
    bundle.wgsl_sha256 = sha256_hex(wgsl.as_bytes());
    std::fs::write(
        dir.join("kernel.json"),
        serde_json::to_vec_pretty(&bundle).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("kernel.wgsl"), wgsl).unwrap();
}

#[test]
fn load_reproduces_digests_and_rejects_tampered_wgsl() {
    let scratch = Scratch::new("digest");
    write_bundle(&scratch.0, bundle_for_arity(1), BASE_WGSL);
    crate::load_bundle(&scratch.0).expect("valid bundle loads");

    std::fs::write(scratch.0.join("kernel.wgsl"), BASE_WGSL.replace("p0", "p1")).unwrap();
    let err = match crate::load_bundle(&scratch.0) {
        Ok(_) => panic!("tampered wgsl was accepted"),
        Err(e) => e,
    };
    assert!(err.contains("wgsl_sha256 does not reproduce"), "{err}");
}

#[test]
fn audit_accepts_the_generated_wrapper_shape() {
    audit(BASE_WGSL, 1).expect("generated shape passes");
}

#[test]
fn audit_rejects_removed_guard() {
    let wgsl = BASE_WGSL.replace(
        "    if (i >= arrayLength(&input0)) { return; }\n",
        "",
    );
    let err = audit(&wgsl, 1).unwrap_err();
    assert!(err.contains("no dominating bounds guard"), "{err}");
}

#[test]
fn audit_rejects_off_by_one_write_index() {
    let wgsl = BASE_WGSL.replace("output0[i] =", "output0[i + 1u] =");
    let err = audit(&wgsl, 1).unwrap_err();
    assert!(err.contains("global_invocation_id.x"), "{err}");
}

#[test]
fn audit_rejects_a_derived_index_local() {
    let wgsl = BASE_WGSL.replace(
        "    let i = gid.x;",
        "    var j = gid.x + 1u;\n    let i = gid.x;",
    )
    .replace("output0[i] =", "output0[j] =");
    let err = audit(&wgsl, 1).unwrap_err();
    assert!(err.contains("store"), "{err}");
}

#[test]
fn audit_rejects_a_second_store() {
    let wgsl = BASE_WGSL.replace(
        "    output0[i] = scalar_entry(input0[i]);",
        "    output0[i] = scalar_entry(input0[i]);\n    output0[i] = 0u;",
    );
    let err = audit(&wgsl, 1).unwrap_err();
    assert!(err.contains("exactly one output store"), "{err}");
}

#[test]
fn audit_rejects_helper_storage_access() {
    let wgsl = BASE_WGSL.replace(
        "fn scalar_entry(p0: u32) -> u32 {\n    return p0;\n}",
        "fn scalar_entry(p0: u32) -> u32 {\n    return input0[0] + p0;\n}",
    );
    let err = audit(&wgsl, 1).unwrap_err();
    assert!(err.contains("scalar helper accesses a storage global"), "{err}");
}

#[test]
fn audit_rejects_an_unread_input() {
    let wgsl = BASE_WGSL.replace(
        "@group(0) @binding(1) var<storage, read_write> output0",
        "@group(0) @binding(1) var<storage, read> input1: array<u32>;\n@group(0) @binding(2) var<storage, read_write> output0",
    );
    let err = audit(&wgsl, 2).unwrap_err();
    // Arity 2 declares input1, but this wrapper never reads it.
    assert!(err.contains("input 1 is never read"), "{err}");
}

#[test]
fn reference_uses_wrapping_u32_semantics() {
    let mut bundle = bundle_for_arity(1);
    bundle.kernel_ir.body = Stmt::Return {
        value: Expr::Intrinsic {
            op: "add".to_string(),
            args: vec![Expr::Param { index: 0 }, Expr::U32 { value: "1".to_string() }],
        },
    };
    let mut ev = crate::reference::Evaluator::new(&bundle.kernel_ir, Default::default()).unwrap();
    let out = ev.run(&[vec![u32::MAX, 41]]).unwrap();
    assert_eq!(out, vec![0, 42]);
}

#[test]
fn reference_refuses_undefined_locals_and_mismatched_lengths() {
    let mut bundle = bundle_for_arity(1);
    bundle.kernel_ir.body = Stmt::Return {
        value: Expr::Local { index: 7 },
    };
    let mut ev = crate::reference::Evaluator::new(&bundle.kernel_ir, Default::default()).unwrap();
    assert!(ev.run(&[vec![1]]).is_err());

    let mut bundle = bundle_for_arity(2);
    bundle.kernel_ir.body = Stmt::Return {
        value: Expr::Param { index: 0 },
    };
    let mut ev = crate::reference::Evaluator::new(&bundle.kernel_ir, Default::default()).unwrap();
    let err = ev.run(&[vec![1], vec![1, 2]]).unwrap_err();
    assert!(err.contains("has length 2, expected 1"), "{err}");
}
