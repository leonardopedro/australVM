//! U15b — the named two-case union, measured.
//!
//! `Either[L, R]` carries a payload in both cases, so a checked wrapper cannot
//! signal a refusal with an empty failure case. Its provider writes `[tag,
//! payload]` into an out pair; the wrapper validates the tag and payload and
//! returns the union pointer the JIT's `match` reads. This test measures the
//! descriptor (names, tags, payload types), the wrapper's refusal and both
//! successful cases, and the real end-to-end path: front-end → descriptor →
//! wrapper → JIT, where 11 and 22 are only reachable by reading the right tag.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;

use austral_cranelift_bridge::adapters::{encode, ScalarType, ScalarValue};
use umbrella::interface::{
    Adapter, GeneratedAdapter, Scalar,
};

fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-either-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
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

    fn checked_fn(&self, name: &str) -> unsafe extern "C" fn(u64) -> *mut u64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }

    fn free_fn(&self, name: &str) -> unsafe extern "C" fn(*mut u64) {
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

fn union_adapter() -> Adapter {
    let mut adapter = Adapter::new(
        "rs_choose__adapter",
        "rs_choose",
        vec![Scalar::Bool],
        Scalar::I64,
    );
    adapter.checked = true;
    adapter.checked_tags = Some((0, 1));
    adapter.checked_union = Some(umbrella::interface::CheckedUnion {
        union_name: "Either".to_string(),
        failure_case: "Left".to_string(),
        success_case: "Right".to_string(),
        failure_payload: Scalar::I64,
    });
    adapter
}

/// The descriptor itself is the contract: the canonical text names the union,
/// both cases and the failure payload's type, and it round-trips. The tag test
/// is the point — `Left` is index 0 and `Right` is index 1, the order the
/// union declares, and both tags are distinct and within the union.
#[test]
fn the_either_descriptor_names_both_cases_and_their_tags() {
    let adapter = union_adapter();
    let text = adapter.descriptor_text();
    assert_eq!(
        text,
        "schema 1\nabi scalar 1\nsymbol rs_choose__adapter\ntarget rs_choose\n\
         params Bool\nreturns Int64\nstatus checked\nfailure 0\nsuccess 1\n\
         union Either\nfailure-case Left\nsuccess-case Right\nfailure-payload Int64\n"
    );
    let parsed = Adapter::parse_descriptors(&text).expect("parse the union descriptor");
    assert_eq!(parsed.len(), 1);
    let parsed = &parsed[0];
    assert_eq!(parsed.checked_tags, Some((0, 1)), "Left 0, Right 1");
    let union = parsed.checked_union.as_ref().expect("the union block");
    assert_eq!(union.union_name, "Either");
    assert_eq!(union.failure_case, "Left");
    assert_eq!(union.success_case, "Right");
    assert_eq!(union.failure_payload, Scalar::I64);
    assert_eq!(parsed.descriptor_text(), text, "canonical round-trip");

    // A partial union block is refused rather than guessed.
    let partial = "schema 1\nabi scalar 1\nsymbol s\ntarget t\nparams Bool\nreturns Int64\n\
                   status checked\nfailure 0\nsuccess 1\nunion Either\nfailure-case Left\n\
                   failure-payload Int64\n";
    assert!(
        Adapter::parse_descriptors(partial).is_err(),
        "a union without both case names must be refused"
    );

    // The provider's out pair means the wrapper has no payload-only export.
    let generated = GeneratedAdapter::Scalar(&adapter);
    assert_eq!(
        generated.exported_symbols(),
        vec![
            "rs_choose__adapter__checked".to_string(),
            "rs_choose__adapter__free".to_string(),
        ],
        "the checked union and the release of the union it allocates"
    );
    let rendered = generated.render().expect("render the union wrapper");
    assert!(rendered.contains("out: *mut u64"), "the provider writes the tag");
    assert!(rendered.contains("static REFUSAL: [u64; 2] = [0, 0];"));
    assert!(rendered.contains("if tag == 0"));
    assert!(rendered.contains("if tag == 1"));
    assert!(
        rendered.contains("pub extern \"C\" fn rs_choose__adapter__free"),
        "the module needs a way to release the union"
    );
    assert!(
        rendered.contains("value == refusal()"),
        "the shared refusal value is static and must not be freed"
    );
}

/// Both successful cases and every refusal path through the real ABI.
#[test]
fn the_either_wrapper_validates_the_provider_tag_and_payload() {
    let dir = scratch("wrapper");
    let provider_src = dir.join("provider.rs");
    std::fs::write(
        &provider_src,
        r#"
#[export_name = "rs_choose"]
pub extern "C-unwind" fn choose(flag: u64, out: *mut u64) -> i64 {
    unsafe {
        if flag != 0 {
            *out = 1;
            *out.add(1) = 7;
        } else {
            *out = 0;
            *out.add(1) = 9;
        }
    }
    0
}

#[export_name = "rs_choose_bad_tag"]
pub extern "C-unwind" fn bad_tag(_flag: bool, out: *mut u64) -> i64 {
    unsafe {
        *out = 2;
        *out.add(1) = 7;
    }
    0
}

#[export_name = "rs_choose_bad_payload"]
pub extern "C-unwind" fn bad_payload(_flag: bool, out: *mut u64) -> i64 {
    unsafe {
        *out = 1;
        *out.add(1) = 2; // not a Bool-like word for Int64? it decodes; see test note
    }
    0
}

#[export_name = "rs_choose_refuse"]
pub extern "C-unwind" fn refuse(_flag: bool, _out: *mut u64) -> i64 { 5 }
"#,
    )
    .expect("write provider");

    let mut bad_tag = union_adapter();
    bad_tag.symbol = "rs_choose_bad_tag__adapter".to_string();
    bad_tag.target = "rs_choose_bad_tag".to_string();
    let mut refuse = union_adapter();
    refuse.symbol = "rs_choose_refuse__adapter".to_string();
    refuse.target = "rs_choose_refuse".to_string();
    // A bool payload word of 2 cannot decode as Bool, which is the payload
    // check for a `Scalar::Bool` failure case.
    let mut bool_payload = union_adapter();
    bool_payload.symbol = "rs_choose_bad_payload__adapter".to_string();
    bool_payload.target = "rs_choose_bad_payload".to_string();
    bool_payload.returns = Scalar::Bool;
    bool_payload.checked_union.as_mut().expect("union").failure_payload = Scalar::Bool;

    // A distinct symbol from the end-to-end test's `rs_choose__adapter`: both
    // tests load into one process, and `load_for_jit` refuses a second
    // registration of the same export at another address.
    let mut adapter = union_adapter();
    adapter.symbol = "rs_choose_unit__adapter".to_string();
    let generated: Vec<GeneratedAdapter> = [&adapter, &bad_tag, &refuse, &bool_payload]
        .iter()
        .map(|a| GeneratedAdapter::Scalar(a))
        .collect();
    let provider = dir.join("libeither_provider.so");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=either_provider")
        .arg("-o")
        .arg(&provider)
        .arg(&provider_src)
        .output()
        .expect("spawn rustc");
    assert!(
        out.status.success(),
        "the provider must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let built = umbrella::adapters::build_next_against(&generated, Some(&provider))
        .expect("build the union wrappers");
    umbrella::adapters::load_for_jit(&built).expect("load the union wrappers");
    let lib = Library::open(&built.path);
    let choose = lib.checked_fn(&format!("{}__checked", adapter.symbol));
    let tag_bad = lib.checked_fn("rs_choose_bad_tag__adapter__checked");
    let refused = lib.checked_fn("rs_choose_refuse__adapter__checked");
    let payload_bad = lib.checked_fn("rs_choose_bad_payload__adapter__checked");
    let free = lib.free_fn(&format!("{}__free", adapter.symbol));
    // Each generated wrapper has its own refusal static, so the release that
    // recognises it is that wrapper's own `__free`.
    let free_refused = lib.free_fn("rs_choose_refuse__adapter__free");
    let union = |ptr: *mut u64| unsafe { [*ptr, *ptr.add(1)] };

    let right = union(unsafe { choose(1) });
    assert_eq!(right[0], 1, "Right is tag 1");
    assert_eq!(
        right[1],
        encode(ScalarType::I64, ScalarValue::I64(7)).expect("encode the right payload")
    );
    let left = union(unsafe { choose(0) });
    assert_eq!(left[0], 0, "Left is tag 0");
    assert_eq!(
        left[1],
        encode(ScalarType::I64, ScalarValue::I64(9)).expect("encode the left payload")
    );
    let bad = union(unsafe { tag_bad(1) });
    assert_eq!(bad[0], 0, "an unknown tag is the failure case");
    assert_eq!(bad[1], 0, "a refused tag carries no payload");
    let nonzero = union(unsafe { refused(1) });
    assert_eq!(nonzero[0], 0, "a non-zero status is the failure case");
    assert_eq!(nonzero[1], 0);
    let undecodable = union(unsafe { payload_bad(1) });
    assert_eq!(undecodable[0], 0, "an undecodable payload is the failure case");
    assert_eq!(undecodable[1], 0);

    // The release path the module's `match` lowering calls: an answered call is
    // an allocation (twice, so the allocator would reuse the address if the
    // first were not freed), and the shared refusal value must be accepted
    // without being freed.
    let answered = unsafe { choose(1) };
    unsafe { free(answered) };
    let again = unsafe { choose(1) };
    unsafe { free(again) };
    // The shared refusal value is static, and the wrapper accepts it without
    // freeing it; a null pointer is likewise a no-op.
    let refusal = unsafe { refused(1) };
    unsafe { free_refused(refusal) };
    unsafe { free(std::ptr::null_mut()) };
}

/// End-to-end: the typer admits `Either[Int64, Int64]` for an `rs_*` import,
/// the descriptor names `Either`/`Left`/`Right` with tags 0/1 and the failure
/// payload type, the generated wrapper is loaded, and the JIT runs a real
/// `match`: `computeRight` returns 11 only for tag 1 + payload 7, `computeLeft`
/// returns 22 only for tag 0 + payload 9.
#[test]
fn the_jit_reads_both_cases_of_the_either_wrapper() {
    use austral_cranelift_bridge::{auth, compile_entry_i64, execute_function};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root");
    let built_front_end = root.join("_build/default/bin/austral.exe");
    let front_end = if built_front_end.exists() {
        built_front_end
    } else {
        root.join("austral")
    };
    assert!(
        front_end.exists(),
        "the Either test needs the dune-built front-end at {}",
        front_end.display()
    );

    let dir = scratch("e2e");
    let provider_src = dir.join("provider.rs");
    std::fs::write(
        &provider_src,
        r#"
// The provider writes the descriptor's tags, not Rust's `Either` order:
// this module's layout puts Right at 0 and Left at 1, so success is 0 and
// failure is 1.
#[export_name = "rs_choose"]
pub extern "C-unwind" fn choose(flag: u64, out: *mut u64) -> i64 {
    unsafe {
        if flag != 0 {
            *out = 0;
            *out.add(1) = 7;
        } else {
            *out = 1;
            *out.add(1) = 9;
        }
    }
    0
}
"#,
    )
    .expect("write provider");
    let provider = dir.join("libeither_e2e_provider.so");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=either_e2e_provider")
        .arg("-o")
        .arg(&provider)
        .arg(&provider_src)
        .output()
        .expect("spawn rustc");
    assert!(
        out.status.success(),
        "the provider must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let cps_path = dir.join("EitherImport.cps");
    let adapters_path = dir.join("EitherImport.adapters");
    let mut command = Command::new(&front_end);
    command
        .arg("compile")
        .arg(root.join("umbrella/modules/EitherImport.aum"))
        .arg("--use-cps-jit")
        .arg("--emit-cps-only")
        .arg("--allow-all")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cps_path.display()))
        .arg(format!("--emit-adapters={}", adapters_path.display()))
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
        descriptor_text.contains("symbol rs_choose__adapter")
            && descriptor_text.contains("status checked")
            // This module's own `Either` monomorph lists Right before Left,
            // so the JIT's layout is Right=0, Left=1; the descriptor must
            // name those indices, not the declaration's order.
            && descriptor_text.contains("failure 1")
            && descriptor_text.contains("success 0")
            && descriptor_text.contains("union Either")
            && descriptor_text.contains("failure-case Left")
            && descriptor_text.contains("success-case Right")
            && descriptor_text.contains("failure-payload Int64"),
        "the compiler must name the union, its cases and the failure payload:\n{descriptor_text}"
    );
    let adapters = Adapter::parse_descriptors(&descriptor_text).expect("parse descriptors");
    assert_eq!(adapters.len(), 1);
    assert_eq!(adapters[0].checked_tags, Some((1, 0)));
    assert_eq!(
        adapters[0]
            .checked_union
            .as_ref()
            .map(|u| u.failure_payload),
        Some(Scalar::I64)
    );
    let generated: Vec<GeneratedAdapter> =
        adapters.iter().map(GeneratedAdapter::Scalar).collect();
    let built = umbrella::adapters::build_next_against(&generated, Some(&provider))
        .expect("build the union wrapper against the provider");
    assert!(
        built
            .exports
            .contains(&"rs_choose__adapter__checked".to_string()),
        "the checked symbol must be exported: {:?}",
        built.exports
    );
    umbrella::adapters::load_for_jit(&built).expect("load the wrapper for the JIT");

    let cps = std::fs::read(&cps_path).expect("read CPS");
    assert!(
        cps.windows("rs_choose__adapter__free".len())
            .any(|window| window == b"rs_choose__adapter__free"),
        "the `match` lowering must release the union the checked wrapper allocates"
    );
    auth::set_allow_all();
    let entry = |name: &str| {
        let function = compile_entry_i64(
            cps.as_ptr(),
            cps.len(),
            name.as_ptr(),
            name.len(),
            0,
        );
        assert!(!function.is_null(), "`{name}` must compile");
        execute_function(function as *const std::ffi::c_void)
    };
    assert_eq!(entry("computeRight"), 11, "Right(7) read out of the wrapper's union");
    assert_eq!(
        entry("computeLeft"),
        22,
        "Left(9) read out of the wrapper's union"
    );
    assert_eq!(
        entry("computeThrough"),
        11,
        "the callee read Right(7) from the union it was passed"
    );
}

