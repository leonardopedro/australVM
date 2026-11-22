//! U12 (OWN01/OWN02) — the buffer wrapper's *bytes*, measured.
//!
//! `interface.rs` proves a buffer wrapper compiles and `check-adapter-descriptors`
//! proves the descriptors come from the real lowering. This proves the wrapper
//! *transports bytes*: a Rust library exporting the declared targets and the
//! wrappers rendered from their descriptors are compiled into one `cdylib`, and
//! both are called through the real ABI — the target as Rust declares it
//! (`*mut u8`, `i64`), the wrapper with the two words the JIT passes.
//!
//! The cases are chosen so a plausible wrong wrapper fails:
//!
//! * a 5-byte fill into an 8-byte array checks that the wrapper passes the
//!   *declared* length and not the allocation's: bytes past it keep their
//!   canary, so "it wrote to the end of the buffer" is not confused with "it
//!   wrote `len` bytes";
//! * a checksum over `[1, 0, 2, 0, 3]` differs from the checksum of `[1, 2, 3]`,
//!   so a wrapper that stopped at the first NUL, or that copied through
//!   `CString`, is caught;
//! * the probe call passes length 0 and therefore a null pointer — the only
//!   empty convention the ABI defines — and the target must still answer
//!   without dereferencing it;
//! * a negative length and a non-zero length with a null pointer must abort
//!   *before* the target runs; both are checked in a child process, because a
//!   panic in an `extern "C"` wrapper aborts and cannot be caught in-process.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;

use austral_cranelift_bridge::adapters::{decode, ScalarType, ScalarValue};
use umbrella::interface::{
    render_buffer_wrapper, BufferAdapter, BufferMode, BufferParam, EmptyConvention, Scalar,
    BUFFER_ABI_VERSION, DESCRIPTOR_SCHEMA,
};

fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-buffers-{}-{unique}-{name}",
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

/// The three descriptors this test renders, each named the way the front-end
/// names them (`<target>__adapter`).
fn fill_adapter() -> BufferAdapter {
    BufferAdapter {
        schema: DESCRIPTOR_SCHEMA,
        abi_version: BUFFER_ABI_VERSION,
        symbol: "avm_fill__adapter".to_string(),
        target: "avm_fill".to_string(),
        params: 2,
        lengths: Scalar::I64,
        buffers: vec![BufferParam {
            mode: BufferMode::ReadWrite,
            buffer: 0,
            length: 1,
        }],
        scalars: vec![],
        empty: EmptyConvention::NullLen0,
        returns: Scalar::I32,
    }
}

fn checksum_adapter() -> BufferAdapter {
    BufferAdapter {
        schema: DESCRIPTOR_SCHEMA,
        abi_version: BUFFER_ABI_VERSION,
        symbol: "avm_checksum__adapter".to_string(),
        target: "avm_checksum".to_string(),
        params: 2,
        lengths: Scalar::I64,
        buffers: vec![BufferParam {
            mode: BufferMode::Read,
            buffer: 0,
            length: 1,
        }],
        scalars: vec![],
        empty: EmptyConvention::NullLen0,
        returns: Scalar::I64,
    }
}

fn probe_adapter() -> BufferAdapter {
    BufferAdapter {
        schema: DESCRIPTOR_SCHEMA,
        abi_version: BUFFER_ABI_VERSION,
        symbol: "avm_probe__adapter".to_string(),
        target: "avm_probe".to_string(),
        params: 2,
        lengths: Scalar::I64,
        buffers: vec![BufferParam {
            mode: BufferMode::ReadWrite,
            buffer: 0,
            length: 1,
        }],
        scalars: vec![],
        empty: EmptyConvention::NullLen0,
        returns: Scalar::I64,
    }
}

