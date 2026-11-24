//! U11 (ABI04/ABI05) — the generated adapter's *values*, measured.
//!
//! `interface.rs` proves the wrapper compiles; this proves it computes. A Rust
//! library exporting the declared targets and the wrapper rendered from the
//! descriptor are compiled into one `cdylib`, then both are called through the
//! real ABI: the target as Rust declares it (`f64`, `i64`, `bool`), the wrapper
//! with `u64` words built by the bridge's own `encode`. Every result is compared
//! **by bits**, because "the float came back" is not the claim — `-0.0` came
//! back as `-0.0`, and an `i64` cast in the middle would return `+0.0`.
//!
//! The cases are chosen so a plausible wrong encoder fails:
//!
//! * `-0.0` (a numeric cast drops the sign),
//! * `rs_sub` (argument order — a commuted pair returns a different value),
//! * `0.0/0.0` and `1.0/0.0` (NaN and infinity survive the word round trip),
//! * `rs_narrow(-7)` (a negative `Int32` sign-extends),
//! * `rs_toggle(true)` (a `Bool` word is 0 or 1, not "nonzero").

mod support;

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use austral_cranelift_bridge::adapters::{decode, encode, ScalarType, ScalarValue};
use umbrella::interface::{render_wrapper, Adapter, Scalar};

fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-scalars-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    support::cleanup_on_exit("umbrella-scalars");
    dir
}

