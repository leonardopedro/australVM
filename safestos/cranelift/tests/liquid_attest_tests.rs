//! PLAN_liquid_types.md L8: the load-time attestation gate, exercised against a
//! fixture whose `liquid.ok` was written by the **OCaml** compiler, not by
//! Rust.
//!
//! That is the point of this file. `liquid_attest.rs` computes an FNV-1a
//! digest in Rust; `lib/liquid/LiquidWhy3.ml` computes the same digest in
//! OCaml and writes the sidecar. Two independent implementations of one hash
//! will drift, and if they do every attestation reads as stale. The fixture is
//! pinned with an OCaml-written `liquid.ok`, so this test fails the moment the
//! two sides stop agreeing — which is the only cheap way to keep them honest.
//!
//! Regenerate the fixture with:
//!
//! ```text
//! AUSTRAL_LIQUID_VERIFY=1 AUSTRAL_LIQUID_SIDECAR=<dir> \
//! AUSTRAL_LIQUID_DUMP=<dir> WHY3_CLI=<prover> \
//!   dune build bin/ && ./_build/default/bin/austral compile --target-type=c \
//!   <dir>/Demo.aui,<dir>/Demo.aum --entrypoint=Demo:main --output=<dir>/module.c
//! cp <dir>/Demo.mlw <dir>/module.mlw
//! ```

use austral_cranelift_bridge::auth::LiquidMode;
use austral_cranelift_bridge::liquid_attest;
use std::fs;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("liquid_demo")
}

/// A sidecar written by the OCaml compiler satisfies the Rust gate.
#[test]
fn ocaml_written_attestation_is_accepted() {
    let d = fixture();
    let note = liquid_attest::verify_module(&d, LiquidMode::Required)
        .expect("an OCaml-written attestation must satisfy the Rust gate");
    assert!(
        note.contains("why3") || note.contains("liquid.ok accepted"),
        "unexpected note: {}",
        note
    );
}

/// Editing a source makes the attestation stale, and a stale attestation is a
/// refusal — the property that makes the compiler's verdict load-bearing
/// instead of advisory.
#[test]
fn editing_a_source_makes_the_attestation_stale() {
    let d = fixture();
    // Work on a copy so the pinned fixture is never mutated.
    let tmp = std::env::temp_dir().join("austral-liquid-stale-e2e");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    for f in ["Demo.aui", "Demo.aum", "liquid.ok", "module.mlw"] {
        fs::copy(d.join(f), tmp.join(f)).unwrap();
    }

    // Unedited copy is fine.
    assert!(liquid_attest::verify_module(&tmp, LiquidMode::Required).is_ok());

    // Change the postcondition, exactly as editing a contract would.
    let body = fs::read_to_string(tmp.join("Demo.aum")).unwrap();
    fs::write(
        tmp.join("Demo.aum"),
        body.replace("result >= n", "result >= 0"),
    )
    .unwrap();

    let err = liquid_attest::verify_module(&tmp, LiquidMode::Required).unwrap_err();
    assert!(
        err.contains("LiquidAttestationStale"),
        "expected a staleness refusal, got: {}",
        err
    );
}

/// `off` is untouched by any of this: a module that says nothing about liquid
/// types is hosted exactly as it was before the feature existed.
#[test]
fn off_modules_are_unaffected() {
    let d = std::env::temp_dir().join("austral-liquid-off-e2e");
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    // Deliberately no liquid.ok at all.
    fs::write(d.join("Demo.aum"), "module body Demo is end module body.\n").unwrap();
    assert!(liquid_attest::verify_module(&d, LiquidMode::Off).is_ok());
    assert!(liquid_attest::verify_module(&d, LiquidMode::Optional).is_ok());
    // …but `required` still refuses it.
    assert!(liquid_attest::verify_module(&d, LiquidMode::Required).is_err());
}