/// The declared targets, written the way Rust declares them, followed by the
/// wrappers rendered from their descriptors.
fn library_source() -> String {
    // `export_name` carries the declared symbol, not the declared Rust name: the
    // generated wrapper declares `extern "C" { fn avm_fill(..) }` in the same
    // crate, and two items may not share a name.
    let mut source = String::from(
        "// implementation of the declared targets\n\
         #[export_name = \"avm_fill\"]\n\
         pub extern \"C\" fn buffer_avm_fill(buf: *mut u8, len: i64) -> i32 {\n\
             if len == 0 { return 0; }\n\
             if buf.is_null() { return -1; }\n\
             let bytes = unsafe { std::slice::from_raw_parts_mut(buf, len as usize) };\n\
             for (i, byte) in bytes.iter_mut().enumerate() {\n\
                 *byte = (i as u8).wrapping_add(1);\n\
             }\n\
             len as i32\n\
         }\n\
         #[export_name = \"avm_checksum\"]\n\
         pub extern \"C\" fn buffer_avm_checksum(buf: *const u8, len: i64) -> i64 {\n\
             if len == 0 { return 0; }\n\
             if buf.is_null() { return -1; }\n\
             let bytes = unsafe { std::slice::from_raw_parts(buf, len as usize) };\n\
             let mut sum: i64 = 0;\n\
             for byte in bytes {\n\
                 sum = sum.wrapping_mul(31).wrapping_add(*byte as i64);\n\
             }\n\
             sum\n\
         }\n\
         #[export_name = \"avm_probe\"]\n\
         pub extern \"C\" fn buffer_avm_probe(buf: *mut u8, cap: i64) -> i64 {\n\
             const NEEDED: i64 = 8;\n\
             if cap < NEEDED || buf.is_null() { return NEEDED; }\n\
             unsafe { std::slice::from_raw_parts_mut(buf, NEEDED as usize) }.fill(b'x');\n\
             NEEDED\n\
         }\n\n",
    );
    // One module per wrapper: the generator writes a file per adapter and each
    // starts with the same `use`, so compiling them into one crate without a
    // module boundary would collide on those imports.
    for (i, adapter) in [fill_adapter(), checksum_adapter(), probe_adapter()]
        .iter()
        .enumerate()
    {
        source.push_str(&format!(
            "mod w{i} {{\n{}\n}}\n",
            render_buffer_wrapper(adapter).expect("render")
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
    let out = dir.join("libbuffers.so");
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

fn build_library() -> PathBuf {
    compile(&library_source(), "buffer_roundtrip")
}

/// One build per process: the test binary links the whole bridge, so a second
/// `rustc` of the same source is minutes of linker time for the same answer.
fn shared_library() -> PathBuf {
    use std::sync::OnceLock;
    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
    LIBRARY.get_or_init(build_library).clone()
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
        unsafe {
            libc::dlclose(self.0);
        }
    }
}

type Wrapper2 = extern "C" fn(u64, u64) -> u64;

#[test]
fn the_wrapper_transports_exactly_the_declared_length() {
    let path = shared_library();
    let lib = Library::open(&path);
    let fill: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("avm_fill__adapter")) };
    let checksum: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("avm_checksum__adapter")) };
    let probe: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("avm_probe__adapter")) };
    let fill_direct: extern "C" fn(*mut u8, i64) -> i32 =
        unsafe { std::mem::transmute(lib.symbol("avm_fill")) };
    let checksum_direct: extern "C" fn(*const u8, i64) -> i64 =
        unsafe { std::mem::transmute(lib.symbol("avm_checksum")) };

    // A declared length of 5 inside an 8-byte allocation: the canary past the
    // length is the assertion that the wrapper did not pass the allocation.
    let mut bytes = [0xAAu8; 8];
    let declared = 5i64;
    let through = fill(bytes.as_mut_ptr() as u64, declared as u64);
    assert_eq!(
        decode(ScalarType::I32, through).expect("result word"),
        ScalarValue::I32(declared as i32),
        "the target must report the length it was given"
    );
    assert_eq!(&bytes[..5], &[1, 2, 3, 4, 5], "the bytes the target wrote");
    assert_eq!(
        &bytes[5..],
        &[0xAA, 0xAA, 0xAA],
        "the wrapper must pass the declared extent, not the allocation"
    );

    // The same call made directly, compared byte for byte.
    let mut direct = [0xAAu8; 8];
    let direct_count = fill_direct(direct.as_mut_ptr(), declared);
    assert_eq!(
        direct_count, declared as i32,
        "the direct call reports the same count"
    );
    assert_eq!(bytes, direct, "wrapper and direct call must agree");

    // Embedded NULs are data, not terminators: read back what the fill wrote
    // (which contains no NUL) and then a buffer that does.
    let with_nul = [1u8, 0, 2, 0, 3];
    let direct_sum = checksum_direct(with_nul.as_ptr(), 5);
    let through_sum = checksum(with_nul.as_ptr() as u64, 5);
    assert_eq!(
        decode(ScalarType::I64, through_sum).expect("result word"),
        ScalarValue::I64(direct_sum),
        "the wrapper must carry the bytes a C string would stop at"
    );
    let without_nul = [1u8, 2, 3];
    let direct_stripped = checksum_direct(without_nul.as_ptr(), 3);
    assert_ne!(
        direct_sum, direct_stripped,
        "the checksum must be sensitive to the NULs for this case to discriminate"
    );

    // The empty convention: length 0 means empty, and only then may the pointer
    // be null. The probe call is the protocol this shape exists for.
    let through_probe = probe(0, 0);
    assert_eq!(
        decode(ScalarType::I64, through_probe).expect("result word"),
        ScalarValue::I64(8),
        "a probe (length 0, null pointer) must still reach the target"
    );
    // A real pointer with length 0 is still the empty buffer, so the wrapper
    // passes null rather than the address; the target answers the probe.
    let mut spare = [0u8; 8];
    let through_probe = probe(spare.as_mut_ptr() as u64, 0);
    assert_eq!(
        decode(ScalarType::I64, through_probe).expect("result word"),
        ScalarValue::I64(8)
    );
    assert_eq!(spare, [0u8; 8], "a zero-length call writes nothing");
}

