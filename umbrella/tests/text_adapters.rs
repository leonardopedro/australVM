//! U21 text transport: `&str` in, owned `String` out, through a generated
//! wrapper and a real provider cdylib.
//!
//! The provider is written the way an ordinary Rust library would be
//! (`extern "C" fn(&str) -> String`), the wrapper is rendered from an
//! `abi text 1` descriptor block, compiled by `adapters::build_at` against that
//! provider, and then dlopened and called. The wire side is two words in
//! (pointer, length) and one boxed pair out, read here at the documented
//! offsets and released with `<symbol>__text_free`.

use std::ffi::{c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::adapters;
use umbrella::interface::{GeneratedAdapter, TextAdapter};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("umbrella-text-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn build_provider(dir: &Path) -> PathBuf {
    let source = dir.join("provider.rs");
    std::fs::write(
        &source,
        r#"
#[no_mangle]
pub extern "C" fn rs_greet(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!("{text}! ({} bytes)", text.len())
}
"#,
    )
    .expect("write provider source");
    let library = dir.join("libgreet.so");
    let output = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("-o")
        .arg(&library)
        .arg(&source)
        .output()
        .expect("spawn rustc");
    assert!(
        output.status.success(),
        "provider rustc: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    library
}

/// A dlopened artifact; `get` resolves a symbol from that exact image.
struct Library(*mut c_void);

impl Library {
    fn open(path: &Path) -> Self {
        let path = CString::new(path.to_str().expect("utf-8 path")).unwrap();
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            let reason = unsafe { CStr::from_ptr(libc::dlerror()) }
                .to_string_lossy()
                .into_owned();
            panic!("dlopen: {reason}");
        }
        Library(handle)
    }

    fn get(&self, name: &str) -> *mut c_void {
        let name = CString::new(name).unwrap();
        let entry = unsafe { libc::dlsym(self.0, name.as_ptr()) };
        assert!(!entry.is_null(), "dlsym {name:?}");
        entry
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.0);
        }
    }
}

/// The pair `TextOut` the wrapper returns: two `u64`s, pointer then length.
#[repr(C)]
struct TextOut {
    ptr: *mut u8,
    len: u64,
}

type TextFn = unsafe extern "C" fn(*const u8, u64) -> *mut TextOut;
type FreeFn = unsafe extern "C" fn(*mut TextOut);

fn render_and_build(dir: &Path) -> (Library, TextFn, FreeFn) {
    let provider = build_provider(dir);
    let document = "\
schema 1
abi text 1
symbol rs_greet__adapter
target rs_greet
params Text
returns Text
";
    let texts = TextAdapter::parse_descriptors(document).expect("parse text block");
    assert_eq!(texts.len(), 1);
    assert_eq!(
        texts[0].canonical_text(),
        document,
        "the block round-trips canonically"
    );
    let adapters = vec![GeneratedAdapter::Text(&texts[0])];
    let built = adapters::build_at(&adapters, &dir.join("built"), 1, Some(&provider))
        .expect("build the text wrapper");
    assert!(
        built.exports.contains(&"rs_greet__adapter".to_string())
            && built
                .exports
                .contains(&"rs_greet__adapter__text_free".to_string()),
        "the pair and its release are the crate's exports: {:?}",
        built.exports
    );
    let library = Library::open(&built.path);
    let wrapper: TextFn = unsafe { std::mem::transmute(library.get("rs_greet__adapter")) };
    let free: FreeFn = unsafe { std::mem::transmute(library.get("rs_greet__adapter__text_free")) };
    (library, wrapper, free)
}

fn call(wrapper: TextFn, free: FreeFn, text: &str) -> Option<String> {
    let out = unsafe { wrapper(text.as_ptr(), text.len() as u64) };
    if out.is_null() {
        return None;
    }
    let pair = unsafe { &*out };
    let bytes = unsafe { std::slice::from_raw_parts(pair.ptr, pair.len as usize) };
    let answer = String::from_utf8(bytes.to_vec()).expect("answer is UTF-8");
    unsafe { free(out) };
    Some(answer)
}

#[test]
fn a_string_crosses_the_generated_text_adapter() {
    let dir = scratch("roundtrip");
    let (_library, wrapper, free) = render_and_build(&dir);
    assert_eq!(
        call(wrapper, free, "hello"),
        Some("hello! (5 bytes)".to_string())
    );
    // Multi-byte UTF-8 is measured in bytes, not chars: the wrapper hands the
    // provider a real `&str` slice, so its `len()` is the byte count.
    assert_eq!(
        call(wrapper, free, "héllo"),
        Some("héllo! (6 bytes)".to_string())
    );
    // The empty answer is a non-null pair of length zero, not a refusal.
    assert_eq!(call(wrapper, free, ""), Some(String::new()));
}

#[test]
fn a_refused_text_argument_is_null_never_a_guessed_string() {
    let dir = scratch("refusals");
    let (_library, wrapper, free) = render_and_build(&dir);

    // Invalid UTF-8: the wrapper validates before calling the provider.
    let invalid = [0xffu8, 0xfe];
    let out = unsafe { wrapper(invalid.as_ptr(), invalid.len() as u64) };
    assert!(out.is_null(), "invalid UTF-8 is a refusal");

    // A length with no pointer cannot describe bytes.
    let out = unsafe { wrapper(std::ptr::null(), 5) };
    assert!(out.is_null(), "null pointer with a length is a refusal");

    // Null with length 0 is the empty string by the transport's convention.
    assert_eq!(call(wrapper, free, ""), Some(String::new()));
}

#[test]
fn text_descriptor_shapes_the_schema_cannot_carry_are_refused() {
    for (document, needle) in [
        (
            "schema 1\nabi text 1\nsymbol a\ntarget t\nparams Text,Text\nreturns Text\n",
            "exactly one `Text` parameter",
        ),
        (
            "schema 1\nabi text 1\nsymbol a\ntarget t\nparams Int64\nreturns Text\n",
            "exactly one `Text` parameter",
        ),
        (
            "schema 1\nabi text 1\nsymbol a\ntarget t\nparams Text\nreturns Unit\n",
            "returns an owned `Text`",
        ),
        (
            "schema 1\nabi text 9\nsymbol a\ntarget t\nparams Text\nreturns Text\n",
            "knows 1 through 1",
        ),
        (
            "schema 1\nabi text 1\nsymbol a\ntarget t\nparams Text\nreturns Text\nstem rs_extra\n",
            "unknown text descriptor field `stem`",
        ),
    ] {
        let error = TextAdapter::parse_descriptors(document).unwrap_err();
        assert!(
            error.contains(needle),
            "{document:?}: expected {needle:?}, got {error:?}"
        );
    }
}
