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
    render_handle_wrapper, GeneratedAdapter, HandleAdapter, DESCRIPTOR_SCHEMA, HANDLE_ABI_VERSION,
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

    #[allow(clippy::type_complexity)]
    fn with_fn(
        &self,
        name: &str,
    ) -> unsafe extern "C" fn(
        i64,
        i64,
        Option<unsafe extern "C" fn(*mut libc::c_void) -> i64>,
        *mut i64,
    ) -> i64 {
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

static WITH_CALLS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static WITH_SEEN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// The "Austral callback": it is handed the provider's own pointer, sees the
/// value for the duration of the call, and gives one word back.
unsafe extern "C" fn widget_double(ptr: *mut libc::c_void) -> i64 {
    WITH_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if ptr.is_null() {
        return -1;
    }
    let value = *(ptr as *mut i64);
    WITH_SEEN.store(value, std::sync::atomic::Ordering::SeqCst);
    value * 2
}

#[test]
fn the_scoped_borrow_lends_the_value_and_keeps_it_live() {
    let lib = Library::open(&fresh_library("with"));
    let widget_new = lib.new_fn("avm_widget__new");
    let widget_with = lib.with_fn("avm_widget__with");
    let widget_dispose = lib.dispose_fn("avm_widget__dispose");
    let widget_live = lib.count_fn("avm_widget_live");
    let widget_drops = lib.count_fn("avm_widget_drops");
    WITH_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);

    let mut words = [0i64; 2];
    assert_eq!(unsafe { widget_new(41, words.as_mut_ptr()) }, 0);

    // 1. The callback is called with the provider's pointer, sees the value,
    //    and its word is what the caller receives — the module never gets the
    //    pointer and never gets the value itself.
    let mut out = [-7i64; 1];
    assert_eq!(
        unsafe { widget_with(words[0], words[1], Some(widget_double), out.as_mut_ptr()) },
        0,
        "lending a live handle succeeds"
    );
    assert_eq!(out[0], 82, "the callback's word came back");
    assert_eq!(
        WITH_SEEN.load(std::sync::atomic::Ordering::SeqCst),
        41,
        "the callback saw the provider's value"
    );
    assert_eq!(unsafe { widget_live() }, 1, "lending does not destroy");

    // 2. The borrow was scoped: the same words are still the caller's to
    //    dispose, and the destructor runs exactly once when they are.
    assert_eq!(unsafe { widget_dispose(words[0], words[1]) }, 0);
    assert_eq!(unsafe { widget_drops() }, 1);

    // 3. Every refusal is before the callback: a stale pair, a null callback
    //    and a null out pointer each leave the callback uncalled and the out
    //    slot untouched.
    let calls = WITH_CALLS.load(std::sync::atomic::Ordering::SeqCst);
    let mut untouched = [-9i64; 1];
    assert_ne!(
        unsafe {
            widget_with(
                words[0],
                words[1],
                Some(widget_double),
                untouched.as_mut_ptr(),
            )
        },
        0,
        "a destroyed handle is not lendable"
    );
    assert_ne!(
        unsafe {
            widget_with(
                words[0],
                words[1] + (1 << 32),
                Some(widget_double),
                untouched.as_mut_ptr(),
            )
        },
        0,
        "a future provider generation is not this table"
    );
    assert_ne!(
        unsafe { widget_with(0, 0, Some(widget_double), untouched.as_mut_ptr()) },
        0,
        "the null pair is refused"
    );
    assert_eq!(
        WITH_CALLS.load(std::sync::atomic::Ordering::SeqCst),
        calls,
        "no refusal reached the callback"
    );
    assert_eq!(untouched[0], -9, "a refusal does not write the out slot");

    let mut live = [0i64; 2];
    assert_eq!(unsafe { widget_new(3, live.as_mut_ptr()) }, 0);
    assert_ne!(
        unsafe { widget_with(live[0], live[1], None, untouched.as_mut_ptr()) },
        0,
        "a null callback is refused"
    );
    assert_ne!(
        unsafe { widget_with(live[0], live[1], Some(widget_double), std::ptr::null_mut()) },
        0,
        "a null out pointer is refused"
    );
    assert_eq!(
        WITH_CALLS.load(std::sync::atomic::Ordering::SeqCst),
        calls,
        "a refused call does not lend the value"
    );
    assert_eq!(unsafe { widget_dispose(live[0], live[1]) }, 0);
    assert_eq!(unsafe { widget_live() }, 0);
}