fn bridge_rlib() -> PathBuf {
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

/// The declared targets, written the way Rust declares them, followed by the
/// wrappers rendered from their descriptors.
fn library_source() -> String {
    let adapters = [
        Adapter::new(
            "rs_scale__adapter",
            "rs_scale",
            vec![Scalar::F64, Scalar::I64],
            Scalar::F64,
        ),
        Adapter::new(
            "rs_sub__adapter",
            "rs_sub",
            vec![Scalar::F64, Scalar::F64],
            Scalar::F64,
        ),
        Adapter::new(
            "rs_div__adapter",
            "rs_div",
            vec![Scalar::F64, Scalar::F64],
            Scalar::F64,
        ),
        Adapter::new(
            "rs_narrow__adapter",
            "rs_narrow",
            vec![Scalar::I32],
            Scalar::I64,
        ),
        Adapter::new(
            "rs_toggle__adapter",
            "rs_toggle",
            vec![Scalar::Bool],
            Scalar::Bool,
        ),
    ];
    // The implementations carry the declared *symbol*, not the declared Rust
    // name: the generated wrapper declares `extern "C" { fn rs_scale(..) }` in
    // the same crate, and two items may not share a name. `export_name` is what
    // the wrapper resolves through — exactly what a real library provides.
    let mut source = String::from(
        "// implementation of the declared targets\n\
         #[export_name = \"rs_scale\"]\n\
         pub extern \"C\" fn scala_rs_scale(factor: f64, value: i64) -> f64 { factor * value as f64 }\n\
         #[export_name = \"rs_sub\"]\n\
         pub extern \"C\" fn scala_rs_sub(a: f64, b: f64) -> f64 { a - b }\n\
         #[export_name = \"rs_div\"]\n\
         pub extern \"C\" fn scala_rs_div(a: f64, b: f64) -> f64 { a / b }\n\
         #[export_name = \"rs_narrow\"]\n\
         pub extern \"C\" fn scala_rs_narrow(width: i32) -> i64 { width as i64 }\n\
         #[export_name = \"rs_toggle\"]\n\
         pub extern \"C\" fn scala_rs_toggle(on: bool) -> bool { !on }\n\n",
    );
    // One module per wrapper: the generator writes a file per adapter and each
    // starts with the same `use`, so compiling them into one crate without a
    // module boundary would collide on those imports. `#[no_mangle]` still
    // exports the symbol from the module, which is what the loader looks up.
    for (i, adapter) in adapters.iter().enumerate() {
        source.push_str(&format!(
            "mod w{i} {{\n{}\n}}\n",
            render_wrapper(adapter).expect("render")
        ));
    }
    source
}

/// Compile `source` into one `cdylib`, returning its path.
fn compile(source: &str, crate_name: &str) -> PathBuf {
    let dir = scratch(crate_name);
    let file = dir.join("lib.rs");
    std::fs::write(&file, source).expect("write library");
    let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/deps");
    let rlib = bridge_rlib();
    let out = dir.join("libscalars.so");
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg(format!("--crate-name={crate_name}"))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("austral_cranelift_bridge={}", rlib.display()))
        .arg("-o")
        .arg(&out)
        .arg(&file)
        .output()
        .expect("spawn rustc");
    assert!(
        output.status.success(),
        "the library and its wrappers must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

/// The library under test: implementations plus the rendered wrappers.
fn build_library() -> PathBuf {
    compile(&library_source(), "scalar_roundtrip")
}

struct Library(*mut libc::c_void);

impl Library {
    fn open(path: &Path) -> Self {
        let c = CString::new(path.to_str().expect("utf-8 path")).expect("no NUL");
        let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        assert!(!handle.is_null(), "dlopen {}", path.display());
        Self(handle)
    }

    fn symbol(&self, name: &str) -> *mut libc::c_void {
        let c = CString::new(name).expect("no NUL");
        let ptr = unsafe { libc::dlsym(self.0, c.as_ptr()) };
        assert!(!ptr.is_null(), "dlsym {name}");
        ptr
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe { libc::dlclose(self.0) };
    }
}

type Wrapper2 = extern "C" fn(u64, u64) -> u64;
type Wrapper1 = extern "C" fn(u64) -> u64;

fn f64_bits(v: f64) -> u64 {
    v.to_bits()
}

fn word(ty: ScalarType, value: ScalarValue) -> u64 {
    encode(ty, value).expect("encode")
}

fn f64_word(v: f64) -> u64 {
    word(ScalarType::F64, ScalarValue::F64(v))
}

#[test]
fn the_wrapper_returns_the_same_bits_as_a_direct_call() {
    let path = build_library();
    let lib = Library::open(&path);

    // Direct target calls, written exactly as the declaration says.
    let scale_direct: extern "C" fn(f64, i64) -> f64 =
        unsafe { std::mem::transmute(lib.symbol("rs_scale")) };
    let sub_direct: extern "C" fn(f64, f64) -> f64 =
        unsafe { std::mem::transmute(lib.symbol("rs_sub")) };
    let div_direct: extern "C" fn(f64, f64) -> f64 =
        unsafe { std::mem::transmute(lib.symbol("rs_div")) };
    let narrow_direct: extern "C" fn(i32) -> i64 =
        unsafe { std::mem::transmute(lib.symbol("rs_narrow")) };
    let toggle_direct: extern "C" fn(bool) -> bool =
        unsafe { std::mem::transmute(lib.symbol("rs_toggle")) };

    let scale: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("rs_scale__adapter")) };
    let sub: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("rs_sub__adapter")) };
    let div: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("rs_div__adapter")) };
    let narrow: Wrapper1 = unsafe { std::mem::transmute(lib.symbol("rs_narrow__adapter")) };
    let toggle: Wrapper1 = unsafe { std::mem::transmute(lib.symbol("rs_toggle__adapter")) };

    // Mixed classes, non-integer result, noncommutative parameters.
    let direct = scale_direct(-2.5, 4);
    let through = scale(f64_word(-2.5), word(ScalarType::I64, ScalarValue::I64(4)));
    assert_eq!(through, f64_bits(direct), "wrapper result differs bitwise");
    assert_eq!(f64::from_bits(through), -10.0);

    // Negative zero: a numeric cast anywhere on the float path returns +0.0,
    // so this bit pattern is the discriminating case for F64.
    let direct = scale_direct(-0.0, 5);
    let through = scale(f64_word(-0.0), word(ScalarType::I64, ScalarValue::I64(5)));
    assert_eq!(through, 0x8000_0000_0000_0000, "the sign of -0.0 was lost");
    assert_eq!(through, f64_bits(direct));

    // Argument order: rs_sub(1.5, 4.0) = -2.5, rs_sub(4.0, 1.5) = +2.5.
    let forward = sub(f64_word(1.5), f64_word(4.0));
    let reverse = sub(f64_word(4.0), f64_word(1.5));
    assert_eq!(f64::from_bits(forward), -2.5);
    assert_eq!(f64::from_bits(reverse), 2.5);
    assert_eq!(forward, f64_bits(sub_direct(1.5, 4.0)));
    assert_ne!(forward, reverse, "the wrapper commuted its arguments");

    // NaN: compare to the direct call, not to a constant — the payload is the
    // implementation's, and the claim here is that the *word* carries it.
    let nan = div(f64_word(0.0), f64_word(0.0));
    let direct_nan = div_direct(0.0, 0.0);
    assert!(f64::from_bits(nan).is_nan(), "0.0/0.0 must stay NaN");
    assert_eq!(nan, f64_bits(direct_nan));

    // Infinity, both signs.
    let pos = div(f64_word(1.0), f64_word(0.0));
    let neg = div(f64_word(-1.0), f64_word(0.0));
    assert_eq!(f64::from_bits(pos), f64::INFINITY);
    assert_eq!(f64::from_bits(neg), f64::NEG_INFINITY);
    assert_eq!(pos, f64_bits(div_direct(1.0, 0.0)));

    // A negative Int32 sign-extends to the Int64 result.
    let through = narrow(word(ScalarType::I32, ScalarValue::I32(-7)));
    assert_eq!(
        decode(ScalarType::I64, through).expect("result word"),
        ScalarValue::I64(narrow_direct(-7))
    );
    assert_eq!(decode(ScalarType::I64, through).unwrap(), ScalarValue::I64(-7));

    // A Bool word is 0 or 1, and the wrapper returns the same encoding it read.
    let through = toggle(word(ScalarType::Bool, ScalarValue::Bool(true)));
    assert_eq!(
        decode(ScalarType::Bool, through).expect("result word"),
        ScalarValue::Bool(toggle_direct(true))
    );
    assert_eq!(through, 0, "`true` toggles to the word 0");
    let through = toggle(word(ScalarType::Bool, ScalarValue::Bool(false)));
    assert_eq!(through, 1);
}

