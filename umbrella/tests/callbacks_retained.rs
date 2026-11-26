//! U16 (retained half) — instance-lifetime callbacks and the reload hook.
//!
//! A `__with` callback lives for one call. A provider that keeps the function
//! pointer needs an *instance* scope: `__retain` opens one at the wrapper
//! crate's provider generation, keeps the checked token, and `__invoke_retained`
//! is the seam its later call lands on. When the provider is reloaded, the host
//! retires every older build through that build's own
//! `__retire_callbacks` hook, so a retained token is refused before it can call
//! into JIT code that has been replaced.
//!
//! The test drives two real wrapper builds through `load_for_jit`, the same
//! reload path the umbrella uses: the second load must retire the first
//! build's retained callback, and the first build must then refuse it while the
//! second works.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::interface::{GeneratedAdapter, HandleAdapter};

fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "umbrella-retained-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn descriptor(target: &str) -> HandleAdapter {
    let text = format!(
        "schema 1\nabi handle 1\nsymbol {target}__new\ntarget {target}\ntype retained\n\
         constructor {target}_ctor\ndestructor {target}_dtor\nparams 2\n"
    );
    HandleAdapter::parse_descriptors(&text)
        .expect("parse the handle descriptor")
        .remove(0)
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

    fn new_fn(&self, name: &str) -> unsafe extern "C" fn(i64, i64, *mut i64) -> i64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }

    fn retain_fn(
        &self,
        name: &str,
    ) -> unsafe extern "C-unwind" fn(
        i64,
        i64,
        Option<unsafe extern "C-unwind" fn(*mut libc::c_void) -> i64>,
    ) -> i64 {
        unsafe { std::mem::transmute(self.symbol(name)) }
    }

    fn invoke_fn(&self, name: &str) -> unsafe extern "C-unwind" fn(i64, i64, *mut i64) -> i64 {
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

unsafe extern "C-unwind" fn read_context(ptr: *mut libc::c_void) -> i64 {
    *(ptr as *const i64)
}

#[test]
fn a_retained_callback_is_refused_after_the_provider_reload() {
    let dir = scratch("build");
    let provider_src = dir.join("provider.rs");
    std::fs::write(
        &provider_src,
        "#[export_name = \"avm_retained_ctor\"]\n\
         pub extern \"C\" fn retained_ctor(a: i64, b: i64) -> *mut core::ffi::c_void {\n\
             Box::into_raw(Box::new(a + b)) as *mut core::ffi::c_void\n\
         }\n\n\
         #[export_name = \"avm_retained_dtor\"]\n\
         pub extern \"C\" fn retained_dtor(ptr: *mut core::ffi::c_void) {\n\
             drop(unsafe { Box::from_raw(ptr as *mut i64) });\n\
         }\n\n\
         #[export_name = \"avm_retained_next_ctor\"]\n\
         pub extern \"C\" fn retained_next_ctor(a: i64, b: i64) -> *mut core::ffi::c_void {\n\
             Box::into_raw(Box::new(a + b)) as *mut core::ffi::c_void\n\
         }\n\n\
         #[export_name = \"avm_retained_next_dtor\"]\n\
         pub extern \"C\" fn retained_next_dtor(ptr: *mut core::ffi::c_void) {\n\
             drop(unsafe { Box::from_raw(ptr as *mut i64) });\n\
         }\n",
    )
    .expect("write provider");
    let provider = dir.join("libretained_provider.so");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=retained_provider")
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

    // Generation 1 and generation 2 are different targets, so the second
    // `load_for_jit` registers a fresh symbol set instead of colliding with the
    // first build's statically registered names.
    let first_descriptor = descriptor("avm_retained");
    let second_descriptor = descriptor("avm_retained_next");
    let first_wrapper = umbrella::adapters::build_next_against(
        &[GeneratedAdapter::Handle(&first_descriptor)],
        Some(&provider),
    )
    .expect("build generation one");
    umbrella::adapters::load_for_jit(&first_wrapper).expect("load generation one");
    assert!(
        first_wrapper
            .retire_symbols
            .contains(&"avm_retained__with__retire_callbacks".to_string()),
        "the build must export its retirement hook: {:?}",
        first_wrapper.retire_symbols
    );

    let first = Library::open(&first_wrapper.path);
    let new = first.new_fn("avm_retained__new");
    let retain = first.retain_fn("avm_retained__with__retain");
    let invoke = first.invoke_fn("avm_retained__with__invoke_retained");

    // Mint a handle, then retain a callback for the life of the instance.
    let mut words = [0i64; 2];
    assert_eq!(unsafe { new(7, 9, words.as_mut_ptr()) }, 0, "mint a handle");
    let (lo, hi) = (words[0], words[1]);
    assert_eq!(
        unsafe { retain(lo, hi, Some(read_context)) },
        0,
        "the instance scope takes the callback"
    );

    // The provider (here the test, on its behalf) calls it later, twice. The
    // callback reads the provider's own value: 7 + 9.
    let mut value = 0i64;
    assert_eq!(unsafe { invoke(lo, hi, &mut value) }, 0);
    assert_eq!(value, 16, "the retained callback reads the provider's value");
    value = 0;
    assert_eq!(unsafe { invoke(lo, hi, &mut value) }, 0);
    assert_eq!(value, 16, "a retained token is not one-shot");

    // Reload: the host retires generation one as it loads generation two.
    let second_wrapper = umbrella::adapters::build_next_against(
        &[GeneratedAdapter::Handle(&second_descriptor)],
        Some(&provider),
    )
    .expect("build generation two");
    umbrella::adapters::load_for_jit(&second_wrapper).expect("load generation two");

    // The first build's retained callback is now refused, before it runs.
    let mut untouched = 0xdead_beefi64;
    assert_eq!(
        unsafe { invoke(lo, hi, &mut untouched) },
        1,
        "a retired generation's retained callback is refused"
    );
    assert_eq!(untouched, 0xdead_beef, "a refused call runs nothing");

    // The new generation has its own scope and token, and works.
    let second = Library::open(&second_wrapper.path);
    let new_next = second.new_fn("avm_retained_next__new");
    let retain_next = second.retain_fn("avm_retained_next__with__retain");
    let invoke_next = second.invoke_fn("avm_retained_next__with__invoke_retained");
    let mut words = [0i64; 2];
    assert_eq!(unsafe { new_next(1, 2, words.as_mut_ptr()) }, 0);
    let (lo, hi) = (words[0], words[1]);
    assert_eq!(unsafe { retain_next(lo, hi, Some(read_context)) }, 0);
    let mut value = 0i64;
    assert_eq!(unsafe { invoke_next(lo, hi, &mut value) }, 0);
    assert_eq!(value, 3, "the reloaded build retains its own callbacks");
}

/// The provider protocol: `__retain` hands native code a dispatch trampoline
/// and an opaque slot (the descriptor's `subscribe` symbol), the provider
/// stores them, and later calls the trampoline itself. After a reload the
/// provider still holds that pair, and the call must be refused instead of
/// jumping into replaced JIT code. The second generation's subscription
/// replaces the first's, and works.
#[test]
fn a_provider_invokes_the_retained_trampoline_and_is_refused_after_reload() {
    let dir = scratch("provider");
    let provider_src = dir.join("provider.rs");
    std::fs::write(
        &provider_src,
        r#"
use std::sync::Mutex;

type Dispatch =
    unsafe extern "C-unwind" fn(slot: *mut core::ffi::c_void, out: *mut i64) -> i64;

// The provider keeps what the wrapper handed it: a function pointer and an
// opaque slot. Both outlive the subscription call.
static STORED: Mutex<Option<(usize, usize)>> = Mutex::new(None);

#[export_name = "avm_sub_a_ctor"]
pub extern "C" fn a_ctor(a: i64, b: i64) -> *mut core::ffi::c_void {
    Box::into_raw(Box::new(a + b)) as *mut core::ffi::c_void
}

#[export_name = "avm_sub_a_dtor"]
pub extern "C" fn a_dtor(ptr: *mut core::ffi::c_void) {
    drop(unsafe { Box::from_raw(ptr as *mut i64) });
}

#[export_name = "avm_sub_a_subscribe"]
pub extern "C-unwind" fn a_subscribe(
    _owner: *mut core::ffi::c_void,
    dispatch: Dispatch,
    slot: *mut core::ffi::c_void,
) -> i64 {
    *STORED.lock().unwrap() = Some((dispatch as usize, slot as usize));
    0
}

#[export_name = "avm_sub_b_ctor"]
pub extern "C" fn b_ctor(a: i64, b: i64) -> *mut core::ffi::c_void {
    Box::into_raw(Box::new(a + b)) as *mut core::ffi::c_void
}

#[export_name = "avm_sub_b_dtor"]
pub extern "C" fn b_dtor(ptr: *mut core::ffi::c_void) {
    drop(unsafe { Box::from_raw(ptr as *mut i64) });
}

#[export_name = "avm_sub_b_subscribe"]
pub extern "C-unwind" fn b_subscribe(
    _owner: *mut core::ffi::c_void,
    dispatch: Dispatch,
    slot: *mut core::ffi::c_void,
) -> i64 {
    *STORED.lock().unwrap() = Some((dispatch as usize, slot as usize));
    0
}

// The provider's own event: it calls the trampoline it was given.
#[export_name = "avm_sub_fire"]
pub extern "C-unwind" fn fire(out: *mut i64) -> i64 {
    let stored = *STORED.lock().unwrap();
    match stored {
        Some((dispatch, slot)) => {
            let dispatch: Dispatch = unsafe { std::mem::transmute(dispatch) };
            unsafe { dispatch(slot as *mut core::ffi::c_void, out) }
        }
        None => 1,
    }
}
"#,
    )
    .expect("write provider");
    let provider = dir.join("libsub_provider.so");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=sub_provider")
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

    let first_descriptor = HandleAdapter::parse_descriptors(&format!(
        "schema 1\nabi handle 1\nsymbol avm_sub_a__new\ntarget avm_sub_a\ntype sub_a\n\
         constructor avm_sub_a_ctor\ndestructor avm_sub_a_dtor\n\
         subscribe avm_sub_a_subscribe\nparams 2\n"
    ))
    .expect("parse the subscribing descriptor")
    .remove(0);
    let second_descriptor = HandleAdapter::parse_descriptors(
        "schema 1\nabi handle 1\nsymbol avm_sub_b__new\ntarget avm_sub_b\ntype sub_b\n\
         constructor avm_sub_b_ctor\ndestructor avm_sub_b_dtor\n\
         subscribe avm_sub_b_subscribe\nparams 2\n",
    )
    .expect("parse the second descriptor")
    .remove(0);
    assert_eq!(
        first_descriptor.descriptor_text(),
        "schema 1\nabi handle 1\nsymbol avm_sub_a__new\ntarget avm_sub_a\ntype sub_a\n\
         constructor avm_sub_a_ctor\ndestructor avm_sub_a_dtor\n\
         subscribe avm_sub_a_subscribe\nparams 2\n",
        "the canonical text keeps the subscription"
    );

    let first_wrapper = umbrella::adapters::build_next_against(
        &[GeneratedAdapter::Handle(&first_descriptor)],
        Some(&provider),
    )
    .expect("build the subscribing generation");
    umbrella::adapters::load_for_jit(&first_wrapper).expect("load the subscribing generation");

    let wrapper = Library::open(&first_wrapper.path);
    let new = wrapper.new_fn("avm_sub_a__new");
    let retain = wrapper.retain_fn("avm_sub_a__with__retain");
    let mut words = [0i64; 2];
    assert_eq!(unsafe { new(7, 9, words.as_mut_ptr()) }, 0);
    let (lo, hi) = (words[0], words[1]);
    assert_eq!(unsafe { retain(lo, hi, Some(read_context)) }, 0);

    // The provider calls the trampoline it was handed, not a wrapper entry
    // point: this is the protocol, not the host seam.
    let provider_lib = Library::open(&provider);
    let fire: unsafe extern "C-unwind" fn(*mut i64) -> i64 =
        unsafe { std::mem::transmute(provider_lib.symbol("avm_sub_fire")) };
    let mut out = 0i64;
    assert_eq!(unsafe { fire(&mut out) }, 0);
    assert_eq!(out, 16, "the provider's call reached the retained callback");
    out = 0;
    assert_eq!(unsafe { fire(&mut out) }, 0);
    assert_eq!(out, 16, "the provider may call it again");

    // Reload. The provider still holds generation one's trampoline and slot.
    let second_wrapper = umbrella::adapters::build_next_against(
        &[GeneratedAdapter::Handle(&second_descriptor)],
        Some(&provider),
    )
    .expect("build the next subscribing generation");
    umbrella::adapters::load_for_jit(&second_wrapper).expect("load the next generation");

    let mut untouched = 0xdead_beefi64;
    assert_eq!(
        unsafe { fire(&mut untouched) },
        1,
        "the provider's stale trampoline is refused after the reload"
    );
    assert_eq!(untouched, 0xdead_beef, "the refused call ran nothing");

    // The second generation subscribes its own pair, replacing the first's.
    let second = Library::open(&second_wrapper.path);
    let new_next = second.new_fn("avm_sub_b__new");
    let retain_next = second.retain_fn("avm_sub_b__with__retain");
    let mut words = [0i64; 2];
    assert_eq!(unsafe { new_next(1, 2, words.as_mut_ptr()) }, 0);
    let (lo, hi) = (words[0], words[1]);
    assert_eq!(unsafe { retain_next(lo, hi, Some(read_context)) }, 0);
    let mut out = 0i64;
    assert_eq!(unsafe { fire(&mut out) }, 0);
    assert_eq!(out, 3, "the reloaded provider reaches the new callback");
}

/// The descriptor's `callback` line, measured twice: in the rendered source
/// (the policy reaches both `__with` and the retained scope, and
/// `lifetime=call` removes the retained API) and at run time (the default
/// caller affinity refuses a call from another thread, `thread=any` allows it).
#[test]
fn the_descriptor_selects_the_callback_policy() {
    let text = "schema 1\nabi handle 1\nsymbol avm_thr_a__new\ntarget avm_thr_a\ntype thr_a\n\
                constructor avm_thr_a_ctor\ndestructor avm_thr_a_dtor\n\
                callback thread=any reentrancy=nested error=retire\nparams 2\n";
    let adapter = HandleAdapter::parse_descriptors(text)
        .expect("parse the policy descriptor")
        .remove(0);
    assert_eq!(
        adapter.descriptor_text(),
        text,
        "the canonical text keeps the callback policy"
    );
    let rendered = GeneratedAdapter::Handle(&adapter)
        .render()
        .expect("render the policy wrapper");
    assert!(
        rendered.contains("thread: ThreadAffinity::AnyThread")
            && rendered.contains("error: ErrorPolicy::RetireOnPanic")
            && rendered.contains("reentrancy: Reentrancy::Nested"),
        "the `__with` scope must use the descriptor's policy:\n{rendered}"
    );
    assert!(
        rendered.contains("__retain"),
        "an instance lifetime keeps the retained API"
    );

    let mut call_scoped = adapter.clone();
    call_scoped.callback.lifetime = Some(umbrella::interface::CallbackLifetime::Call);
    let generated = GeneratedAdapter::Handle(&call_scoped);
    let rendered = generated.render().expect("render the call-scoped wrapper");
    assert!(
        !rendered.contains("__retain") && !rendered.contains("__invoke_retained"),
        "a call-scoped descriptor must not render a retained API:\n{rendered}"
    );
    assert!(generated.retirement_symbols().is_empty());

    // A `subscribe` symbol needs retention: the provider cannot be handed a
    // callback the descriptor says cannot outlive the call.
    let mut contradictory = call_scoped.clone();
    contradictory.subscribe_symbol = Some("avm_thr_a_subscribe".to_string());
    assert!(
        contradictory.validate().is_err(),
        "`lifetime=call` with `subscribe` must be refused"
    );

    // Runtime: the default policy is the registration thread, and a spawned
    // thread is refused before the callback runs.
    let dir = scratch("policy");
    let provider_src = dir.join("provider.rs");
    std::fs::write(
        &provider_src,
        r#"
#[export_name = "avm_thr_a_ctor"]
pub extern "C" fn a_ctor(a: i64, b: i64) -> *mut core::ffi::c_void {
    Box::into_raw(Box::new(a + b)) as *mut core::ffi::c_void
}

#[export_name = "avm_thr_a_dtor"]
pub extern "C" fn a_dtor(ptr: *mut core::ffi::c_void) {
    drop(unsafe { Box::from_raw(ptr as *mut i64) });
}

#[export_name = "avm_thr_b_ctor"]
pub extern "C" fn b_ctor(a: i64, b: i64) -> *mut core::ffi::c_void {
    Box::into_raw(Box::new(a + b)) as *mut core::ffi::c_void
}

#[export_name = "avm_thr_b_dtor"]
pub extern "C" fn b_dtor(ptr: *mut core::ffi::c_void) {
    drop(unsafe { Box::from_raw(ptr as *mut i64) });
}
"#,
    )
    .expect("write provider");
    let provider = dir.join("libpolicy_provider.so");
    let out = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("--crate-name=policy_provider")
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

    let caller_default = HandleAdapter::parse_descriptors(
        "schema 1\nabi handle 1\nsymbol avm_thr_a__new\ntarget avm_thr_a\ntype thr_a\n\
         constructor avm_thr_a_ctor\ndestructor avm_thr_a_dtor\nparams 2\n",
    )
    .expect("parse the default-policy descriptor")
    .remove(0);
    let any_thread = HandleAdapter::parse_descriptors(
        "schema 1\nabi handle 1\nsymbol avm_thr_b__new\ntarget avm_thr_b\ntype thr_b\n\
         constructor avm_thr_b_ctor\ndestructor avm_thr_b_dtor\n\
         callback thread=any\nparams 2\n",
    )
    .expect("parse the any-thread descriptor")
    .remove(0);

    for adapter in [&caller_default, &any_thread] {
        let built = umbrella::adapters::build_next_against(
            &[GeneratedAdapter::Handle(adapter)],
            Some(&provider),
        )
        .expect("build the policy wrapper");
        umbrella::adapters::load_for_jit(&built).expect("load the policy wrapper");
        let lib = Library::open(&built.path);
        let new = lib.new_fn(&adapter.symbol);
        // The instance scope is opened on *this* thread; the invocation below
        // runs on another one, which is where the affinity is real (inside a
        // single `__with` the scope is opened by the caller, so affinity never
        // differs).
        let retain = lib.retain_fn(&format!("{}__with__retain", adapter.target));
        let invoke = lib.invoke_fn(&format!("{}__with__invoke_retained", adapter.target));
        let mut words = [0i64; 2];
        assert_eq!(unsafe { new(7, 9, words.as_mut_ptr()) }, 0);
        let (lo, hi) = (words[0], words[1]);
        assert_eq!(unsafe { retain(lo, hi, Some(read_context)) }, 0);
        let any = adapter.callback.thread == Some(umbrella::interface::CallbackThread::Any);
        let (status, value) = std::thread::spawn(move || {
            let mut out = 0i64;
            let status = unsafe { invoke(lo, hi, &mut out) };
            (status, out)
        })
        .join()
        .expect("the wrapper call must not panic");
        if any {
            assert_eq!(status, 0, "`thread=any` accepts a call from another thread");
            assert_eq!(value, 16, "the callback ran on that thread");
        } else {
            assert_eq!(status, 1, "the default caller affinity refuses another thread");
            assert_eq!(value, 0, "a refused call ran nothing");
        }
    }
}
