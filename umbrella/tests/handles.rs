//! U13 (bounded start) acceptance: an owned value is used only through the
//! handle that its own provider issued, and destruction is exact.
//!
//! Every test is a different mistake the registry has to refuse, and each one
//! would otherwise be a use-after-free, a cross-type reinterpretation or a
//! double destructor in generated code. The drop counter is the oracle for
//! destruction: it is independent of the registry's own release accounting, so
//! the two must agree.

use std::cell::RefCell;
use std::rc::Rc;

use umbrella::handles::{Handle, HandleError, HandleRegistry, Owner, TypeTag, HANDLE_SCHEMA};

/// A value that counts its own destructions.
#[derive(Debug)]
struct Tracked {
    id: u32,
    drops: Rc<RefCell<Vec<u32>>>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.borrow_mut().push(self.id);
    }
}

fn drops() -> Rc<RefCell<Vec<u32>>> {
    Rc::new(RefCell::new(Vec::new()))
}

fn tracked(id: u32, drops: &Rc<RefCell<Vec<u32>>>) -> Tracked {
    Tracked {
        id,
        drops: Rc::clone(drops),
    }
}

fn registry() -> HandleRegistry<Tracked> {
    HandleRegistry::new(Owner::issued(11), TypeTag::named("Tracked"), 1)
}

#[test]
fn the_handle_schema_is_versioned() {
    assert_eq!(HANDLE_SCHEMA, 1);
}

#[test]
fn a_stored_value_is_readable_through_its_own_handle() {
    let drops = drops();
    let mut registry = registry();
    let handle = registry.insert(tracked(1, &drops)).expect("insert");

    assert_eq!(registry.get(handle).expect("get").id, 1);
    registry.get_mut(handle).expect("get_mut").id = 2;
    assert_eq!(registry.get(handle).expect("get").id, 2);
    assert_eq!(registry.live(), 1);

    registry.destroy(handle).expect("destroy");
    assert_eq!(&*drops.borrow(), &[2]);
    assert_eq!(registry.live(), 0);
    assert_eq!(registry.released(), 1);
}

#[test]
fn destroying_a_handle_twice_does_not_drop_the_value_twice() {
    let drops = drops();
    let mut registry = registry();
    let handle = registry.insert(tracked(1, &drops)).expect("insert");

    registry.destroy(handle).expect("first destroy");
    let err = registry.destroy(handle).expect_err("second destroy");
    assert!(matches!(err, HandleError::Stale { .. }), "{err}");

    assert_eq!(&*drops.borrow(), &[1], "the destructor ran exactly once");
    assert_eq!(registry.released(), 1, "the release count did not double");
}

#[test]
fn a_reused_slot_does_not_resurrect_the_destroyed_value() {
    let drops = drops();
    let mut registry = registry();
    let old = registry.insert(tracked(1, &drops)).expect("insert");
    registry.destroy(old).expect("destroy");

    let new = registry.insert(tracked(2, &drops)).expect("insert");
    assert_eq!(new.slot(), old.slot(), "the freed slot is reused");
    assert_ne!(new, old, "but the handle is a different one");

    let err = registry.get(old).expect_err("stale handle");
    assert!(matches!(err, HandleError::Stale { .. }), "{err}");
    assert_eq!(registry.get(new).expect("new handle").id, 2);
    assert_eq!(&*drops.borrow(), &[1]);

    registry.destroy(new).expect("destroy");
    assert_eq!(&*drops.borrow(), &[1, 2]);
    assert_eq!(registry.issued(), 2);
    assert_eq!(registry.released(), 2);
}

#[test]
fn a_handle_from_another_provider_instance_is_refused() {
    let drops = drops();
    let mut mine = HandleRegistry::<Tracked>::new(Owner::issued(1), TypeTag::named("Tracked"), 1);
    let mut theirs = HandleRegistry::<Tracked>::new(Owner::issued(2), TypeTag::named("Tracked"), 1);

    let handle: Handle = theirs.insert(tracked(9, &drops)).expect("insert");
    let err = mine.get(handle).expect_err("foreign handle");
    match err {
        HandleError::ForeignOwner { expected, found } => {
            assert_eq!((expected, found), (1, 2));
        }
        other => panic!("expected ForeignOwner, got {other}"),
    }

    // Same slot index and generation in this table is still not this handle.
    let local = mine.insert(tracked(1, &drops)).expect("insert");
    assert_eq!(local.slot(), handle.slot());
    assert_ne!(local, handle);
    assert_eq!(mine.get(local).expect("local").id, 1);
}

