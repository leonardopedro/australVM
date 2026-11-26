//! U15 — the checked scalar wrapper's status channel, measured.
//!
//! The payload wrapper returns the encoded value and aborts on a bad word or a
//! provider panic. The checked wrapper (`<symbol>__checked`) returns a status
//! word instead and writes the payload through `out`, so a caller can branch.
//! This compiles the provider and the rendered wrapper into one `cdylib` and
//! calls them through the real ABI: success, a refused word, a provider panic,
//! and a null out slot.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;

use austral_cranelift_bridge::adapters::{encode, ScalarType, ScalarValue};
use austral_cranelift_bridge::handles::{STATUS_OK, STATUS_PANIC, STATUS_REFUSED};
use umbrella::interface::{
    render_wrapper_checked, render_wrapper_checked_option, Adapter, Scalar,
};

fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-checked-{}-{unique}-{name}",
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

fn compiled_library() -> PathBuf {
    let identity = Adapter::new("rs_checked", "rs_checked", vec![Scalar::I64], Scalar::I64);
    let strict_bool = Adapter::new(
        "rs_checked_bool",
        "rs_checked_bool",
        vec![Scalar::Bool],
        Scalar::Bool,
    );
    let panicking = Adapter::new(
        "rs_checked_fault",
        "rs_checked_fault",
        vec![Scalar::I64],
        Scalar::I64,
    );
    // U15's Option tag: a scalar parameter whose declared result is
    // `Option[Int64]`. The checked wrapper allocates `{tag, payload}` and
    // returns it, so the test reads the union exactly as the JIT's `match`
    // does. Tag 0 is `None`, tag 1 is `Some`.
    let mut option = Adapter::new(
        "rs_lookup__adapter",
        "rs_lookup",
        vec![Scalar::Bool],
        Scalar::I64,
    );
    option.checked = true;
    option.checked_tags = Some((0, 1));
    let mut option_fault = Adapter::new(
        "rs_lookup_fault__adapter",
        "rs_lookup_fault",
        vec![Scalar::Bool],
        Scalar::I64,
    );
    option_fault.checked = true;
    option_fault.checked_tags = Some((0, 1));
    let mut source = String::from(
        "#[export_name = \"rs_checked\"]\n\
         pub extern \"C-unwind\" fn provider_checked(value: i64) -> i64 { value }\n\n\
         #[export_name = \"rs_checked_bool\"]\n\
         pub extern \"C-unwind\" fn provider_bool(value: bool) -> bool { value }\n\n\
         #[export_name = \"rs_checked_fault\"]\n\
         pub extern \"C-unwind\" fn provider_fault(_value: i64) -> i64 {\n\
             panic!(\"the provider call failed\");\n\
         }\n\n\
         #[export_name = \"rs_lookup\"]\n\
         pub extern \"C-unwind\" fn lookup(flag: bool) -> i64 { if flag { 7 } else { 9 } }\n\n\
         #[export_name = \"rs_lookup_fault\"]\n\
         pub extern \"C-unwind\" fn lookup_fault(_flag: bool) -> i64 {\n\
             panic!(\"the lookup failed\");\n\
         }\n\n",
    );
    for (i, adapter) in [&identity, &strict_bool, &panicking, &option, &option_fault]
        .iter()
        .enumerate()
    {
        let rendered = if adapter.checked {
            render_wrapper_checked_option(adapter).expect("render option")
        } else {
            render_wrapper_checked(adapter).expect("render")
        };
        source.push_str(&format!("mod w{i} {{\n{rendered}\n}}\n"));
    }

    let dir = scratch("wrappers");
    let file = dir.join("lib.rs");
    std::fs::write(&file, source).expect("write library");
    let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/deps");
    let rlib = bridge_rlib();
    let out = dir.join("libchecked.so");
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=checked_wrappers")
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
        "the provider and its checked wrappers must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
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

    fn checked_fn(&self, name: &str) -> unsafe extern "C" fn(u64, *mut u64) -> i64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }

    fn option_fn(&self, name: &str) -> unsafe extern "C" fn(u64) -> *mut u64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.0);
        }
    }
}

