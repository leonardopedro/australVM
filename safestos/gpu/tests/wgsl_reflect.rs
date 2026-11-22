//! PC05a: the parsed-WGSL reflection of the real clamp, and its refusals.
//!
//! `REAL_WGSL` is the shader `austral compile --target-type=wgsl
//! --entrypoint=Pointwise:clamp` emits for `examples/gpu/pointwise`; the digest
//! asserted below is the bundle's own `wgsl_sha256`, so a change to the
//! generator that is not reflected here fails this test rather than passing on
//! a stale copy. The KernelIR fixture is the bundle's `kernel_ir` verbatim.
//!
//! Each mutation is applied to the parsed shader *text* and re-parsed, with the
//! WGSL digest recomputed, so a refusal can only come from the reflection or
//! the witness — never from a digest mismatch.

use avm_gpu::schema::{sha256_hex, Expr, KernelIr, MapKind, MemoryPlan, Stmt};
use avm_gpu::wgsl_reflect::{check_artifact, describe, reflect, reflect_ir, render, witness};

const REAL_WGSL: &str = r#"// Generated from typed KernelIR by the Austral GPU backend. Do not edit.
@group(0) @binding(0) var<storage, read> input0: array<u32>;
@group(0) @binding(1) var<storage, read_write> output0: array<u32>;

fn scalar_entry(p0: u32) -> u32 {
    let v0: bool = (p0 > 100u);
    if v0 {
        return 100u;
    } else {
        return p0;
    }
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&input0)) { return; }
    output0[i] = scalar_entry(input0[i]);
}
"#;

/// The bundle's `wgsl_sha256` for the clamp entry, as emitted.
const REAL_WGSL_SHA256: &str =
    "468a150d1e195ffc1670bd45783da8574905ac2c35c0cb865d99439732ad3161";

fn clamp_ir() -> KernelIr {
    KernelIr {
        entry: "clamp".to_string(),
        input_arity: 1,
        helpers: Vec::new(),
        body: Stmt::Seq {
            items: vec![
                Stmt::Let {
                    local: 0,
                    value: Expr::Compare {
                        op: "gt".to_string(),
                        left: Box::new(Expr::Param { index: 0 }),
                        right: Box::new(Expr::U32 {
                            value: "100".to_string(),
                        }),
                    },
                },
                Stmt::If {
                    condition: Expr::Local { index: 0 },
                    then: Box::new(Stmt::Return {
                        value: Expr::U32 {
                            value: "100".to_string(),
                        },
                    }),
                    r#else: Box::new(Stmt::Return {
                        value: Expr::Param { index: 0 },
                    }),
                },
            ],
        },
        memory: MemoryPlan {
            map: MapKind::Map1,
            output_index: "identity".to_string(),
            guard_before_load: true,
            workgroup_size: 64,
        },
        numeric_profile: "u32-modular-v1".to_string(),
    }
}

fn ir_sha() -> String {
    sha256_hex(&clamp_ir().canonical_bytes().unwrap())
}

#[test]
fn the_reflected_shader_is_the_one_the_pipeline_emits() {
    assert_eq!(
        sha256_hex(REAL_WGSL.as_bytes()),
        REAL_WGSL_SHA256,
        "REAL_WGSL is no longer the pipeline's clamp shader; regenerate it"
    );
    let r = reflect(REAL_WGSL, 1).expect("the pipeline's clamp reflects");
    assert_eq!(r.entry, "main");
    assert_eq!(r.helper, "scalar_entry");
    assert_eq!(r.param_name, "p0");
    assert_eq!(r.param_index, 0);
    assert_eq!(r.param_width, 32);
    assert_eq!(r.comparison, "GreaterThan");
    assert_eq!(r.literal, "100");
    assert_eq!(r.branch, "100");
    assert_eq!(r.operator, "umeet");
    assert_eq!(r.input_role, "read");
    assert_eq!(r.output_role, "read_write");
    assert_eq!(r.workgroup_size, 64);
    println!("[pass] PC05a parsed WGSL: {}", describe(&r));
}

#[test]
fn the_parsed_shader_and_the_kernel_ir_agree() {
    let wgsl = reflect(REAL_WGSL, 1).unwrap();
    let ir = reflect_ir(&clamp_ir()).expect("the fixture IR reflects");
    let joined = witness(&wgsl, &ir).expect("both subjects name the same clamp");
    assert_eq!(joined.entry, "clamp", "the artifact records the IR entry");
    assert_eq!(joined.helper, "scalar_entry", "and the shader's helper");
    assert_eq!(joined.operator, "umeet");

    // A KernelIR comparison or literal that differs from the shader is refused.
    let mut other = clamp_ir();
    if let Stmt::Seq { items } = &mut other.body {
        if let Stmt::Let { value, .. } = &mut items[0] {
            if let Expr::Compare { op, .. } = value {
                *op = "lt".to_string();
            }
        }
    }
    let err = witness(&wgsl, &reflect_ir(&other).unwrap()).unwrap_err();
    assert!(err.contains("does not match the KernelIR comparison"), "{err}");

    let mut other = clamp_ir();
    if let Stmt::Seq { items } = &mut other.body {
        if let Stmt::If { then, .. } = &mut items[1] {
            if let Stmt::Return { value } = then.as_mut() {
                *value = Expr::U32 {
                    value: "101".to_string(),
                };
            }
        }
    }
    let err = witness(&wgsl, &reflect_ir(&other).unwrap()).unwrap_err();
    assert!(err.contains("does not match the KernelIR branch"), "{err}");
}