/// The refusal paths, each in a child process: the wrapper panics inside an
/// `extern "C"` function, which aborts rather than unwinding, and the point is
/// that it aborts *before* the target sees the bad extent.
#[test]
fn malformed_lengths_abort_before_the_target_runs() {
    // Built here once and handed to the children: each of them would otherwise
    // relink the whole bridge to reach the same bytes.
    let library = shared_library();
    for (case, needle) in [
        ("negative", "does not fit a signed 64-bit extent"),
        ("null-with-length", "with a null pointer"),
    ] {
        let out = Command::new(std::env::current_exe().expect("current exe"))
            .arg("--exact")
            .arg("buffer_trap_child_exit")
            .arg("--nocapture")
            .env("AVM_BUFFER_TRAP", case)
            .env("AVM_BUFFER_LIBRARY", &library)
            .output()
            .expect("re-exec the test binary");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success(),
            "case {case} must not return normally; stdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        );
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(
                out.status.signal(),
                Some(6),
                "case {case} must abort (SIGABRT); stderr:\n{stderr}"
            );
        }
        assert!(
            stderr.contains(needle),
            "case {case} must say `{needle}`; stderr:\n{stderr}"
        );
    }
}

/// The live path: a buffer call from a *compiled Austral module* through the
/// JIT. The module (`umbrella/modules/BufferLive.aum`) allocates a Nat8 buffer
/// with `allocateBuffer` — the `au_calloc` intrinsic, a real JIT symbol —
/// passes it with a `Nat64` length to three `rs_*` targets, and returns 170
/// only if the second call reads back exactly the four bytes the first wrote
/// and the third returns that same sum as an `Int64`.
///
/// It is a different claim from the tests above: those call a rendered wrapper
/// directly from Rust, so they measure the wrapper; this one measures the whole
/// chain — the front-end naming `rs_live_fill__adapter`, the driver generating
/// and loading that wrapper, the JIT resolving the mediated call, and the
/// wrapper marshalling a pointer the *module* allocated.
///
/// The oracle is inside the run: `rs_live_fill` returns -1 if the byte past the
/// declared extent is not the allocation's zero, so a slice built from the
/// allocation size instead of the declared length yields RESULT 0 rather than
/// 170.
///
/// The third call is the one that used to bypass the wrapper. Its signature is
/// all 64-bit words (`Address`, `Nat64`, `Int64` result), which was word-native
/// and therefore called `rs_live_wide` directly; buffer calls are now mediated
/// unconditionally. The manifest grants `rs_live_wide__adapter` and **not**
/// `rs_live_wide`, so a regression to the direct symbol is refused by the JIT
/// gate instead of passing quietly — the grant list is this test's discriminator
/// for that half.
#[test]
fn the_jit_executes_a_buffer_call_from_a_compiled_module() {
    let front_end = Path::new(env!("CARGO_MANIFEST_DIR")).join("../austral");
    assert!(
        front_end.exists(),
        "the live buffer lane needs the dune-built front-end at {}; \
         build it with `nix develop \"git+file://$PWD#ocaml\" -c make all`",
        front_end.display()
    );

    let crate_dir = scratch("live-crate");
    std::fs::create_dir_all(crate_dir.join("src")).expect("crate dir");
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\n\
         name = \"buffer_live\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\n\
         [lib]\n\
         crate-type = [\"cdylib\"]\n",
    )
    .expect("crate manifest");
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        "#[no_mangle]\n\
         pub extern \"C\" fn rs_live_fill(buf: *mut u8, len: u64) -> i32 {\n\
             let n = len as usize;\n\
             let out = unsafe { std::slice::from_raw_parts_mut(buf, n) };\n\
             for (i, b) in out.iter_mut().enumerate() {\n\
                 *b = ((i + 1) as u8) * 0x11;\n\
             }\n\
             // The allocation is zeroed and the declared extent is 4 of 8: a\n\
             // wrapper that passed the allocation size would overwrite this.\n\
             if unsafe { *buf.add(n) } != 0 { return -1; }\n\
             len as i32\n\
         }\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_live_checksum(buf: *const u8, len: u64) -> i32 {\n\
             let n = len as usize;\n\
             let bytes = unsafe { std::slice::from_raw_parts(buf, n) };\n\
             bytes.iter().map(|b| *b as i32).sum()\n\
         }\n\
         // All-64-bit words beside a buffer: word-native, and still mediated.\n\
         #[no_mangle]\n\
         pub extern \"C\" fn rs_live_wide(buf: *const u8, len: u64) -> i64 {\n\
             let n = len as usize;\n\
             let bytes = unsafe { std::slice::from_raw_parts(buf, n) };\n\
             bytes.iter().map(|b| *b as i64).sum()\n\
         }\n",
    )
    .expect("crate source");

    let manifest = scratch("live-manifest").join("module.toml");
    std::fs::write(
        &manifest,
        "[module]\n\
         name = \"BufferLive\"\n\n\
         [grants]\n\
         kernel = [\"rs_live_fill__adapter\", \"rs_live_checksum__adapter\", \"rs_live_wide__adapter\"]\n",
    )
    .expect("manifest");

    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("modules/BufferLive.aum");
    let out = Command::new(env!("CARGO_BIN_EXE_umbrella"))
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
        .arg("170")
        .arg(&module)
        .output()
        .expect("run umbrella");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "umbrella failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(stdout.trim(), "RESULT 170", "{stderr}");
    // The wrappers were generated from the emitted descriptors, not found by
    // accident: the driver names both it built and loaded.
    assert!(
        stderr.contains("built and loaded 3 adapter wrapper(s)"),
        "the build must generate and load the buffer wrappers; stderr:\n{stderr}"
    );
    for symbol in [
        "rs_live_fill__adapter",
        "rs_live_checksum__adapter",
        "rs_live_wide__adapter",
    ] {
        assert!(
            stderr.contains(symbol),
            "stderr must name {symbol}:\n{stderr}"
        );
    }
}

/// The child half of the test above: no `AVM_BUFFER_TRAP` means it is an
/// ordinary (and empty) run, so a stray direct invocation cannot pass for the
/// refusal check.
#[test]
fn buffer_trap_child_exit() {
    let Ok(case) = std::env::var("AVM_BUFFER_TRAP") else {
        return;
    };
    let path = std::env::var_os("AVM_BUFFER_LIBRARY")
        .expect("the parent must pass the library it already built");
    let lib = Library::open(Path::new(&path));
    let fill: Wrapper2 = unsafe { std::mem::transmute(lib.symbol("avm_fill__adapter")) };
    let mut bytes = [0u8; 8];
    match case.as_str() {
        "negative" => {
            fill(bytes.as_mut_ptr() as u64, u64::MAX);
        }
        "null-with-length" => {
            fill(0, 4);
        }
        other => panic!("unknown AVM_BUFFER_TRAP case {other}"),
    }
    panic!("the wrapper must have aborted before returning");
}
