//! U15c — the checked union's release path, measured with an allocation
//! counter rather than read off the CPS.
//!
//! The wrapper allocates one `Box<[u64; 2]>` per answered call and exports
//! `<symbol>__free`. Nothing outside that crate can see those allocations, so
//! this test builds the *same* generated wrapper in a crate that installs a
//! counting global allocator and exports its counters. Then:
//!
//! - 2000 `__checked` + `__free` pairs allocate and free exactly 2000 times and
//!   leave no live bytes (the release actually returns the allocation);
//! - 500 unreleased calls are visible as 500 live allocations of two words,
//!   and releasing them afterwards returns the counter to its baseline (the
//!   counter measures the wrapper, not the allocator's bookkeeping);
//! - the JIT runs `computeThrough`, which passes a checked union to `consume`;
//!   the callee's `match` releases it, so 5000 calls allocate 5000 times, free
//!   5000 times and leave no live bytes. A release after the case dispatch
//!   would be skipped by the case's `return` and this counter would grow by
//!   5000 allocations.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::interface::{Adapter, GeneratedAdapter};

/// The bridge's JIT is process-global, and these tests load providers by
/// symbol name into it. Two of them in one process must not interleave.
static JIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-either-alloc-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// The bridge rlib the generated wrapper compiles against: the same lookup
/// `umbrella::adapters` uses, because the wrapper calls its `encode`/`decode`.
fn bridge_rlib() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let profile = exe.parent().expect("profile dir").to_path_buf();
    let mut found = Vec::new();
    for dir in [profile.join("deps"), profile] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("libaustral_cranelift_bridge") && name.ends_with(".rlib") {
                found.push(path);
            }
        }
    }
    found.sort();
    found.pop().expect("no bridge rlib next to the test executable")
}

/// Compile `fixture` through the compile-only front-end and return its CPS and
/// adapter descriptor paths.
fn compile_fixture(root: &Path, fixture: &str, dir: &Path) -> (PathBuf, PathBuf) {
    let stem = Path::new(fixture)
        .file_stem()
        .expect("fixture stem")
        .to_string_lossy()
        .into_owned();
    compile_modules(root, &[fixture], dir, &stem)
}

/// Compile a whole module list in one front-end invocation (dependency order),
/// tagging the staged paths so prefix runs do not overwrite each other.
fn compile_modules(
    root: &Path,
    modules: &[&str],
    dir: &Path,
    tag: &str,
) -> (PathBuf, PathBuf) {
    let built_front_end = root.join("_build/default/bin/austral.exe");
    let front_end = if built_front_end.exists() {
        built_front_end
    } else {
        root.join("austral")
    };
    assert!(
        front_end.exists(),
        "the allocation test needs the dune-built front-end at {}",
        front_end.display()
    );
    let cps_path = dir.join(format!("{tag}.cps"));
    let adapters_path = dir.join(format!("{tag}.adapters"));
    let mut command = Command::new(&front_end);
    command.arg("compile");
    for module in modules {
        // An interface/body pair is one positional argument, comma-separated.
        let joined = module
            .split(',')
            .map(|part| root.join(part).display().to_string())
            .collect::<Vec<_>>()
            .join(",");
        command.arg(joined);
    }
    command
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
    (cps_path, adapters_path)
}