/// The Result shape through the same transport: `Result[Int64, Int64]` has a
/// payload-bearing `Err` case, so the typer admits it exactly like `Either`.
/// The descriptor names `Result`/`Err`/`Ok`, the provider writes the tags the
/// layout reports, and the JIT reads both cases. The CPS must also name
/// `rs_parse__adapter__free`, the release the `match` lowering emits.
#[test]
fn the_jit_reads_both_cases_of_the_result_wrapper() {
    use austral_cranelift_bridge::{auth, compile_entry_i64, execute_function};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root");
    let built_front_end = root.join("_build/default/bin/austral.exe");
    let front_end = if built_front_end.exists() {
        built_front_end
    } else {
        root.join("austral")
    };
    assert!(
        front_end.exists(),
        "the Result test needs the dune-built front-end at {}",
        front_end.display()
    );

    let dir = scratch("result-e2e");
    let cps_path = dir.join("ResultImport.cps");
    let adapters_path = dir.join("ResultImport.adapters");
    let mut command = Command::new(&front_end);
    command
        .arg("compile")
        .arg(root.join("umbrella/modules/ResultImport.aum"))
        .arg("--use-cps-jit")
        .arg("--emit-cps-only")
        .arg("--allow-all")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cps_path.display()))
        .arg(format!("--emit-adapters={}", adapters_path.display()))
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
        "the front-end must typecheck the Result fixture:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let descriptor_text = std::fs::read_to_string(&adapters_path).expect("read descriptors");
    assert!(
        descriptor_text.contains("symbol rs_parse__adapter")
            && descriptor_text.contains("status checked")
            && descriptor_text.contains("union Result")
            && descriptor_text.contains("failure-case Err")
            && descriptor_text.contains("success-case Ok")
            && descriptor_text.contains("failure-payload Int64"),
        "the compiler must name the Result union, its cases and the failure payload:\n{descriptor_text}"
    );
    let adapters = Adapter::parse_descriptors(&descriptor_text).expect("parse descriptors");
    assert_eq!(adapters.len(), 1);
    let (failure_tag, success_tag) = adapters[0].checked_tags.expect("tags");
    assert_ne!(failure_tag, success_tag);
    assert_eq!(
        adapters[0].checked_union.as_ref().map(|u| u.union_name.as_str()),
        Some("Result")
    );

    // The provider writes the tags the layout reported, not Rust's enum order.
    let provider_src = dir.join("provider.rs");
    std::fs::write(
        &provider_src,
        format!(
            r#"
#[export_name = "rs_parse"]
pub extern "C-unwind" fn parse(flag: u64, out: *mut u64) -> i64 {{
    unsafe {{
        if flag != 0 {{
            *out = {success_tag};
            *out.add(1) = 7;
        }} else {{
            *out = {failure_tag};
            *out.add(1) = 9;
        }}
    }}
    0
}}
"#
        ),
    )
    .expect("write provider");
    let provider = dir.join("libresult_e2e_provider.so");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=result_e2e_provider")
        .arg("-o")
        .arg(&provider)
        .arg(&provider_src)
        .output()
        .expect("spawn rustc");
    assert!(
        out.status.success(),
        "the provider must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let generated: Vec<GeneratedAdapter> =
        adapters.iter().map(GeneratedAdapter::Scalar).collect();
    let built = umbrella::adapters::build_next_against(&generated, Some(&provider))
        .expect("build the Result wrapper against the provider");
    assert!(
        built
            .exports
            .contains(&"rs_parse__adapter__checked".to_string())
            && built
                .exports
                .contains(&"rs_parse__adapter__free".to_string()),
        "the checked symbol and its release must be exported: {:?}",
        built.exports
    );
    umbrella::adapters::load_for_jit(&built).expect("load the wrapper for the JIT");

    let cps = std::fs::read(&cps_path).expect("read CPS");
    assert!(
        cps.windows("rs_parse__adapter__free".len())
            .any(|window| window == b"rs_parse__adapter__free"),
        "the `match` lowering must release the Result union it allocates"
    );
    auth::set_allow_all();
    let entry = |name: &str| {
        let function = compile_entry_i64(
            cps.as_ptr(),
            cps.len(),
            name.as_ptr(),
            name.len(),
            0,
        );
        assert!(!function.is_null(), "`{name}` must compile");
        execute_function(function as *const std::ffi::c_void)
    };
    assert_eq!(entry("computeOk"), 11, "Ok(7) read out of the wrapper's union");
    assert_eq!(
        entry("computeErr"),
        22,
        "Err(9) read out of the wrapper's union"
    );
}
