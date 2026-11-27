//! U18 bounded start: a warm front-end run reuses the checked artifact, and any
//! edit to an input the artifact depends on rebuilds it.
//!
//! The compiler is a script whose output embeds a run counter, so "the compiler
//! did not run" is measured (the counter file) instead of inferred from a fast
//! test. A cache entry is bytes and the compiler's own reported facts; the
//! admission checks that use it (`trust::verify_*`, the UK-4001 call gate) are
//! not part of the cache and run on every hit.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use umbrella::cache::{self, Outcome};
use umbrella::frontend::Artifact;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "umbrella-cache-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    let mut perm = std::fs::metadata(path).expect("meta").permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(path, perm).expect("chmod");
}

/// A front-end that writes a CPS buffer naming its own run number and a fixed
/// adapter document, and reports the facts `frontend::compile` requires.
fn fake_compiler(dir: &Path, version: &str) -> PathBuf {
    let path = dir.join("fake-austral");
    let runs = dir.join("runs");
    executable(
        &path,
        &format!(
            "#!/bin/sh\n\
             for a in \"$@\"; do\n\
               case \"$a\" in\n\
                 --emit-cps=*) out=\"${{a#--emit-cps=}}\" ;;\n\
                 --emit-adapters=*) adapters=\"${{a#--emit-adapters=}}\" ;;\n\
               esac\n\
             done\n\
             [ -n \"$out\" ] && [ -n \"$adapters\" ] || exit 9\n\
             n=$(cat '{runs}' 2>/dev/null || echo 0)\n\
             n=$((n+1))\n\
             echo \"$n\" > '{runs}'\n\
             printf 'CPS%s-{version}' \"$n\" > \"$out\"\n\
             printf 'abi scalar 1\\nsymbol rs_probe__adapter\\n' > \"$adapters\"\n\
             bytes=$(wc -c < \"$out\" | tr -d ' ')\n\
             printf 'CPS JIT: module M\\nCPS JIT: Generated 1 functions (%s bytes)\\n\
             CPS JIT: export\\tcompute\\t\\tInt64\\n\
             CPS JIT: Emitted CPS binary to %s\\n' \"$bytes\" \"$out\" >&2\n\
             exit 0\n",
            runs = runs.display(),
            version = version
        ),
    );
    path
}

fn runs(dir: &Path) -> u32 {
    std::fs::read_to_string(dir.join("runs"))
        .expect("the compiler ran")
        .trim()
        .parse()
        .expect("run counter")
}

fn module(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write module");
    path
}

fn compile(
    dir: &Path,
    compiler: &Path,
    modules: &[PathBuf],
    liquid_required: bool,
) -> (Artifact, Outcome) {
    cache::compile_cached(
        compiler,
        modules,
        liquid_required,
        Some(&dir.join("cache")),
    )
    .expect("compile through the cache")
}

#[test]
fn a_warm_run_reuses_the_artifact_and_the_compiler_does_not_run() {
    let dir = scratch("warm");
    let compiler = fake_compiler(&dir, "v1");
    let source = module(&dir, "M.aum", "module M;\n");

    let (first, outcome) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(outcome, Outcome::Miss(_)), "{outcome:?}");
    assert_eq!(first.cps, b"CPS1-v1");
    assert_eq!(first.functions, 1);
    assert_eq!(runs(&dir), 1);

    let (second, outcome) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(outcome, Outcome::Hit(_)), "{outcome:?}");
    assert_eq!(second, first, "a hit must hand back the same artifact");
    assert_eq!(
        second.adapters, "abi scalar 1\nsymbol rs_probe__adapter\n",
        "the adapter document a project pins is part of the cached artifact"
    );
    assert_eq!(runs(&dir), 1, "the compiler must not run on a warm hit");
}

#[test]
fn an_edited_source_rebuilds() {
    let dir = scratch("source");
    let compiler = fake_compiler(&dir, "v1");
    let source = module(&dir, "M.aum", "module M;\n");
    let (_, first) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(first, Outcome::Miss(_)), "{first:?}");

    std::fs::write(&source, "module M;\n-- an edit\n").unwrap();
    let (artifact, outcome) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(outcome, Outcome::Miss(_)), "{outcome:?}");
    assert_eq!(artifact.cps, b"CPS2-v1");
    assert_eq!(runs(&dir), 2);
    assert_ne!(outcome.key(), first.key(), "an edit must move the key");
}