/// The counting crate: the generated wrapper plus a provider that answers with
/// Right(7)/Left(9) and counters the allocator sees.
fn counted_crate_source(wrapper: &str, symbol: &str) -> String {
    format!(
        r#"
use std::alloc::{{GlobalAlloc, Layout, System}};
use std::sync::atomic::{{AtomicI64, Ordering}};

static LIVE: AtomicI64 = AtomicI64::new(0);
static ALLOCS: AtomicI64 = AtomicI64::new(0);
static FREES: AtomicI64 = AtomicI64::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {{
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {{
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        LIVE.fetch_add(layout.size() as i64, Ordering::Relaxed);
        System.alloc(layout)
    }}
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {{
        FREES.fetch_add(1, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }}
}}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[no_mangle]
pub extern "C" fn counted_live() -> i64 {{ LIVE.load(Ordering::Relaxed) }}
#[no_mangle]
pub extern "C" fn counted_allocs() -> i64 {{ ALLOCS.load(Ordering::Relaxed) }}
#[no_mangle]
pub extern "C" fn counted_frees() -> i64 {{ FREES.load(Ordering::Relaxed) }}

#[export_name = "{symbol}"]
pub extern "C-unwind" fn choose(flag: u64, out: *mut u64) -> i64 {{
    unsafe {{
        if flag != 0 {{
            *out = 0; // Right
            *out.add(1) = 7;
        }} else {{
            *out = 1; // Left
            *out.add(1) = 9;
        }}
    }}
    0
}}

mod w0 {{
{wrapper}
}}
"#
    )
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

    fn counter(&self, name: &str) -> unsafe extern "C" fn() -> i64 {
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
fn the_free_path_balances_the_allocations_it_releases() {
    use austral_cranelift_bridge::{auth, compile_entry_i64, execute_function};
    let _jit = JIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root");

    // 1. The compiler's own descriptor for the fixture, exactly as the live
    //    end-to-end test reads it.
    let dir = scratch("jit");
    let (cps_path, adapters_path) =
        compile_fixture(root, "umbrella/modules/EitherImport.aum", &dir);

    let descriptor_text = std::fs::read_to_string(&adapters_path).expect("read descriptors");
    let adapters = Adapter::parse_descriptors(&descriptor_text).expect("parse descriptors");
    assert_eq!(adapters.len(), 1, "one checked adapter in the fixture");
    let adapter = &adapters[0];
    assert_eq!(adapter.checked_tags, Some((1, 0)), "Left 1, Right 0");
    let generated = GeneratedAdapter::Scalar(adapter);
    let wrapper = generated.render().expect("render the checked union wrapper");

    // 2. The same wrapper in a crate whose allocator counts allocations. The
    //    provider writes the descriptor's tags: Right is 0, Left is 1.
    let source = counted_crate_source(&wrapper, "rs_choose");
    let crate_source = dir.join("counting_wrapper.rs");
    std::fs::write(&crate_source, source).expect("write the counting crate");
    let counted = dir.join("libeither_counted.so");
    let rlib = bridge_rlib();
    let deps = rlib.parent().expect("rlib directory");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=either_counted")
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("austral_cranelift_bridge={}", rlib.display()))
        .arg("-o")
        .arg(&counted)
        .arg(&crate_source)
        .output()
        .expect("spawn rustc");
    assert!(
        out.status.success(),
        "the counting wrapper must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lib = Library::open(&counted);
    let checked = lib.checked_fn("rs_choose__adapter__checked");
    let free = lib.free_fn("rs_choose__adapter__free");
    let live = lib.counter("counted_live");
    let allocs = lib.counter("counted_allocs");
    let frees = lib.counter("counted_frees");

    // 3. Balanced calls: one allocation and one release each, no live bytes.
    let base_live = unsafe { live() };
    let base_allocs = unsafe { allocs() };
    let base_frees = unsafe { frees() };
    for _ in 0..2000 {
        let union = unsafe { checked(1) };
        assert!(!union.is_null(), "an answered call allocates");
        unsafe { free(union) };
    }
    assert_eq!(
        unsafe { allocs() } - base_allocs,
        2000,
        "one Box per answered call"
    );
    assert_eq!(
        unsafe { frees() } - base_frees,
        2000,
        "one release per answered call"
    );
    assert_eq!(unsafe { live() }, base_live, "no live allocation after the loop");

    // 4. The counter can see a leak: 500 unreleased unions are 500 live
    //    two-word allocations, and releasing them returns to the baseline.
    let mut held = [std::ptr::null_mut(); 500];
    for slot in held.iter_mut() {
        *slot = unsafe { checked(1) };
    }
    assert_eq!(
        unsafe { allocs() } - base_allocs,
        2500,
        "500 more allocations"
    );
    assert_eq!(
        unsafe { live() } - base_live,
        500 * 16,
        "500 live Box<[u64; 2]> allocations"
    );
    for union in held {
        unsafe { free(union) };
    }
    assert_eq!(
        unsafe { live() },
        base_live,
        "releasing every held union returns to the baseline"
    );

    // 5. The module path: `computeThrough` passes the checked union to
    //    `consume`, whose `match` releases it. One allocation and one release
    //    per call, and nothing live afterwards.
    let exports = generated.exported_symbols();
    let generation = umbrella::interface::next_provider_generation();
    umbrella::adapters::load_for_jit(&umbrella::adapters::AdapterCrate {
        path: counted.clone(),
        generation,
        exports,
        retire_symbols: Vec::new(),
        drain_symbols: Vec::new(),
    })
    .expect("load the counting wrapper for the JIT");
    auth::set_allow_all();
    let cps = std::fs::read(&cps_path).expect("read CPS");
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
    assert_eq!(entry("computeThrough"), 11, "Right(7) read by the callee");

    let jit_live = unsafe { live() };
    let jit_allocs = unsafe { allocs() };
    let jit_frees = unsafe { frees() };
    for _ in 0..5000 {
        assert_eq!(entry("computeThrough"), 11);
    }
    assert_eq!(
        unsafe { allocs() } - jit_allocs,
        5000,
        "one allocation per transferred checked call"
    );
    assert_eq!(
        unsafe { frees() } - jit_frees,
        5000,
        "the callee's `match` releases the union it was passed"
    );
    assert_eq!(
        unsafe { live() },
        jit_live,
        "no live allocation after 5000 transferred calls"
    );
}

/// U15c's other call shape: the union is passed to a **concrete instance
/// method** (`Sink(Int64)`'s `send`), not to a free function. The method is
/// declared in the fixture's module, so its body is emitted and its `match`
/// releases the union the call transferred into its parameter.
#[test]
fn a_concrete_method_releases_the_union_it_is_passed() {
    use austral_cranelift_bridge::{auth, compile_entry_i64, execute_function};
    let _jit = JIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root");
    let dir = scratch("method");
    let (cps_path, adapters_path) =
        compile_fixture(root, "umbrella/modules/EitherMethod.aum", &dir);

    let descriptor_text = std::fs::read_to_string(&adapters_path).expect("read descriptors");
    let adapters = Adapter::parse_descriptors(&descriptor_text).expect("parse descriptors");
    assert_eq!(adapters.len(), 1, "one checked adapter in the fixture");
    assert_eq!(adapters[0].checked_tags, Some((1, 0)), "Left 1, Right 0");
    let wrapper = GeneratedAdapter::Scalar(&adapters[0])
        .render()
        .expect("render the checked union wrapper");

    let source = counted_crate_source(&wrapper, "rs_choose_method");
    let crate_source = dir.join("counting_method.rs");
    std::fs::write(&crate_source, source).expect("write the counting crate");
    let counted = dir.join("libeither_method_counted.so");
    let rlib = bridge_rlib();
    let deps = rlib.parent().expect("rlib directory");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=either_method_counted")
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("austral_cranelift_bridge={}", rlib.display()))
        .arg("-o")
        .arg(&counted)
        .arg(&crate_source)
        .output()
        .expect("spawn rustc");
    assert!(
        out.status.success(),
        "the counting wrapper must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lib = Library::open(&counted);
    let live = lib.counter("counted_live");
    let allocs = lib.counter("counted_allocs");
    let frees = lib.counter("counted_frees");
    let generated = GeneratedAdapter::Scalar(&adapters[0]);
    let generation = umbrella::interface::next_provider_generation();
    umbrella::adapters::load_for_jit(&umbrella::adapters::AdapterCrate {
        path: counted.clone(),
        generation,
        exports: generated.exported_symbols(),
        retire_symbols: Vec::new(),
        drain_symbols: Vec::new(),
    })
    .expect("load the counting wrapper for the JIT");
    auth::set_allow_all();
    let cps = std::fs::read(&cps_path).expect("read CPS");
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
    assert_eq!(entry("computeThrough"), 11, "Right(7) read by the method");

    let jit_live = unsafe { live() };
    let jit_allocs = unsafe { allocs() };
    let jit_frees = unsafe { frees() };
    for _ in 0..5000 {
        assert_eq!(entry("computeThrough"), 11);
    }
    assert_eq!(
        unsafe { allocs() } - jit_allocs,
        5000,
        "one allocation per transferred method call"
    );
    assert_eq!(
        unsafe { frees() } - jit_frees,
        5000,
        "the method's `match` releases the union it was passed"
    );
    assert_eq!(
        unsafe { live() },
        jit_live,
        "no live allocation after 5000 transferred method calls"
    );
}

/// U15c's cross-module boundary: `EitherConsumer` produces the checked union
/// and passes it to `EitherProducer.consume`, whose body is a different CPS
/// artifact. The callee cannot name the caller's `__free`, so the caller
/// releases the union after the call returns: 5000 calls allocate 5000 times,
/// free 5000 times and leave no live bytes. A release that the callee emitted
/// instead would double-free, and one that neither side emitted would leave the
/// 5000 allocations live — both are the failure this counter would show.
#[test]
fn a_union_crossing_a_module_boundary_is_released_by_the_caller() {
    use austral_cranelift_bridge::{
        auth, compile_entry_i64, compile_to_function_named, execute_function,
    };
    let _jit = JIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("repo root");
    let dir = scratch("crossmodule");
    let producer = "umbrella/modules/EitherProducer.aui,umbrella/modules/EitherProducer.aum";
    let consumer = "umbrella/modules/EitherConsumer.aum";
    let (producer_cps, _) = compile_modules(root, &[producer], &dir, "producer");
    let (consumer_cps, adapters_path) =
        compile_modules(root, &[producer, consumer], &dir, "consumer");

    let descriptor_text = std::fs::read_to_string(&adapters_path).expect("read descriptors");
    let adapters = Adapter::parse_descriptors(&descriptor_text).expect("parse descriptors");
    assert_eq!(adapters.len(), 1, "one checked adapter in the consumer");
    assert_eq!(adapters[0].checked_tags, Some((1, 0)), "Left 1, Right 0");
    let wrapper = GeneratedAdapter::Scalar(&adapters[0])
        .render()
        .expect("render the checked union wrapper");

    let source = counted_crate_source(&wrapper, "rs_choose_consumer");
    let crate_source = dir.join("counting_crossmodule.rs");
    std::fs::write(&crate_source, source).expect("write the counting crate");
    let counted = dir.join("libeither_crossmodule_counted.so");
    let rlib = bridge_rlib();
    let deps = rlib.parent().expect("rlib directory");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=either_crossmodule_counted")
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("austral_cranelift_bridge={}", rlib.display()))
        .arg("-o")
        .arg(&counted)
        .arg(&crate_source)
        .output()
        .expect("spawn rustc");
    assert!(
        out.status.success(),
        "the counting wrapper must compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let lib = Library::open(&counted);
    let live = lib.counter("counted_live");
    let allocs = lib.counter("counted_allocs");
    let frees = lib.counter("counted_frees");
    let generated = GeneratedAdapter::Scalar(&adapters[0]);
    let generation = umbrella::interface::next_provider_generation();
    umbrella::adapters::load_for_jit(&umbrella::adapters::AdapterCrate {
        path: counted.clone(),
        generation,
        exports: generated.exported_symbols(),
        retire_symbols: Vec::new(),
        drain_symbols: Vec::new(),
    })
    .expect("load the counting wrapper for the JIT");
    auth::set_allow_all();

    // The dependency module is compiled into the same JIT module for its
    // definitions only, exactly as the umbrella's stage 4 does.
    let producer_bytes = std::fs::read(&producer_cps).expect("read producer CPS");
    let producer_fn = compile_to_function_named(
        producer_bytes.as_ptr(),
        producer_bytes.len(),
        std::ptr::null(),
        0,
    );
    assert!(!producer_fn.is_null(), "the producer module must compile");

    let cps = std::fs::read(&consumer_cps).expect("read consumer CPS");
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
    assert_eq!(
        entry("computeThrough"),
        11,
        "the producer's body read Right(7) across the module boundary"
    );

    let jit_live = unsafe { live() };
    let jit_allocs = unsafe { allocs() };
    let jit_frees = unsafe { frees() };
    for _ in 0..5000 {
        assert_eq!(entry("computeThrough"), 11);
    }
    assert_eq!(
        unsafe { allocs() } - jit_allocs,
        5000,
        "one allocation per checked call, produced in the caller's module"
    );
    assert_eq!(
        unsafe { frees() } - jit_frees,
        5000,
        "the caller releases the union after the cross-module call returns"
    );
    assert_eq!(
        unsafe { live() },
        jit_live,
        "no live allocation after 5000 cross-module transferred calls"
    );
}
