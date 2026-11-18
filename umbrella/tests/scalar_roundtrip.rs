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

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;

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