/// The negative control for the `-0.0` case above. The plan forbids converting a
/// float through a numeric integer cast; a wrapper that does exactly that on its
/// way out still compiles, still returns `-10.0` for `rs_scale(-2.5, 4)`, and
/// only loses the sign of zero — so the bit comparison is the assertion that
/// has to catch it.
#[test]
fn a_wrapper_that_casts_the_float_through_i64_loses_negative_zero() {
    let adapter = Adapter::new(
        "rs_scale__adapter",
        "rs_scale",
        vec![Scalar::F64, Scalar::I64],
        Scalar::F64,
    );
    let honest = render_wrapper(&adapter).expect("render");
    let mutated = honest.replace(
        "ScalarValue::F64(result)",
        "ScalarValue::F64((result as i64) as f64)",
    );
    assert_ne!(mutated, honest, "the mutation must actually change the source");
    let source = format!(
        "#[export_name = \"rs_scale\"]\n\
         pub extern \"C\" fn scala_rs_scale(factor: f64, value: i64) -> f64 {{ factor * value as f64 }}\n\
         mod w0 {{\n{mutated}\n}}\n"
    );
    let path = compile(&source, "scalar_roundtrip_mutated");
    let lib = Library::open(&path);
    let scale: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("rs_scale__adapter")) };
    let through = scale(f64_word(-0.0), word(ScalarType::I64, ScalarValue::I64(5)));
    assert_eq!(
        through, 0,
        "the cast is expected to return +0.0 here; if it did not, the control is \
         no longer discriminating"
    );
    assert_ne!(
        through,
        f64_bits(-0.0_f64),
        "the honest wrapper returns the sign bit, so this mutation must differ"
    );
}