#[test]
fn a_reversed_comparator_reflects_as_the_join_and_breaks_the_witness() {
    let mutated = REAL_WGSL.replace("(p0 > 100u)", "(p0 < 100u)");
    assert_ne!(mutated, REAL_WGSL);
    let r = reflect(&mutated, 1).expect("a clamp shape still reflects");
    assert_eq!(r.comparison, "LessThan");
    assert_eq!(r.operator, "ujoin");
    let err = witness(&r, &reflect_ir(&clamp_ir()).unwrap()).unwrap_err();
    assert!(err.contains("WGSL comparison LessThan"), "{err}");
}

#[test]
fn a_changed_literal_or_operand_is_refused_against_the_ir() {
    let literal = REAL_WGSL.replace("(p0 > 100u)", "(p0 > 101u)").replace(
        "return 100u;",
        "return 101u;",
    );
    let r = reflect(&literal, 1).unwrap();
    assert_eq!(r.literal, "101");
    let err = witness(&r, &reflect_ir(&clamp_ir()).unwrap()).unwrap_err();
    assert!(err.contains("WGSL literal 101"), "{err}");

    // Comparing a constant instead of the parameter is refused by the
    // reflection itself (Naga folds the constant comparison, so the condition
    // is a bool literal and not the reflected fragment).
    let operand = REAL_WGSL.replace("(p0 > 100u)", "(101u > 100u)");
    let err = reflect(&operand, 1).unwrap_err();
    assert!(err.contains("the reflected fragment compares the parameter"), "{err}");

    // A high branch that returns something else is refused too.
    let branch = REAL_WGSL.replace("return 100u;", "return 101u;");
    let err = reflect(&branch, 1).unwrap_err();
    assert!(err.contains("the reflected clamp returns the literal"), "{err}");
}

#[test]
fn a_changed_storage_role_or_width_is_refused() {
    let role = REAL_WGSL.replace(
        "var<storage, read> input0",
        "var<storage, read_write> input0",
    );
    let err = reflect(&role, 1).unwrap_err();
    assert!(
        err.contains("storage role `read_write`") || err.contains("reads 0 storage"),
        "{err}"
    );

    // A signed shader of the same width is still not the reflected u32.
    let width = REAL_WGSL
        .replace("array<u32>", "array<i32>")
        .replace("fn scalar_entry(p0: u32) -> u32", "fn scalar_entry(p0: i32) -> i32")
        .replace("(p0 > 100u)", "(p0 > 100)")
        .replace("return 100u;", "return 100;");
    let err = reflect(&width, 1).unwrap_err();
    assert!(err.contains("not an unsigned integer"), "{err}");
}

#[test]
fn unsupported_parsed_nodes_are_refused() {
    // An extra local is a `Store`/`LocalVariable` statement in the helper, not
    // part of the reflected fragment.
    let extra = REAL_WGSL.replace(
        "    } else {\n        return p0;\n    }\n}",
        "    } else {\n        return p0;\n    }\n    var w: u32 = p0;\n    return w;\n}",
    );
    assert_ne!(extra, REAL_WGSL);
    let err = reflect(&extra, 1).unwrap_err();
    assert!(
        err.contains("not part of the reflected clamp") || err.contains("returns"),
        "{err}"
    );

    // A shader with no scalar helper call is not the reflected fragment.
    let inlined = REAL_WGSL.replace(
        "    output0[i] = scalar_entry(input0[i]);",
        "    if (input0[i] > 100u) { output0[i] = 100u; } else { output0[i] = input0[i]; }",
    );
    let err = reflect(&inlined, 1).unwrap_err();
    assert!(err.contains("calls no scalar helper"), "{err}");
}

#[test]
fn the_artifact_binds_both_subjects_and_a_recomputed_digest_does_not_help() {
    let fresh = witness(
        &reflect(REAL_WGSL, 1).unwrap(),
        &reflect_ir(&clamp_ir()).unwrap(),
    )
    .unwrap();
    let (mm0, mm1) = render(&fresh, REAL_WGSL_SHA256, &ir_sha());
    check_artifact(&fresh, REAL_WGSL_SHA256, &ir_sha(), &mm0).expect("a fresh artifact checks");
    assert!(mm1.contains("pub theorem wgsl_clamp_matches_ir"));
    assert!(!mm1.contains("sorry"));

    // The same inputs render byte-identically, so a committed copy can be
    // regenerated and diffed.
    let (again, _) = render(&fresh, REAL_WGSL_SHA256, &ir_sha());
    assert_eq!(mm0, again);

    // A mutated shader with its own digest recorded: the digest matches the
    // text it was computed from, and the check still refuses because the
    // recorded subject is not the current one.
    let mutated = REAL_WGSL.replace("(p0 > 100u)", "(p0 < 100u)");
    let mutated_sha = sha256_hex(mutated.as_bytes());
    assert_ne!(mutated_sha, REAL_WGSL_SHA256);
    let err = check_artifact(&fresh, REAL_WGSL_SHA256, &ir_sha(), &mm0.replace(REAL_WGSL_SHA256, &mutated_sha)).unwrap_err();
    assert!(err.contains("bound to WGSL subject"), "{err}");

    // An artifact regenerated from the mutated shader is refused against the
    // real reflection, on the recorded line first.
    let mutated_reflection = witness(
        &reflect(&mutated, 1).unwrap(),
        &reflect_ir(&clamp_ir()).unwrap(),
    );
    assert!(mutated_reflection.is_err(), "the witness refuses the reversal");
    let forged = reflect(&mutated, 1).unwrap();
    let (forged_mm0, _) = render(&forged, &mutated_sha, &ir_sha());
    let err = check_artifact(&fresh, &mutated_sha, &ir_sha(), &forged_mm0).unwrap_err();
    assert!(err.contains("re-reflection gives"), "{err}");
}
