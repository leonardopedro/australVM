//! U10 (bounded start) acceptance: the descriptor is versioned, and the code
//! generated from it is real Rust.
//!
//! The typecheck is the point. A generator that emits text nobody compiles is
//! a template, not an adapter: here the generated wrapper is handed to `rustc`
//! against the bridge's own rlib, and a deliberately broken copy must fail to
//! compile — otherwise the check proves nothing.


mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::interface::{
    render_buffer_wrapper, render_handle_wrapper, render_wrapper, Adapter, BufferAdapter,
    HandleAdapter, Scalar, ScalarParam, ADAPTER_ABI_VERSION, BUFFER_ABI_VERSION,
    DESCRIPTOR_SCHEMA, HANDLE_ABI_VERSION,
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
    support::cleanup_on_exit("umbrella-iface");
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
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-programs/adapters/adapters.golden");
    let text = std::fs::read_to_string(&golden)
        .unwrap_or_else(|e| panic!("read {}: {e}", golden.display()));
    // The same call the build makes on the file the front-end writes.
    let adapters = Adapter::parse_descriptors(&text).expect("the emitted document parses");
    let symbols: Vec<&str> = adapters.iter().map(|a| a.symbol.as_str()).collect();
    assert_eq!(
        symbols,
        vec![
            "rs_narrow__adapter",
            "rs_scale__adapter",
            "rs_toggle__adapter"
        ],
        "descriptors must be sorted by symbol and the word-native import absent"
    );
    for adapter in &adapters {
        adapter
            .validate()
            .expect("the compiler emits valid descriptors");
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

    // The same document carries the U12 blocks: the compiler's writer and this
    // generator's reader are pinned together for buffers too.
    let buffers = BufferAdapter::parse_descriptors(&text).expect("the buffer blocks parse");
    let symbols: Vec<&str> = buffers.iter().map(|a| a.symbol.as_str()).collect();
    assert_eq!(
        symbols,
        vec!["rs_count__adapter", "rs_fill__adapter", "rs_stamp__adapter"]
    );
    for adapter in &buffers {
        adapter
            .validate()
            .expect("the compiler emits valid buffer descriptors");
        // The first two blocks are version 1 and stay byte-identical; only a
        // signature with a word beside the buffer is written as version 2.
        assert!(adapter.abi_version <= BUFFER_ABI_VERSION);
        assert_eq!(adapter.empty.name(), "null-len0");
        let wrapper = render_buffer_wrapper(adapter).expect("buffer wrapper");
        assert!(
            wrapper.contains(&format!("pub extern \"C\" fn {}__adapter", adapter.target)),
            "{wrapper}"
        );
    }
    // `Pointer` is read-only, `Address` mutable: the direction comes from the
    // parameter type, not from the ownership annotation Monomorphize drops.
    assert_eq!(buffers[0].buffers[0].mode.name(), "read");
    assert_eq!(buffers[0].lengths, Scalar::U64);
    assert_eq!(buffers[0].returns, Scalar::U32);
    assert_eq!(buffers[1].buffers[0].mode.name(), "readwrite");
    assert_eq!(buffers[1].lengths, Scalar::I64);
    assert_eq!(buffers[1].returns, Scalar::I32);
    assert_eq!(buffers[0].params, 2);
    assert_eq!(buffers[1].params, 2);
    assert!(buffers[0].scalars.is_empty());

    // Version 2 carries `tag` beside the buffer: the block names it, so the
    // wrapper passes it through instead of reading it as a second length.
    let stamp = &buffers[2];
    assert_eq!(stamp.abi_version, 2);
    assert_eq!(stamp.params, 3);
    assert_eq!(
        stamp.scalars,
        vec![ScalarParam {
            index: 2,
            ty: Scalar::I64
        }]
    );
    let wrapper = render_buffer_wrapper(stamp).expect("version 2 wrapper");
    assert!(wrapper.contains("s2: i64"), "{wrapper}");
    assert!(wrapper.contains("w2 as i64"), "{wrapper}");
    assert!(!wrapper.contains("let len2"), "{wrapper}");

    // U13: the same pair of goldens covers an owned-value handle. The module
    // writes only the handle's *target*; the lowering derives the dispose
    // symbol its call names and writes this block, which is what this crate
    // turns into `avm_widget__new` / `avm_widget__dispose`.
    let handle_golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-programs/adapters/handles.golden");
    let handle_text = std::fs::read_to_string(&handle_golden)
        .unwrap_or_else(|e| panic!("read {}: {e}", handle_golden.display()));
    let handles =
        HandleAdapter::parse_descriptors(&handle_text).expect("the emitted handle block parses");
    assert_eq!(handles.len(), 1);
    let handle = &handles[0];
    handle
        .validate()
        .expect("the compiler emits a valid handle descriptor");
    assert_eq!(handle.abi_version, HANDLE_ABI_VERSION);
    assert_eq!(handle.schema, DESCRIPTOR_SCHEMA);
    assert_eq!(handle.symbol, "avm_widget__new");
    assert_eq!(handle.dispose_symbol(), "avm_widget__dispose");
    assert_eq!(handle.target, "avm_widget");
    assert_eq!(handle.params, 2);
    assert_eq!(
        handle.use_symbol.as_deref(),
        Some("avm_widget_read"),
        "the module names the provider's reader; the generator renders `__use`"
    );
    assert_eq!(handle.owner, 1, "the generator numbers registries from 1");
    let wrapper = render_handle_wrapper(handle).expect("handle wrapper");
    assert!(
        wrapper.contains("pub extern \"C\" fn avm_widget__new"),
        "{wrapper}"
    );
    assert!(
        wrapper.contains("pub extern \"C\" fn avm_widget__dispose"),
        "{wrapper}"
    );
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
    assert!(Adapter::parse_descriptors(header)
        .expect("header only")
        .is_empty());
}
