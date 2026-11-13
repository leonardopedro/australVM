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

/// The object digest: sha256 over the `AVCE` capture of a CPS module — i.e. the
/// machine code cranelift actually emitted for those bytes, not a record of a
/// file on disk.
///
/// The capture is host-native (`cranelift_native`), so this number describes
/// *this* architecture's codegen: `austral_object_arch` names it, and the
/// certificate records both. A reader on another architecture can re-derive
/// nothing from the digest, which is why the artifact says so instead of
/// pretending the number is portable.
///
/// Reproducibility caveat, measured: cranelift-module allocates `FuncId`s from a
/// process-global counter, so the *second* module compiled in one process
/// encodes different declaration indices than the first and fingerprints
/// differently. The digest is therefore defined for the first compilation in a
/// fresh process — which is what `scripts/cps-fingerprint` always does (one
/// call, then exit) and what `verify-invariants` compares. A caller that needs
/// it twice should fork rather than assume stability.
///
/// Returns NULL when the module does not compile (there is then no object to
/// fingerprint, and the artifact must fall back to `obj=unrecorded`).
#[no_mangle]
pub extern "C" fn austral_object_fingerprint(bytes: *const u8, len: usize) -> *mut c_char {
    if bytes.is_null() {
        return std::ptr::null_mut();
    }
    let ir = unsafe { std::slice::from_raw_parts(bytes, len) };
    let mut cap = match crate::capture::CaptureModule::new() {
        Ok(cap) => cap,
        Err(_) => return std::ptr::null_mut(),
    };
    if crate::cps::compile_cps_to_clif(&mut cap, ir).is_err() {
        return std::ptr::null_mut();
    }
    if cap.defined().is_empty() {
        return std::ptr::null_mut();
    }
    let object = cap.encode_avce();
    austral_cps_fingerprint(object.as_ptr(), object.len())
}

/// Host architecture the object digest was taken on (`x86_64`, `aarch64`, …).
#[no_mangle]
pub extern "C" fn austral_object_arch() -> *mut c_char {
    let arch = match crate::capture::CaptureModule::new() {
        Ok(cap) => cap.arch_name(),
        Err(_) => return std::ptr::null_mut(),
    };
    match CString::new(arch) {
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
        assert!(austral_object_fingerprint(std::ptr::null(), 0).is_null());
    }

    /// The minimal valid v2 module: magic, module name, one function returning
    /// 42 (same shape as `lib.rs::compile_guard_tests::healthy_module_ir`).
    fn tiny_cps() -> Vec<u8> {
        let mut ir: Vec<u8> = Vec::new();
        ir.extend_from_slice(&0x43505332u32.to_le_bytes()); // magic v2
        ir.extend_from_slice(&2u32.to_le_bytes());
        ir.extend_from_slice(b"m0");
        ir.extend_from_slice(&1u32.to_le_bytes()); // func count
        ir.extend_from_slice(&2u32.to_le_bytes());
        ir.extend_from_slice(b"f0");
        ir.extend_from_slice(&0u32.to_le_bytes()); // param count
        ir.push(0); // return type i64
        let mut body: Vec<u8> = vec![0x07u8, 0x01]; // return (int_lit 42)
        body.extend_from_slice(&42i64.to_le_bytes());
        ir.extend_from_slice(&(body.len() as u32).to_le_bytes());
        ir.extend_from_slice(&body);
        ir
    }
    /// Stability across runs is a property of *fresh processes* (see the
    /// caveat on `austral_object_fingerprint`: the second module in a process
    /// gets different declaration ids), so it is checked where it is true —
    /// `verify-invariants` re-derives the flux1d digest in a new process every
    /// run and compares it to the pinned value. What a unit test can hold is
    /// that a module produces a well-formed digest and arch, and that two
    /// different modules do not collide.
    #[test]
    fn the_object_digest_is_well_formed_and_arch_named() {
        let ir = tiny_cps();
        let a = austral_object_fingerprint(ir.as_ptr(), ir.len());
        assert!(!a.is_null(), "tiny CPS failed to compile");
        let a = unsafe { CString::from_raw(a) }
            .to_string_lossy()
            .to_string();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));

        let mut other = ir.clone();
        let last = other.len() - 1;
        other[last] ^= 0x01; // a different literal in the body
        let b = austral_object_fingerprint(other.as_ptr(), other.len());
        if !b.is_null() {
            let b = unsafe { CString::from_raw(b) }
                .to_string_lossy()
                .to_string();
            assert_ne!(a, b, "two different modules must not collide");
        }

        let arch = austral_object_arch();
        assert!(!arch.is_null());
        let arch = unsafe { CString::from_raw(arch) }
            .to_string_lossy()
            .to_string();
        assert!(!arch.is_empty());
    }
}
