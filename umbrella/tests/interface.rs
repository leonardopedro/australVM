//! U10 (bounded start) acceptance: the descriptor is versioned, and the code
//! generated from it is real Rust.
//!
//! The typecheck is the point. A generator that emits text nobody compiles is
//! a template, not an adapter: here the generated wrapper is handed to `rustc`
//! against the bridge's own rlib, and a deliberately broken copy must fail to
//! compile — otherwise the check proves nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::interface::{
    render_wrapper, Adapter, Scalar, ADAPTER_ABI_VERSION, DESCRIPTOR_SCHEMA,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adapters")
}

/// A directory no other test can collide with: tests run in parallel, and a
/// shared path silently let one test compile the other's source.
fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-iface-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn bridge_rlib() -> PathBuf {
    // Cargo keeps the hashed rlib in `deps/` for some profiles and the
    // unhashed copy at the profile root for others; accept either.
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug");
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in [target.join("deps"), target.clone()] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let is_rlib = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("libaustral_cranelift_bridge") && n.ends_with(".rlib"))
                .unwrap_or(false);
            if is_rlib {
                found.push(path);
            }
        }
    }
    found.sort();
    found.pop().unwrap_or_else(|| {
        panic!(
            "no bridge rlib under {}; run `cargo test` first",
            target.display()
        )
    })
}

/// `rustc --emit=metadata` on `source`, returning the compiler's stderr.
fn typecheck(source: &str) -> Result<(), String> {
    let dir = scratch("typecheck");
    let file = dir.join("wrapper.rs");
    std::fs::write(&file, source).expect("write source");
    let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/deps");
    let rlib = bridge_rlib();
    let out = Command::new("rustc")
        .arg("--crate-type=lib")
        .arg("--edition=2021")
        .arg("--emit=metadata")
        .arg("--crate-name=generated_wrapper")
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("austral_cranelift_bridge={}", rlib.display()))
        .arg("-o")
        .arg(dir.join("wrapper.rmeta"))
        .arg(&file)
        .output()
        .expect("spawn rustc");
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[test]
fn the_descriptor_names_the_encoding_the_bridge_implements() {
    assert_eq!(DESCRIPTOR_SCHEMA, 1);
    assert_eq!(
        ADAPTER_ABI_VERSION,
        austral_cranelift_bridge::adapters::SCALAR_ABI_VERSION,
        "the descriptor's encoding version must be the bridge's"
    );
}

#[test]
fn the_committed_golden_descriptor_roundtrips() {
    let text =
        std::fs::read_to_string(fixtures().join("rs_scale.descriptor")).expect("golden descriptor");
    let adapter = Adapter::parse_descriptor(&text).expect("parse golden");
    assert_eq!(
        adapter,
        Adapter::new(
            "rs_scale__adapter",
            "rs_scale",
            vec![Scalar::I32, Scalar::F64],
            Scalar::F64
        )
    );
    assert_eq!(
        adapter.descriptor_text(),
        text,
        "canonical form must be stable"
    );
    assert_eq!(
        render_wrapper(&adapter).expect("render"),
        std::fs::read_to_string(fixtures().join("rs_scale.wrapper.rs")).expect("golden wrapper")
    );
}

#[test]
fn the_generated_wrapper_typechecks_against_the_bridge() {
    let adapter = Adapter::new(
        "rs_scale__adapter",
        "rs_scale",
        vec![Scalar::I32, Scalar::F64],
        Scalar::F64,
    );
    let source = render_wrapper(&adapter).expect("render");
    typecheck(&source).expect("the generated wrapper must compile");
}

#[test]
fn a_wrapper_whose_call_does_not_match_its_signature_is_rejected_by_rustc() {
    let adapter = Adapter::new(
        "rs_scale__adapter",
        "rs_scale",
        vec![Scalar::I32, Scalar::F64],
        Scalar::F64,
    );
    let source = render_wrapper(&adapter).expect("render");
    let broken = source.replace("unsafe { rs_scale(a0, a1) }", "unsafe { rs_scale(a0) }");
    assert_ne!(
        broken, source,
        "the mutation must actually change the source"
    );
    let err = typecheck(&broken).expect_err("a one-argument call to a two-argument fn");
    assert!(
        err.contains("E0061") || err.contains("arguments were supplied"),
        "the control must fail for the missing argument, not something else:\n{err}"
    );
}

#[test]
fn a_descriptor_with_another_encoding_version_cannot_render() {
    let mut adapter = Adapter::new("s__adapter", "s", vec![Scalar::I64], Scalar::I64);
    adapter.abi_version = 99;
    let err = render_wrapper(&adapter).unwrap_err();
    assert!(err.contains("scalar encoding version 99"), "{err}");
}

/// The compiler's own emission must be readable here: this pins the two sides
/// of U11 together. `scripts/check-adapter-descriptors` keeps the golden equal
/// to what the CPS lowering emits; this test keeps the golden parseable by the
/// generator, so a field the compiler renames cannot pass unnoticed.
#[test]
fn the_compilers_emitted_descriptors_parse_here() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../test-programs/adapters/adapters.golden");
    let text = std::fs::read_to_string(&golden)
        .unwrap_or_else(|e| panic!("read {}: {e}", golden.display()));
    // The same call the build makes on the file the front-end writes.
    let adapters = Adapter::parse_descriptors(&text).expect("the emitted document parses");
    let symbols: Vec<&str> = adapters.iter().map(|a| a.symbol.as_str()).collect();
    assert_eq!(
        symbols,
        vec!["rs_narrow__adapter", "rs_scale__adapter", "rs_toggle__adapter"],
        "descriptors must be sorted by symbol and the word-native import absent"
    );
    for adapter in &adapters {
        adapter.validate().expect("the compiler emits valid descriptors");
        assert_eq!(adapter.abi_version, ADAPTER_ABI_VERSION);
        assert_eq!(adapter.schema, DESCRIPTOR_SCHEMA);
        assert_eq!(
            adapter.symbol.strip_suffix("__adapter").map(str::to_string),
            Some(adapter.target.clone()),
            "the wrapper exports `<target>__adapter`"
        );
    }
    assert_eq!(adapters[1].params, vec![Scalar::F64, Scalar::I64]);
    assert_eq!(adapters[1].returns, Scalar::F64);
}

/// The build's parser is the one that has to be strict, not the front-end's
/// writer: a document with two blocks for one symbol is refused rather than
/// letting the second wrapper's layout silently win.
#[test]
fn a_document_with_a_repeated_symbol_is_refused() {
    let block = "schema 1\nabi scalar 1\nsymbol rs_scale__adapter\ntarget rs_scale\n\
                 params Int32,Float64\nreturns Float64\n";
    let err = Adapter::parse_descriptors(&format!("{block}\n{block}")).unwrap_err();
    assert!(err.contains("duplicate descriptor"), "{err}");
    assert!(err.contains("rs_scale__adapter"), "{err}");

    // A comment-only document is *not* an error: a module whose calls are all
    // word-native writes exactly the compiler's header and needs no wrapper.
    let header = "# U11 adapter descriptors\n# sorted by symbol.\n";
    assert!(Adapter::parse_descriptors(header).expect("header only").is_empty());
}