#[test]
fn a_handle_for_another_type_is_refused() {
    let numbers = HandleRegistry::<u32>::new(Owner::issued(1), TypeTag::named("Counter"), 1);
    let mut strings = HandleRegistry::<u32>::new(Owner::issued(1), TypeTag::named("Label"), 1);

    let handle = strings.insert(7).expect("insert");
    let err = numbers.get(handle).expect_err("wrong type");
    match err {
        HandleError::ForeignType { expected, found } => {
            assert_eq!((expected, found), ("Counter", "Label"));
        }
        other => panic!("expected ForeignType, got {other}"),
    }
}

#[test]
fn a_handle_from_a_previous_provider_generation_is_refused() {
    let drops = drops();
    let mut old = HandleRegistry::<Tracked>::new(Owner::issued(1), TypeTag::named("Tracked"), 1);
    let handle = old.insert(tracked(1, &drops)).expect("insert");

    // The provider reloaded: same instance tag, same type, next generation.
    let mut new = HandleRegistry::<Tracked>::new(Owner::issued(1), TypeTag::named("Tracked"), 2);
    let err = new.get(handle).expect_err("stale provider");
    match err {
        HandleError::StaleProvider { expected, found } => assert_eq!((expected, found), (2, 1)),
        other => panic!("expected StaleProvider, got {other}"),
    }

    let fresh = new.insert(tracked(2, &drops)).expect("insert");
    assert_eq!(fresh.slot(), handle.slot());
    assert_ne!(fresh, handle);
    assert_eq!(new.get(fresh).expect("fresh").id, 2);
}

#[test]
fn a_word_pair_round_trips_and_the_null_pair_is_refused() {
    let mut registry = HandleRegistry::<u32>::new(Owner::issued(3), TypeTag::named("W"), 4);
    let handle = registry.insert(7).expect("insert");
    let (lo, hi) = handle.to_words();

    // Every component is 32 bits, so both words are the pair's fields verbatim.
    assert_eq!((lo >> 32, lo as u32), (3, handle.slot()));
    assert_eq!((hi >> 32, hi as u32), (4, 0));
    assert_eq!(*registry.get_words(lo, hi).expect("words"), 7);

    assert_eq!(
        registry.get_words(0, 0),
        Err(HandleError::Malformed { lo: 0, hi: 0 }),
        "the null pair is refused rather than read as slot 0"
    );
    assert_eq!(
        registry.get_words(0, hi),
        Err(HandleError::ForeignOwner {
            expected: 3,
            found: 0
        })
    );
    assert_eq!(
        registry.get_words(lo, hi + (1 << 32)),
        Err(HandleError::StaleProvider {
            expected: 4,
            found: 5
        })
    );
}

#[test]
fn allocation_and_free_counters_stand_apart_from_the_drop_oracle() {
    let drops = drops();
    let mut registry = registry();
    let destroyed = registry.insert(tracked(1, &drops)).expect("insert");
    let moved = registry.insert(tracked(2, &drops)).expect("insert");

    registry.destroy(destroyed).expect("destroy");
    let value = registry.take(moved).expect("take");

    assert_eq!(registry.allocated(), 2, "two values were stored");
    assert_eq!(registry.freed(), 1, "only `destroy` frees in the table");
    assert_eq!(registry.released(), 2, "both slots are free");
    assert_eq!(registry.live(), 0);
    assert_eq!(
        &*drops.borrow(),
        &[1],
        "a moved value is the caller's to drop"
    );

    drop(value);
    assert_eq!(
        &*drops.borrow(),
        &[1, 2],
        "the oracle agrees with `freed` + the moved drop"
    );
}