/// A tiny provider for the reload test. Its symbols are named apart from the
/// other tests' provider on purpose: the wrapper crates this test loads are
/// separate images that bind these symbols from the global scope, and a
/// shared `avm_widget_ctor` would let the loader pick the other provider.
const HOT_PROVIDER: &str = r##"
static HOT_LIVE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[export_name = "avm_hotbuild_ctor"]
pub extern "C" fn hot_ctor(value: i64) -> *mut core::ffi::c_void {
    HOT_LIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Box::into_raw(Box::new(value)) as *mut core::ffi::c_void
}

#[export_name = "avm_hotbuild_dtor"]
pub unsafe extern "C" fn hot_dtor(ptr: *mut core::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    HOT_LIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    drop(Box::from_raw(ptr as *mut i64));
}

#[export_name = "avm_hotbuild_live"]
pub extern "C" fn hot_live() -> i64 {
    HOT_LIVE.load(std::sync::atomic::Ordering::SeqCst)
}
"##;

/// The provider, built and loaded the way `main.rs` step 2 loads it: one
/// cdylib, mapped globally so the adapter crates built later can bind its
/// constructors and destructors with `RTLD_NOW`.
fn hot_provider() -> PathBuf {
    // One copy per process: the adapter crates bind `avm_hot_*` through
    // `RTLD_GLOBAL`, so a second copy would make `avm_hot_live` in this test
    // read a different registry than the constructors the adapters reached.
    // The reload keeps its own adapter tables, which is the part under test.
    static PROVIDER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(path) = PROVIDER.get() {
        return path.clone();
    }
    let dir = scratch("hot-provider");
    let file = dir.join("lib.rs");
    std::fs::write(&file, HOT_PROVIDER).expect("write provider");
    let out = dir.join("libhot_provider.so");
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=hot_provider")
        .arg("-o")
        .arg(&out)
        .arg(&file)
        .output()
        .expect("spawn rustc");
    assert!(
        output.status.success(),
        "the provider must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    umbrella::policy::dlopen(&out).expect("load the provider globally");
    PROVIDER.get_or_init(|| out).clone()
}

#[test]
fn a_host_reload_builds_and_loads_the_next_generation() {
    let provider = hot_provider();
    let live = Library::open(&provider).count_fn("avm_hotbuild_live");
    let adapter = handle_adapter("avm_hotbuild", "hot", 1, None);
    let adapters = [GeneratedAdapter::Handle(&adapter)];

    // The host's own build path, twice: each call renders at the next provider
    // generation and writes a new artifact, and the reload *loads* the second
    // one instead of rendering into the previous crate.
    let first = umbrella::adapters::build_next(&adapters).expect("first build");
    umbrella::adapters::load(&first).expect("load the first adapter crate");

    assert!(first.generation >= 1, "generation 0 is reserved");
    let second = umbrella::adapters::build_next(&adapters).expect("reload build");
    umbrella::adapters::load(&second).expect("load the reloaded adapter crate");
    assert_eq!(
        second.generation,
        first.generation + 1,
        "a reload is the next build, not a second render of the same one"
    );
    assert_ne!(
        first.path, second.path,
        "a reload writes a new image: dlopen keys on the path, so reusing it \
         would hand back the previous build"
    );

    let old = Library::open(&first.path);
    let reloaded = Library::open(&second.path);
    let first_new = old.new_fn("avm_hotbuild__new");
    let first_dispose = old.dispose_fn("avm_hotbuild__dispose");
    let reload_new = reloaded.new_fn("avm_hotbuild__new");
    let reload_dispose = reloaded.dispose_fn("avm_hotbuild__dispose");

    // One provider value per build: the two tables are separate registries,
    // as a reload's are.
    let mut before = [0i64; 2];
    assert_eq!(unsafe { first_new(1, before.as_mut_ptr()) }, 0);
    let mut after = [0i64; 2];
    assert_eq!(unsafe { reload_new(2, after.as_mut_ptr()) }, 0);
    assert_eq!(unsafe { live() }, 2);

    // The new table refuses the old build's words: the slot they name is this
    // table's slot too, and the value behind it is a different one.
    assert_ne!(
        unsafe { reload_dispose(before[0], before[1]) },
        0,
        "the previous build's words are refused by the reloaded table"
    );
    assert_eq!(
        unsafe { live() },
        2,
        "the refusal ran no destructor: the old value is still alive"
    );

    // The old value is still usable through the table that minted it, which is
    // what "refused by the new build" has to mean rather than "leaked".
    assert_eq!(
        unsafe { first_dispose(before[0], before[1]) },
        0,
        "the previous crate still resolves its own words"
    );
    assert_eq!(unsafe { live() }, 1);
    assert_eq!(unsafe { reload_dispose(after[0], after[1]) }, 0);
    assert_eq!(unsafe { live() }, 0);
}

// The provider for the live-reload test. Its own image is loaded once, before
// any wrapper crate, so the wrapper crates' `RTLD_NOW` binding finds these
// symbols. `wait`/`waiting`/`release` are the host-module gate; `remember`/
// `recall` carry the pre-reload handle words out to the host.
const RELOAD_PROVIDER: &str = r##"
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

static LIVE: AtomicI64 = AtomicI64::new(0);
static DROPS: AtomicI64 = AtomicI64::new(0);
static WAITING: AtomicI64 = AtomicI64::new(0);
static RELEASED: AtomicI64 = AtomicI64::new(0);
static HELD_LO: AtomicI64 = AtomicI64::new(0);
static HELD_HI: AtomicI64 = AtomicI64::new(0);

#[export_name = "avm_hot_ctor"]
pub extern "C" fn hot_ctor(value: i64) -> *mut core::ffi::c_void {
    LIVE.fetch_add(1, Ordering::SeqCst);
    Box::into_raw(Box::new(value)) as *mut core::ffi::c_void
}

#[export_name = "avm_hot_dtor"]
pub unsafe extern "C" fn hot_dtor(ptr: *mut core::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    LIVE.fetch_sub(1, Ordering::SeqCst);
    DROPS.fetch_add(1, Ordering::SeqCst);
    drop(Box::from_raw(ptr as *mut i64));
}

#[export_name = "avm_hot_read"]
pub unsafe extern "C" fn hot_read(ptr: *mut core::ffi::c_void) -> i64 {
    if ptr.is_null() { -1 } else { *(ptr as *mut i64) }
}

#[export_name = "avm_hot_live"]
pub extern "C" fn hot_live() -> i64 { LIVE.load(Ordering::SeqCst) }

#[export_name = "avm_hot_drops"]
pub extern "C" fn hot_drops() -> i64 { DROPS.load(Ordering::SeqCst) }

#[export_name = "avm_hot_wait"]
pub extern "C" fn hot_wait() -> i64 {
    WAITING.store(1, Ordering::SeqCst);
    let start = Instant::now();
    while RELEASED.load(Ordering::SeqCst) == 0 {
        if start.elapsed() > Duration::from_secs(30) {
            WAITING.store(0, Ordering::SeqCst);
            return 1;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    0
}

#[export_name = "avm_hot_waiting"]
pub extern "C" fn hot_waiting() -> i64 { WAITING.load(Ordering::SeqCst) }

#[export_name = "avm_hot_release"]
pub extern "C" fn hot_release() -> i64 {
    RELEASED.store(1, Ordering::SeqCst);
    0
}

#[export_name = "avm_hot_remember"]
pub extern "C" fn hot_remember(lo: i64, hi: i64) -> i64 {
    HELD_LO.store(lo, Ordering::SeqCst);
    HELD_HI.store(hi, Ordering::SeqCst);
    0
}

#[export_name = "avm_hot_recall"]
pub unsafe extern "C" fn hot_recall(out: *mut i64) -> i64 {
    if out.is_null() {
        return 1;
    }
    *out = HELD_LO.load(Ordering::SeqCst);
    *out.add(1) = HELD_HI.load(Ordering::SeqCst);
    0
}
"##;

/// Build the live-reload provider into its own scratch directory. It is not
/// loaded here: the test maps it globally first, because a wrapper crate's
/// `RTLD_NOW` has to find its constructor at load time.
fn reload_provider() -> PathBuf {
    let dir = scratch("reload-provider");
    let file = dir.join("lib.rs");
    std::fs::write(&file, RELOAD_PROVIDER).expect("write provider");
    let out = dir.join("libreload_provider.so");
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=reload_provider")
        .arg("-o")
        .arg(&out)
        .arg(&file)
        .output()
        .expect("spawn rustc");
    assert!(
        output.status.success(),
        "the reload provider must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

/// The same provider with its own symbol prefix: both reload tests map a
/// provider `RTLD_GLOBAL`, and a shared `avm_hot_*` name would let one test's
/// image answer the other's calls.
fn live_provider() -> PathBuf {
    let dir = scratch("live-provider");
    let file = dir.join("lib.rs");
    std::fs::write(&file, RELOAD_PROVIDER.replace("avm_hot_", "avm_live_"))
        .expect("write provider");
    let out = dir.join("liblive_provider.so");
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=live_provider")
        .arg("-o")
        .arg(&out)
        .arg(&file)
        .output()
        .expect("spawn rustc");
    assert!(
        output.status.success(),
        "the live provider must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    out
}

#[test]
fn a_module_running_across_a_reload_sees_the_next_generation() {
    let front_end = Path::new(env!("CARGO_MANIFEST_DIR")).join("../austral");
    assert!(
        front_end.exists(),
        "the live reload test needs the dune-built front-end at {}; \
         build it with `nix develop \"git+file://$PWD#ocaml\" -c make all`",
        front_end.display()
    );
    let bridge_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../safestos/cranelift/target/release");

    // 1. The provider image, mapped globally before any wrapper crate.
    let provider = live_provider();
    umbrella::policy::dlopen(&provider).expect("load the provider globally");
    let provider_lib = Library::open(&provider);
    let live = provider_lib.count_fn("avm_live_live");
    let drops = provider_lib.count_fn("avm_live_drops");
    let waiting = provider_lib.count_fn("avm_live_waiting");
    let release = provider_lib.count_fn("avm_live_release");
    let recall: unsafe extern "C" fn(*mut i64) -> i64 =
        unsafe { std::mem::transmute(provider_lib.symbol("avm_live_recall")) };

    // 2. The front-end over the real fixture module: CPS bytes plus the
    //    `abi handle 1` descriptor the generator renders.
    let dir = scratch("reload-front-end");
    let cps_path = dir.join("HotHandle.cps");
    let adapters_path = dir.join("HotHandle.adapters");
    let mut command = Command::new(&front_end);
    command
        .arg("compile")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("modules/HotHandle.aum"))
        .arg("--use-cps-jit")
        .arg("--allow-all")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cps_path.display()))
        .arg(format!("--emit-adapters={}", adapters_path.display()))
        // The front-end's own JIT attempt runs before the wrappers exist; the
        // test compiles the CPS itself with them loaded.
        .env("CPS_JIT_ADVISORY", "1");
    if bridge_dir.join("libaustral_cranelift_bridge.so").exists() {
        let existing = std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default();
        let joined = if existing.is_empty() {
            bridge_dir.display().to_string()
        } else {
            format!("{}:{}", bridge_dir.display(), existing.to_string_lossy())
        };
        command.env("LD_LIBRARY_PATH", joined);
    }
    let out = command.output().expect("run the front-end");
    assert!(
        out.status.success(),
        "the front-end must typecheck the fixture:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let cps = std::fs::read(&cps_path).expect("read CPS");
    let descriptor_text = std::fs::read_to_string(&adapters_path).expect("read descriptors");
    let mut parsed = HandleAdapter::parse_descriptors(&descriptor_text).expect("parse descriptors");
    assert_eq!(parsed.len(), 1, "the fixture names one handle target");
    let adapter = parsed.remove(0);
    assert_eq!(adapter.target, "avm_live");
    let adapters = [GeneratedAdapter::Handle(&adapter)];

    // 3. The first generation, built and loaded after the provider.
    let first = umbrella::adapters::build_next(&adapters).expect("first build");
    umbrella::adapters::load(&first).expect("load the first adapter crate");

    // 4. Compile the module with the gate open and the wrappers resolvable.
    austral_cranelift_bridge::auth::set_allow_all();
    let entry = b"liveReload";
    let function = austral_cranelift_bridge::compile_entry_i64(
        cps.as_ptr(),
        cps.len(),
        entry.as_ptr(),
        entry.len(),
        0,
    );
    assert!(!function.is_null(), "the module must compile once the wrappers are loaded");

    // 5. Run it on another thread, so the host can swap the generation while
    //    the module is inside `avm_live_wait`.
    let address = function as usize;
    let run = std::thread::spawn(move || {
        austral_cranelift_bridge::execute_function(address as *const std::ffi::c_void)
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while unsafe { waiting() } == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the module never reached the reload gate"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // 6. The reload: next generation, loaded, and the live forwarders bound to
    //    it. The module was compiled against generation one, and keeps working.
    let second = umbrella::adapters::build_next(&adapters).expect("reload build");
    umbrella::adapters::load(&second).expect("load the reloaded adapter crate");
    assert_eq!(second.generation, first.generation + 1);
    assert_ne!(first.path, second.path);
    umbrella::adapters::bind(&first.path, &second, &adapter).expect("bind the reload");
    unsafe { release() };

    let result = run.join().expect("the module thread must not panic");
    assert_eq!(
        result, 0,
        "the module reports a wrong step (11 mint, 12 read, 13 gate, \
         14 stale accepted, 18 remember); provider live={}, drops={}, \
         generations {}/{}",
        unsafe { live() },
        unsafe { drops() },
        first.generation,
        second.generation
    );

    // 7. The refusal ran no destructor and the old value is still owned; the
    //    new generation has minted nothing yet.
    assert_eq!(unsafe { live() }, 1, "the pre-reload value is still owned");
    assert_eq!(unsafe { drops() }, 0, "a refused dispose drops nothing");

    // 8. The forwarders the running module called now answer from the new
    //    generation: the same addresses mint, read and dispose a handle whose
    //    words name generation two.
    let forwarding = Library::open(&first.path);
    let mint = forwarding.new_fn("avm_live__new");
    let read = forwarding.use_fn("avm_live__use");
    let dispose = forwarding.dispose_fn("avm_live__dispose");
    let mut fresh = [0i64; 2];
    assert_eq!(unsafe { mint(9, fresh.as_mut_ptr()) }, 0, "post-reload mint");
    assert_eq!(
        (fresh[1] >> 32) as u32,
        second.generation,
        "the post-reload handle names the generation that minted it"
    );
    let mut word = [0i64; 2];
    assert_eq!(unsafe { read(fresh[0], fresh[1], word.as_mut_ptr()) }, 0);
    assert_eq!(word[0], 9, "the forwarder read the value the provider holds");
    assert_eq!(unsafe { dispose(fresh[0], fresh[1]) }, 0);
    assert_eq!(unsafe { live() }, 1);
    assert_eq!(unsafe { drops() }, 1);

    // 9. The old value was not leaked: its own generation still resolves its
    //    words, so the host frees it through the table that minted it.
    let mut words = [0i64; 2];
    assert_eq!(unsafe { recall(words.as_mut_ptr()) }, 0, "recall the words");
    assert_ne!(words, [0, 0], "the fixture recorded a real handle");
    let first_lib = Library::open(&first.path);
    let first_dispose = first_lib.dispose_fn(&HandleAdapter::implementation_symbol(
        &adapter.dispose_symbol(),
        first.generation,
    ));
    assert_eq!(
        unsafe { first_dispose(words[0], words[1]) },
        0,
        "the previous crate still resolves the words its table minted"
    );
    assert_eq!(unsafe { live() }, 0);
    assert_eq!(unsafe { drops() }, 2);
}

#[test]
fn a_forwarder_reaches_the_generation_it_was_bound_to() {
    let dir = scratch("binding-provider");
    let file = dir.join("lib.rs");
    std::fs::write(&file, RELOAD_PROVIDER).expect("write provider");
    let provider = dir.join("libbinding_provider.so");
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=binding_provider")
        .arg("-o")
        .arg(&provider)
        .arg(&file)
        .output()
        .expect("spawn rustc");
    assert!(output.status.success(), "provider build");
    umbrella::policy::dlopen(&provider).expect("load provider");

    let adapter = handle_adapter("avm_hot", "hot", 1, Some("avm_hot_read"));
    let adapters = [GeneratedAdapter::Handle(&adapter)];
    let first = umbrella::adapters::build_next(&adapters).expect("first build");
    umbrella::adapters::load(&first).expect("load first");
    let second = umbrella::adapters::build_next(&adapters).expect("second build");
    umbrella::adapters::load(&second).expect("load second");

    let first_lib = Library::open(&first.path);
    let new = first_lib.new_fn("avm_hot__new");
    let dispose = first_lib.dispose_fn("avm_hot__dispose");
    let mut before = [0i64; 2];
    assert_eq!(unsafe { new(5, before.as_mut_ptr()) }, 0);
    assert_eq!((before[1] >> 32) as u32, first.generation);

    umbrella::adapters::bind(&first.path, &second, &adapter).expect("bind");
    let mut after = [0i64; 2];
    assert_eq!(unsafe { new(9, after.as_mut_ptr()) }, 0);
    assert_eq!(
        (after[1] >> 32) as u32,
        second.generation,
        "the forwarder now reaches the bound build's table"
    );
    assert_ne!(
        unsafe { dispose(before[0], before[1]) },
        0,
        "the bound table refuses the previous generation's words"
    );
    assert_eq!(unsafe { dispose(after[0], after[1]) }, 0);
}
