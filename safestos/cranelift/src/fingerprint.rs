//! The code fingerprint: sha256 over the bytes the JIT is handed.
//!
//! W5 of `PLAN_sota_kernel_verification.md` asks for the certificate's `code=`
//! line to be *checked* rather than merely *recorded*. The distinction is who
//! computes the number. While the emitter's own tooling hashed the file it had
//! just written, the digest was a restatement of a byte count — the artifact
//! and the record could agree while both were wrong about what the JIT would
//! actually execute, because nothing in the loop ever looked at the bytes from
//! the side that runs them.
//!
//! This export is that side. `scripts/cps-fingerprint` loads this cdylib and
//! asks *it* for the digest of a CPS file, so the gate's number comes from the
//! same crate that compiles and executes those bytes, and a disagreement with
//! the emitter's independent `hashlib` implementation is a real disagreement
//! between two implementations rather than a file compared to itself.
//!
//! Deliberately not a JIT symbol (`uk_*`/`au_*`): compiled Austral code must
//! never be able to call it, so it needs no manifest grant and no entry in
//! `UNFER_SYMBOLS` / `SYMBOL_REGISTRY`. It is an FFI export of the same kind as
//! `austral_defeq_translate`.

use std::ffi::CString;
use std::os::raw::c_char;

use sha2::{Digest, Sha256};

/// Hex sha256 of `len` bytes at `bytes`, or NULL when `bytes` is NULL.
///
/// The returned string is owned by the bridge: free it with
/// [`crate::cranelift_free_string`].
#[no_mangle]
pub extern "C" fn austral_cps_fingerprint(bytes: *const u8, len: usize) -> *mut c_char {
    if bytes.is_null() {
        return std::ptr::null_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    let mut hasher = Sha256::new();
    hasher.update(slice);
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    match CString::new(hex) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The digest of the empty input, so a change to the primitive is visible
    /// against a value that is public rather than against itself.
    #[test]
    fn empty_input_matches_the_published_digest() {
        let out = austral_cps_fingerprint(b"".as_ptr(), 0);
        assert!(!out.is_null());
        let s = unsafe { CString::from_raw(out) }
            .to_string_lossy()
            .to_string();
        assert_eq!(
            s,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_single_flipped_byte_changes_the_digest() {
        let a = austral_cps_fingerprint(b"cps".as_ptr(), 3);
        let b = austral_cps_fingerprint(b"cpT".as_ptr(), 3);
        let a = unsafe { CString::from_raw(a) }
            .to_string_lossy()
            .to_string();
        let b = unsafe { CString::from_raw(b) }
            .to_string_lossy()
            .to_string();
        assert_ne!(a, b);
    }

    #[test]
    fn null_bytes_is_refused_not_dereferenced() {
        assert!(austral_cps_fingerprint(std::ptr::null(), 0).is_null());
    }
}