#[test]
fn take_moves_the_value_out_and_leaves_the_handle_stale() {
    let drops = drops();
    let mut registry = registry();
    let handle = registry.insert(tracked(5, &drops)).expect("insert");

    let value = registry.take(handle).expect("take");
    assert_eq!(value.id, 5);
    assert!(drops.borrow().is_empty(), "moving does not drop");
    drop(value);
    assert_eq!(&*drops.borrow(), &[5]);

    let err = registry.take(handle).expect_err("second take");
    assert!(matches!(err, HandleError::Stale { .. }), "{err}");
    assert_eq!(&*drops.borrow(), &[5], "the value was not dropped again");
}

// ── the generated `__new`/`__dispose` wrappers, through the real ABI ────────
//
// The registry above is the table. These tests are the other half: wrappers
// rendered from `abi handle` descriptors, compiled next to a provider that
// implements the declared constructor and destructor, and called the way the
// JIT calls them — two 64-bit handle words in, a status word out. The
// provider's own drop counter is the oracle; nothing in the test can ask the
// registry what it thinks.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::interface::{
    render_handle_wrapper, render_handle_wrapper_at, HandleAdapter, DESCRIPTOR_SCHEMA,
    HANDLE_ABI_VERSION,
};

fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-handles-{}-{unique}-{name}",
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

fn handle_adapter(
    target: &str,
    type_name: &str,
    owner: u32,
    accessor: Option<&str>,
) -> HandleAdapter {
    HandleAdapter {
        schema: DESCRIPTOR_SCHEMA,
        abi_version: HANDLE_ABI_VERSION,
        symbol: format!("{target}__new"),
        target: target.to_string(),
        constructor: format!("{target}_ctor"),
        destructor: format!("{target}_dtor"),
        type_name: type_name.to_string(),
        params: 1,
        owner,
        use_symbol: accessor.map(str::to_string),
    }
}

/// The declared targets, written the way Rust declares them, followed by the
/// two generated wrapper modules. Both counters live in the provider, so a
/// destructor call that did not happen (or happened twice) is visible.
fn library_source() -> String {
    let mut source = String::from(
        r##"// the provider's side of the declared handle targets
static WIDGET_LIVE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static WIDGET_DROPS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static GADGET_DROPS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[export_name = "avm_widget_ctor"]
pub extern "C" fn widget_ctor(value: i64) -> *mut core::ffi::c_void {
    WIDGET_LIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Box::into_raw(Box::new(value)) as *mut core::ffi::c_void
}

#[export_name = "avm_widget_dtor"]
pub unsafe extern "C" fn widget_dtor(ptr: *mut core::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    WIDGET_LIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    WIDGET_DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    drop(Box::from_raw(ptr as *mut i64));
}

#[export_name = "avm_gadget_ctor"]
pub extern "C" fn gadget_ctor(value: i64) -> *mut core::ffi::c_void {
    Box::into_raw(Box::new(value)) as *mut core::ffi::c_void
}

#[export_name = "avm_gadget_dtor"]
pub unsafe extern "C" fn gadget_dtor(ptr: *mut core::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    GADGET_DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    drop(Box::from_raw(ptr as *mut i64));
}

#[export_name = "avm_widget_live"]
pub extern "C" fn widget_live() -> i64 {
    WIDGET_LIVE.load(std::sync::atomic::Ordering::SeqCst)
}

#[export_name = "avm_widget_read"]
pub unsafe extern "C" fn widget_read(ptr: *mut core::ffi::c_void) -> i64 {
    if ptr.is_null() {
        return -1;
    }
    *(ptr as *mut i64)
}

#[export_name = "avm_widget_drops"]
pub extern "C" fn widget_drops() -> i64 {
    WIDGET_DROPS.load(std::sync::atomic::Ordering::SeqCst)
}

#[export_name = "avm_gadget_drops"]
pub extern "C" fn gadget_drops() -> i64 {
    GADGET_DROPS.load(std::sync::atomic::Ordering::SeqCst)
}

"##,
    );
    let widget = handle_adapter("avm_widget", "widget", 1, Some("avm_widget_read"));
    let gadget = handle_adapter("avm_gadget", "gadget", 2, None);
    for (i, adapter) in [&widget, &gadget].iter().enumerate() {
        source.push_str(&format!(
            "mod w{i} {{\n{}\n}}\n",
            render_handle_wrapper(adapter).expect("render")
        ));
    }
    // The same descriptor rendered again, as a provider *reload* renders it: a
    // second build of the same provider, at generation 2, with its own table.
    // Only the exported wrapper names differ (`avm_reload__*`), because two
    // copies cannot both own the generation-1 symbols in one process.
    let mut reloaded = render_handle_wrapper_at(&widget, 2).expect("render reloaded");
    // The generation is the render's, not the descriptor's, so rendering at 2
    // already wrote it; only the exported symbol names need renaming to keep
    // this crate linkable beside the generation-1 module above.
    assert!(
        reloaded.contains("const PROVIDER_GENERATION: u32 = 2;"),
        "{reloaded}"
    );
    for (from, to) in [
        ("fn avm_widget__new", "fn avm_reload__new"),
        ("fn avm_widget__use", "fn avm_reload__use"),
        ("fn avm_widget__dispose", "fn avm_reload__dispose"),
    ] {
        assert!(reloaded.contains(from), "the reload edit `{from}` applies");
        reloaded = reloaded.replace(from, to);
    }
    source.push_str(&format!("mod reloaded {{\n{reloaded}\n}}\n"));
    source
}