#[test]
fn an_edited_import_graph_rebuilds() {
    let dir = scratch("graph");
    let compiler = fake_compiler(&dir, "v1");
    let a = module(&dir, "A.aum", "module A;\n");
    let b = module(&dir, "B.aum", "module B;\n");
    let ordered = vec![a.clone(), b.clone()];

    let (_, first) = compile(&dir, &compiler, &ordered, false);
    assert!(matches!(first, Outcome::Miss(_)), "{first:?}");

    // The source bytes are the import graph: B now imports A.
    std::fs::write(&b, "module B;\nimport A;\n").unwrap();
    let (artifact, outcome) = compile(&dir, &compiler, &ordered, false);
    assert!(matches!(outcome, Outcome::Miss(_)), "{outcome:?}");
    assert_eq!(artifact.cps, b"CPS2-v1");
    assert_ne!(outcome.key(), first.key());
}

#[test]
fn a_new_compiler_binary_rebuilds() {
    let dir = scratch("compiler");
    let compiler = fake_compiler(&dir, "v1");
    let source = module(&dir, "M.aum", "module M;\n");
    let (_, first) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(first, Outcome::Miss(_)), "{first:?}");

    // Same script behaviour, different bytes: the toolchain is an input.
    let compiler = fake_compiler(&dir, "v2");
    let (artifact, outcome) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(outcome, Outcome::Miss(_)), "{outcome:?}");
    assert_eq!(artifact.cps, b"CPS2-v2");
    assert_ne!(outcome.key(), first.key());
}

#[test]
fn the_liquid_mode_rebuilds() {
    let dir = scratch("liquid");
    let compiler = fake_compiler(&dir, "v1");
    let source = module(&dir, "M.aum", "module M;\n");
    let (_, first) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(first, Outcome::Miss(_)), "{first:?}");

    let (artifact, outcome) = compile(&dir, &compiler, std::slice::from_ref(&source), true);
    assert!(matches!(outcome, Outcome::Miss(_)), "{outcome:?}");
    assert_eq!(artifact.cps, b"CPS2-v1");
    assert_ne!(outcome.key(), first.key());
    assert_eq!(
        artifact.liquid,
        umbrella::frontend::Liquid::Required,
        "the cached artifact keeps the mode it was built in"
    );
}

#[test]
fn a_corrupted_entry_is_refused_and_rebuilt() {
    let dir = scratch("corrupt");
    let compiler = fake_compiler(&dir, "v1");
    let source = module(&dir, "M.aum", "module M;\n");
    let (_, first) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    let key = first.key().expect("a key").to_string();
    assert!(matches!(first, Outcome::Miss(_)), "{first:?}");

    std::fs::write(dir.join("cache").join(&key).join("module.cps"), b"truncated").unwrap();
    let (artifact, outcome) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    match &outcome {
        Outcome::Invalid(reported, reason) => {
            assert_eq!(reported, &key);
            assert!(reason.contains("sha256"), "{reason}");
        }
        other => panic!("a corrupted entry must be reported, got {other:?}"),
    }
    assert_eq!(artifact.cps, b"CPS2-v1", "the rebuild is the fresh artifact");
    assert_eq!(runs(&dir), 2);

    // The refusal replaced the bad entry, so the next run is warm again.
    let (_, outcome) = compile(&dir, &compiler, std::slice::from_ref(&source), false);
    assert!(matches!(outcome, Outcome::Hit(_)), "{outcome:?}");
    assert_eq!(runs(&dir), 2);
}

#[test]
fn without_a_cache_directory_every_run_compiles() {
    let dir = scratch("disabled");
    let compiler = fake_compiler(&dir, "v1");
    let source = module(&dir, "M.aum", "module M;\n");
    for expected_run in 1..=2 {
        let (artifact, outcome) =
            cache::compile_cached(&compiler, std::slice::from_ref(&source), false, None)
                .expect("compile");
        assert_eq!(outcome, Outcome::Disabled);
        assert_eq!(artifact.cps, format!("CPS{expected_run}-v1").as_bytes());
        assert_eq!(runs(&dir), expected_run);
    }
}