/// The live path, through the real binary: an Austral module whose three
/// `rs_*` calls are all non-word-native, so the front-end names
/// `<target>__adapter`; the build must generate, compile and load the wrappers,
/// and the JIT must resolve the mediated calls and execute the entry.
///
/// `compute` returns 0 unless every mediated result is right, and the Float64
/// leg is checked end to end: `rs_scale(2.5, 4)` returns 10.0, the module does
/// `(scaled + 2.5 - 0.5) * 2.0 / 4.0` on it, and `rs_bits` reads that float's
/// word, which the entry returns. The arithmetic is CPS-lowered to the bridge's
/// `au_f64_*` primitives, so 6.0's IEEE bits — 0x4018000000000000 — also prove
/// the four operators did not land on the integer `Sub`/`Mul` opcodes (which
/// would compute on the bit patterns, and trap on the multiply). The first bug
/// in this path returned exactly 12, so this one number covers the literal
/// encoding, the `F64` parameter decode, the arithmetic and the result encode.
///
/// The Float32 legs must return 0x3FC00000 (1.5 written straight into an
/// argument) and 0x40200000 (2.5 held in a Float32 local, then scaled) before
/// the Float64 leg runs. Both are checked *inside* the module, so an unnarrowed
/// literal cannot reach the entry as a passing RESULT: its f64 bits have bit 31
/// set and the wrapper's `F32` decode refuses the word, which aborts instead.
///
/// The unsigned legs are the `Nat32`/`Nat64` half of U11 step 1: `rs_u32_low`
/// takes the largest `Nat32` (0xFFFF_FFFF) and returns its low half, and
/// `rs_u64_high` takes `2^63+1` — the word whose sign bit is set — and returns
/// its high half `0x8000_0000`. A `U32` decode that truncated, or a `U64`
/// parameter that was sign-extended or narrowed to 32 bits, changes one of the
/// two numbers the module checks before returning its RESULT.
#[test]
fn the_jit_executes_an_austral_module_through_the_generated_adapters() {
    const EXPECTED: i64 = 0x4018_0000_0000_0000;

    let front_end = Path::new(env!("CARGO_MANIFEST_DIR")).join("../austral");
    assert!(
        front_end.exists(),
        "the live adapter test needs the dune-built front-end at {}; \
         build it with `nix develop \"git+file://$PWD#ocaml\" -c make all`",
        front_end.display()
    );

    // A crate that knows nothing about australVM: plain `rs_*` exports.
    let crate_dir = scratch("live-crate");
    std::fs::create_dir_all(crate_dir.join("src")).expect("crate dir");
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\n\
         name = \"adapter_live\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\n\
         [lib]\n\
         crate-type = [\"cdylib\"]\n",
    )
    .expect("crate manifest");
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        "#[no_mangle]\n\
         pub extern \"C\" fn rs_narrow(width: i32) -> i64 { width as i64 }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_toggle(on: bool) -> bool { !on }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_scale(factor: f64, value: i64) -> f64 { factor * value as f64 }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_bits(value: f64) -> i64 { value.to_bits() as i64 }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_f32_scale(factor: f32, value: i64) -> f32 { factor * value as f32 }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_f32_bits(value: f32) -> i64 { value.to_bits() as i64 }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_u32_low(value: u32) -> u32 { value & 0xFFFF }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_u64_high(value: u64) -> u32 { (value >> 32) as u32 }\n",
    )
    .expect("crate source");

    // The gate sees the symbol the CPS names, which is the adapter's.
    let manifest = scratch("live-manifest").join("module.toml");
    std::fs::write(
        &manifest,
        "[module]\n\
         name = \"AdapterLive\"\n\n\
         [grants]\n\
         kernel = [\"rs_narrow__adapter\", \"rs_toggle__adapter\", \"rs_scale__adapter\", \"rs_bits__adapter\", \"rs_f32_scale__adapter\", \"rs_f32_bits__adapter\", \"rs_u32_low__adapter\", \"rs_u64_high__adapter\"]\n",
    )
    .expect("manifest");

    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("modules/AdapterLive.aum");
    // The CLI builds adapter crates under `$TMPDIR/umbrella-adapters-<pid>-<gen>`
    // — roughly 585 MB each — and must remove them on exit; this test pins that
    // cleanup so a subprocess CLI cannot leak a directory per run. The child pid is
    // captured so the check names exactly the
    // directories this run minted, not the test binary's own or a concurrent
    // run's.
    fn adapters_scratch_for(pid: u32) -> Vec<PathBuf> {
        let prefix = format!("umbrella-adapters-{pid}-");
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix))
                    .unwrap_or(false)
            })
            .collect()
    }
    let run = |expect: i64| {
        let child = Command::new(env!("CARGO_BIN_EXE_umbrella"))
            .arg("--austral")
            .arg(&front_end)
            .arg("--cratedir")
            .arg(&crate_dir)
            .arg("--manifest")
            .arg(&manifest)
            .arg("--entry")
            .arg("compute")
            .arg("--offline")
            .arg("--expect")
            .arg(expect.to_string())
            .arg(&module)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run umbrella");
        let pid = child.id();
        let out = child.wait_with_output().expect("wait for umbrella");
        let leftovers = adapters_scratch_for(pid);
        assert!(
            leftovers.is_empty(),
            "the CLI must remove its adapter scratch directory on exit; \
             pid {pid} left {leftovers:?}"
        );
        out
    };

    let out = run(EXPECTED);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "umbrella failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(stdout.trim(), format!("RESULT {EXPECTED}"), "{stderr}");
    // The wrappers were generated from the emitted descriptors, not found by
    // accident: the driver names every one it built and loaded.
    assert!(
        stderr.contains("built and loaded 8 adapter wrapper(s)"),
        "the build must generate and load the wrappers; stderr:\n{stderr}"
    );
    for symbol in [
        "rs_narrow__adapter",
        "rs_toggle__adapter",
        "rs_scale__adapter",
        "rs_bits__adapter",
        "rs_f32_scale__adapter",
        "rs_f32_bits__adapter",
        "rs_u32_low__adapter",
        "rs_u64_high__adapter",
    ] {
        assert!(stderr.contains(symbol), "stderr must name {symbol}:\n{stderr}");
    }

    // The oracle: the same module with a wrong expectation must fail, so
    // "RESULT 0" from a broken mediated call cannot pass as agreement.
    let out = run(0);
    assert!(
        !out.status.success(),
        "a wrong --expect must fail; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