fn compiled_library() -> PathBuf {
    use std::sync::OnceLock;
    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
    LIBRARY
        .get_or_init(|| {
            let dir = scratch("wrappers");
            let file = dir.join("lib.rs");
            std::fs::write(&file, library_source()).expect("write library");
            let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/deps");
            let rlib = bridge_rlib();
            let out = dir.join("libhandles.so");
            let output = Command::new("rustc")
                .arg("--crate-type=cdylib")
                .arg("--edition=2021")
                .arg("--crate-name=handle_wrappers")
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
                "the provider and its generated wrappers must compile:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            out
        })
        .clone()
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

    fn try_symbol(&self, name: &str) -> Option<*mut libc::c_void> {
        let c = CString::new(name).expect("no NUL");
        let ptr = unsafe { libc::dlsym(self.0, c.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr)
        }
    }

    fn new_fn(&self, name: &str) -> unsafe extern "C" fn(i64, *mut i64) -> i64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }

    fn use_fn(&self, name: &str) -> unsafe extern "C" fn(i64, i64, *mut i64) -> i64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }

    fn dispose_fn(&self, name: &str) -> unsafe extern "C" fn(i64, i64) -> i64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }

    fn count_fn(&self, name: &str) -> unsafe extern "C" fn() -> i64 {
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

/// The wrappers' provider counters (`live`, `drops`) are statics in the loaded
/// library, and tests run in parallel, so two tests sharing one image would
/// read each other's numbers. `dlopen` keys on the file, so each test gets its
/// own copy of the compiled wrappers and therefore its own statics.
fn fresh_library(name: &str) -> PathBuf {
    let dir = scratch(name);
    let copy = dir.join("libhandles.so");
    std::fs::copy(compiled_library(), &copy).expect("copy the compiled wrappers");
    copy
}

#[test]
fn the_generated_wrappers_create_and_destroy_a_provider_value_once() {
    let lib = Library::open(&fresh_library("create"));
    let widget_new = lib.new_fn("avm_widget__new");
    let widget_dispose = lib.dispose_fn("avm_widget__dispose");
    let widget_live = lib.count_fn("avm_widget_live");
    let widget_drops = lib.count_fn("avm_widget_drops");
    let gadget_dispose = lib.dispose_fn("avm_gadget__dispose");
    let gadget_drops = lib.count_fn("avm_gadget_drops");

    // 1. A constructor writes two words and reports done.
    let mut words = [0i64; 2];
    let status = unsafe { widget_new(7, words.as_mut_ptr()) };
    assert_eq!(status, 0, "the constructor reports done");
    assert_ne!(words, [0, 0], "the handle is not the null pair");
    assert_eq!(unsafe { widget_live() }, 1);
    assert_eq!(unsafe { widget_drops() }, 0, "nothing is freed yet");

    // 2. Disposing the same words frees exactly once; the second attempt is a
    //    refusal, not a second destructor call.
    let status = unsafe { widget_dispose(words[0], words[1]) };
    assert_eq!(status, 0, "the first dispose succeeds");
    assert_eq!(unsafe { widget_live() }, 0);
    assert_eq!(unsafe { widget_drops() }, 1);
    let status = unsafe { widget_dispose(words[0], words[1]) };
    assert_ne!(status, 0, "the second dispose is refused");
    assert_eq!(
        unsafe { widget_drops() },
        1,
        "the destructor did not run twice"
    );

    // 3. Another type's registry refuses these words: the owner tag differs,
    //    so no cross-type destructor is reachable.
    let mut gadget_words = [0i64; 2];
    let gadget_new = lib.new_fn("avm_gadget__new");
    assert_eq!(unsafe { gadget_new(9, gadget_words.as_mut_ptr()) }, 0);
    assert_ne!(
        unsafe { gadget_dispose(words[0], words[1]) },
        0,
        "a widget's words do not dispose a gadget's table entry"
    );
    assert_eq!(
        unsafe { gadget_dispose(gadget_words[0], gadget_words[1]) },
        0,
        "the gadget's own words still work"
    );
    assert_eq!(unsafe { gadget_drops() }, 1);

    // 4. A slot reused after disposal bumps its generation, so the old words
    //    stay stale and the new value survives their refusal.
    let mut first = [0i64; 2];
    assert_eq!(unsafe { widget_new(1, first.as_mut_ptr()) }, 0);
    assert_eq!(unsafe { widget_dispose(first[0], first[1]) }, 0);
    let mut reused = [0i64; 2];
    assert_eq!(unsafe { widget_new(2, reused.as_mut_ptr()) }, 0);
    assert_eq!(
        reused[0], first[0],
        "the same slot is handed out again (same low word)"
    );
    assert_ne!(reused[1], first[1], "but its generation moved on");
    assert_ne!(
        unsafe { widget_dispose(first[0], first[1]) },
        0,
        "the stale words are refused"
    );
    assert_eq!(
        unsafe { widget_live() },
        1,
        "the live value was not touched"
    );
    assert_eq!(unsafe { widget_dispose(reused[0], reused[1]) }, 0);
    assert_eq!(unsafe { widget_live() }, 0);

    // 5. Forged words: a previous provider generation, a null out pointer and
    //    the null pair are all refusals before any provider code runs.
    let mut live = [0i64; 2];
    assert_eq!(unsafe { widget_new(5, live.as_mut_ptr()) }, 0);
    assert_ne!(
        unsafe { widget_dispose(live[0], live[1] + (1 << 32)) },
        0,
        "a future provider generation is not this provider"
    );
    assert_eq!(unsafe { widget_live() }, 1);
    assert_ne!(
        unsafe { widget_new(6, std::ptr::null_mut()) },
        0,
        "a null out pointer is refused"
    );
    assert_eq!(unsafe { widget_live() }, 1, "nothing was allocated for it");
    assert_ne!(
        unsafe { widget_dispose(0, 0) },
        0,
        "the null pair is refused"
    );
    assert_eq!(unsafe { widget_dispose(live[0], live[1]) }, 0);
    assert_eq!(unsafe { widget_live() }, 0);
}

#[test]
fn the_reading_wrapper_hands_back_the_provider_value() {
    let lib = Library::open(&fresh_library("use"));
    let widget_new = lib.new_fn("avm_widget__new");
    let widget_dispose = lib.dispose_fn("avm_widget__dispose");
    let widget_use = lib.use_fn("avm_widget__use");
    let widget_live = lib.count_fn("avm_widget_live");
    let widget_drops = lib.count_fn("avm_widget_drops");

    let mut words = [0i64; 2];
    assert_eq!(unsafe { widget_new(41, words.as_mut_ptr()) }, 0);

    // 1. The reader resolves the words through the same registry the
    //    constructor wrote them to, and the provider's accessor is what turns
    //    its pointer into the word the module reads.
    let mut out = [0i64; 1];
    assert_eq!(
        unsafe { widget_use(words[0], words[1], out.as_mut_ptr()) },
        0,
        "reading a live handle succeeds"
    );
    assert_eq!(out[0], 41, "the value the constructor stored");
    assert_eq!(unsafe { widget_live() }, 1, "reading does not destroy");

    // 2. A refusal leaves the out slot untouched: no provider code ran, so
    //    the module cannot mistake an old word for a fresh read.
    let mut untouched = [-9i64; 1];
    assert_ne!(
        unsafe { widget_use(words[0], words[1] + (1 << 32), untouched.as_mut_ptr()) },
        0,
        "a future provider generation is not this table"
    );
    assert_eq!(untouched[0], -9, "a refusal does not write the out slot");
    assert_ne!(
        unsafe { widget_use(0, 0, untouched.as_mut_ptr()) },
        0,
        "the null pair is refused"
    );
    assert_eq!(untouched[0], -9);
    assert_ne!(
        unsafe { widget_use(words[0], words[1], std::ptr::null_mut()) },
        0,
        "a null out pointer is refused"
    );

    // 3. After disposal the words are stale, so the reader refuses them too —
    //    the read path cannot see a value the destroy path already freed.
    assert_eq!(unsafe { widget_dispose(words[0], words[1]) }, 0);
    assert_ne!(
        unsafe { widget_use(words[0], words[1], out.as_mut_ptr()) },
        0,
        "a destroyed handle is not readable"
    );
    assert_eq!(out[0], 41, "the refusal did not rewrite the last read");
    assert_eq!(unsafe { widget_drops() }, 1);

    // 4. A target with no `use` line gets no reading wrapper: the descriptor
    //    decides, so a provider cannot be made to expose a reader it never
    //    named.
    assert!(
        lib.try_symbol("avm_gadget__use").is_none(),
        "a descriptor without `use` renders no `__use` wrapper"
    );
}

#[test]
fn a_reload_refuses_handles_from_the_previous_build() {
    let lib = Library::open(&fresh_library("reload"));
    let widget_new = lib.new_fn("avm_widget__new");
    let widget_dispose = lib.dispose_fn("avm_widget__dispose");
    let reload_new = lib.new_fn("avm_reload__new");
    let reload_dispose = lib.dispose_fn("avm_reload__dispose");
    let reload_use = lib.use_fn("avm_reload__use");
    let widget_live = lib.count_fn("avm_widget_live");

    // One provider value per build: the two tables are separate registries,
    // as a reload's are.
    let mut old = [0i64; 2];
    assert_eq!(unsafe { widget_new(1, old.as_mut_ptr()) }, 0);
    let mut fresh = [0i64; 2];
    assert_eq!(unsafe { reload_new(2, fresh.as_mut_ptr()) }, 0);
    assert_eq!(unsafe { widget_live() }, 2);

    // The new table refuses the old build's words: the slot they name is this
    // table's slot too, and the value behind it is a different one.
    let mut out = [0i64; 1];
    assert_ne!(
        unsafe { reload_dispose(old[0], old[1]) },
        0,
        "the generation-1 words are refused by the generation-2 table"
    );
    assert_ne!(
        unsafe { reload_use(old[0], old[1], out.as_mut_ptr()) },
        0,
        "and they are not readable through it either"
    );
    assert_eq!(
        unsafe { widget_live() },
        2,
        "the refusal ran no destructor: the old value is still alive"
    );

    // The old value is still usable through the table that minted it, which is
    // what "refused by the new build" has to mean rather than "leaked".
    assert_eq!(
        unsafe { widget_dispose(old[0], old[1]) },
        0,
        "the generation-1 table still resolves its own words"
    );
    assert_eq!(unsafe { widget_live() }, 1);
    assert_eq!(
        unsafe { reload_use(fresh[0], fresh[1], out.as_mut_ptr()) },
        0
    );
    assert_eq!(
        out[0], 2,
        "the new build's value reads through its own table"
    );
    assert_eq!(unsafe { reload_dispose(fresh[0], fresh[1]) }, 0);
    assert_eq!(unsafe { widget_live() }, 0);
}