#[test]
fn a_checked_scalar_wrapper_reports_status_and_writes_the_payload() {
    let lib = Library::open(&compiled_library());
    let identity = lib.checked_fn("rs_checked__checked");
    let strict_bool = lib.checked_fn("rs_checked_bool__checked");
    let fault = lib.checked_fn("rs_checked_fault__checked");
    let good = encode(ScalarType::I64, ScalarValue::I64(41)).expect("encode an i64 word");

    // 1. Success: the payload reaches `out`.
    let mut out = [0u64; 1];
    assert_eq!(unsafe { identity(good, out.as_mut_ptr()) }, STATUS_OK);
    assert_eq!(out[0], good);

    // 2. A word that does not decode is refused, and the out slot is untouched.
    //    A boolean word must be 0 or 1; 2 is the `decode` refusal.
    let mut out = [0xdead_beefu64; 1];
    assert_eq!(unsafe { strict_bool(2, out.as_mut_ptr()) }, STATUS_REFUSED);
    assert_eq!(out[0], 0xdead_beef, "a refused call writes nothing");

    // 3. A provider panic is a status, not an abort, and writes nothing.
    let mut out = [0xfeed_faceu64; 1];
    assert_eq!(unsafe { fault(good, out.as_mut_ptr()) }, STATUS_PANIC);
    assert_eq!(out[0], 0xfeed_face, "a panicking call writes nothing");

    // 4. A null out slot is refused before the provider runs.
    assert_eq!(
        unsafe { identity(good, std::ptr::null_mut()) },
        STATUS_REFUSED
    );

    // The same call still works after a refusal and after a panic.
    let mut out = [0u64; 1];
    assert_eq!(unsafe { identity(good, out.as_mut_ptr()) }, STATUS_OK);
    assert_eq!(out[0], good);
}

/// U15's Option tag, measured through the real ABI: the wrapper returns the
/// union pointer itself, with the tag at offset 0 and the encoded payload at
/// offset 8 — exactly the layout `Mt.MCase` reads. A bad word and a provider
/// panic are the failure tag, never the payload word.
#[test]
fn a_checked_option_wrapper_returns_the_union_the_jit_matches_on() {
    let lib = Library::open(&compiled_library());
    let lookup = lib.option_fn("rs_lookup__adapter__checked");
    let fault = lib.option_fn("rs_lookup_fault__adapter__checked");
    let union = |ptr: *mut u64| unsafe { [*ptr, *ptr.add(1)] };

    let some = union(unsafe { lookup(1) });
    assert_eq!(some[0], 1, "Some is the success tag");
    assert_eq!(
        some[1],
        encode(ScalarType::I64, ScalarValue::I64(7)).expect("encode the payload"),
        "the payload word is the encoded domain result"
    );

    let refused = union(unsafe { lookup(2) });
    assert_eq!(refused[0], 0, "a bad bool word is None");
    assert_eq!(refused[1], 0, "a failure carries no payload");

    let panicked = union(unsafe { fault(1) });
    assert_eq!(panicked[0], 0, "a caught provider panic is None");
    assert_eq!(panicked[1], 0, "a panic carries no payload");

    let none = union(unsafe { lookup(0) });
    assert_eq!(none[0], 1, "a false flag is still a successful call");
    assert_eq!(
        none[1],
        encode(ScalarType::I64, ScalarValue::I64(9)).expect("encode the false branch")
    );
}

/// U15 end-to-end: the typer admits `Option[Int64]` for an `rs_*` import, the
/// CPS lowering selects `<symbol>__checked`, the generated wrapper is built and
/// loaded, and the JIT runs a real `match` over the returned union.
///
/// The provider lives in its own dylib here, like a real one, so the panic leg
/// stays out of this test: a Rust panic from one dylib cannot be caught by the
/// panic runtime of another (the same-crate test above measures the caught
/// panic). What is measured here is the selection and the tags: 11 can only be
/// produced by reading tag 1 and payload 7 from the wrapper's union.
#[test]
fn the_jit_calls_the_checked_wrapper_the_compiler_selected() {
    use austral_cranelift_bridge::{auth, compile_entry_i64, execute_function};
    use umbrella::interface::{Adapter, GeneratedAdapter};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root");
    // Prefer the tree's current build: the staged `./austral` is only as new
    // as the last `make all`.
    let built_front_end = root.join("_build/default/bin/austral.exe");
    let front_end = if built_front_end.exists() {
        built_front_end
    } else {
        root.join("austral")
    };
    assert!(
        front_end.exists(),
        "the checked-Option test needs the dune-built front-end at {}; \
         build it with `nix develop \"git+file://$PWD#ocaml\" -c make all`",
        front_end.display()
    );

    // 1. The provider: a successful lookup and one that panics.
    let dir = scratch("checked-option");
    let provider_src = dir.join("provider.rs");
    std::fs::write(
        &provider_src,
        "#[export_name = \"rs_lookup\"]\n\
         pub extern \"C-unwind\" fn lookup(flag: bool) -> i64 { if flag { 7 } else { 9 } }\n",
    )
    .expect("write provider");
    let provider = dir.join("libchecked_option_provider.so");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=checked_option_provider")
        .arg("-o")
        .arg(&provider)
        .arg(&provider_src)
        .output()
        .expect("spawn rustc for the provider");
    assert!(
        out.status.success(),
        "the provider must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 2. The real front-end over the fixture module.
    let cps_path = dir.join("CheckedOption.cps");
    let adapters_path = dir.join("CheckedOption.adapters");
    let mut command = Command::new(&front_end);
    command
        .arg("compile")
        .arg(root.join("umbrella/modules/CheckedOption.aum"))
        .arg("--use-cps-jit")
        .arg("--emit-cps-only")
        .arg("--allow-all")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cps_path.display()))
        .arg(format!("--emit-adapters={}", adapters_path.display()))
        // The umbrella's own front-end never consults the advisory opt-out;
        // `--emit-cps-only` is the mode that makes the artifact and stops.
        .env_remove("CPS_JIT_ADVISORY");
    let bridge_dir = root.join("safestos/cranelift/target/release");
    let existing = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
    let joined = if existing.is_empty() {
        format!("{}:{}", root.display(), bridge_dir.display())
    } else {
        format!(
            "{}:{}:{}",
            root.display(),
            bridge_dir.display(),
            existing.to_string_lossy()
        )
    };
    command.env("LD_LIBRARY_PATH", joined);
    let out = command.output().expect("run the front-end");
    assert!(
        out.status.success(),
        "the front-end must typecheck the fixture:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let descriptor_text = std::fs::read_to_string(&adapters_path).expect("read descriptors");
    assert!(
        descriptor_text.contains("symbol rs_lookup__adapter") &&
        descriptor_text.contains("status checked") &&
        descriptor_text.contains("failure 0") &&
        descriptor_text.contains("success 1"),
        "the compiler must name the checked wrappers and the union's own tags:\n{descriptor_text}"
    );

    // 3. Render, build and load the wrappers the descriptor names.
    let adapters = Adapter::parse_descriptors(&descriptor_text).expect("parse descriptors");
    assert_eq!(adapters.len(), 1, "one wrapper per import");
    let generated: Vec<GeneratedAdapter> =
        adapters.iter().map(GeneratedAdapter::Scalar).collect();
    let built = umbrella::adapters::build_next_against(&generated, Some(&provider))
        .expect("build the wrappers against the provider");
    assert!(
        built.exports.contains(&"rs_lookup__adapter__checked".to_string()),
        "the checked symbol must be exported, not only the payload wrapper: {:?}",
        built.exports
    );
    umbrella::adapters::load_for_jit(&built).expect("load the wrappers for the JIT");

    // 4. Compile the module's entry and run it.
    let cps = std::fs::read(&cps_path).expect("read CPS");
    auth::set_allow_all();
    let entry = b"compute";
    let function = compile_entry_i64(
        cps.as_ptr(),
        cps.len(),
        entry.as_ptr(),
        entry.len(),
        0,
    );
    assert!(
        !function.is_null(),
        "the module must compile once the checked wrappers are loaded"
    );
    let address = function as usize;
    let result = execute_function(address as *const std::ffi::c_void);
    assert_eq!(result, 11, "Some(7) read out of the wrapper's union");
}
