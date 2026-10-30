//! PLAN_valen §4 Tier 1 — the reference `extern rust` crate.
//!
//! This crate is the tier-1 shape made executable: an Austral module declares
//! `extern rust fn` whose parameters and return are restricted to the buffer
//! protocol the kernel already speaks (`Address[T]` + `Int64` pairs, or plain
//! `Int64` scalars), and each declaration carries an ownership annotation
//! (`borrowed` / `owned`) that the compiler checks on the MTAST and erases
//! before CPS (same erasure discipline as Plan L's group regions).
//!
//! The Austral side guarantees the *shape* of what crosses the boundary. The
//! safety story on this side is each function's own `unsafe` contract,
//! stated per function below.
//!
//! Registration: the bridge (`safestos/cranelift`) registers these symbols in
//! the Cranelift JIT under the `rs_` namespace via the data-driven
//! `RS_FFI_SYMBOLS` table — the same mechanism as `UNFER_SYMBOLS` (`uk_*`).
//! Authorization is inherited, not new: every gated call passes
//! `check_call_permission` under the calling module's principal, and a
//! manifest that grants `uk_push` does not thereby grant an `rs_*` call
//! (PLAN_valen §4, "Authorization is inherited, not new").

/// FNV-1a 64-bit basis (its offset basis, to be exact).
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Tier-1 reference: checksum over a borrowed buffer pair.
///
/// # Ownership shape (checked on the Austral side, erased before CPS)
///
/// `buf`/`len` are one linear cell crossing the boundary as a `borrowed`
/// pair: the caller keeps the cell across the call, and this function must
/// not retain either half. Borrows cannot outlive the call.
///
/// # Safety contract (this side)
///
/// * If `len > 0`, `buf` must point to at least `len` readable bytes.
/// * The buffer must not be concurrently mutated for the duration of the
///   call (the caller's linearity gives this: a borrowed cell has no alias).
///
/// Both are shape violations on the caller's side; the function still
/// refuses null and negative lengths rather than trusting them, so a
/// violating caller gets `-1`, not UB.
#[no_mangle]
pub unsafe extern "C" fn rs_checksum(buf: *const u8, len: i64) -> i64 {
    if buf.is_null() || len < 0 {
        return -1;
    }
    let slice = unsafe { std::slice::from_raw_parts(buf, len as usize) };
    let mut hash: u64 = FNV_OFFSET_BASIS;
    for &b in slice {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as i64
}

/// Tier-1 reference: scalar-only case (no buffer crosses).
///
/// Returns the reference crate's version: the tier's shape allows plain
/// `Int64` parameters and returns, not only buffer pairs.
#[no_mangle]
pub extern "C" fn rs_version() -> i64 {
    1
}

/// The measurement primitive: the first 8 bytes of SHA-256 over the
/// buffer, big-endian, as an `Int64`.
///
/// The input is the **canonical `svcmux-bundle-v1` preimage** —
/// `"svcmux-bundle-v1" ++ NUL ++ id ++ NUL ++ binary ++ NUL ++ config ++
/// NUL ++ state_root ++ NUL ++ listen` — so the tag equals the first 8
/// bytes of the bundle digest that `svcmux` and each webservice's
/// `/attestation` endpoint publish in full. An Austral module holding the
/// three services' descriptors can therefore compare its computed tags
/// against an independently measured manifest without ever seeing a
/// 32-byte digest: 64 bits are enough to state *distinctness*, which is
/// what the isolation certificate needs.
///
/// # Ownership shape (checked on the Austral side, erased before CPS)
///
/// `buf`/`len` are a `borrowed` pair: the caller keeps the cell, this
/// function retains nothing — same contract as `rs_checksum`.
///
/// # Safety contract (this side)
///
/// * If `len > 0`, `buf` must point to at least `len` readable bytes.
/// * Null or negative lengths are refused with `-1` rather than trusted.
#[no_mangle]
pub unsafe extern "C" fn rs_bundle_tag(buf: *const u8, len: i64) -> i64 {
    use sha2::{Digest, Sha256};
    if buf.is_null() || len < 0 {
        return -1;
    }
    let slice = unsafe { std::slice::from_raw_parts(buf, len as usize) };
    let digest = Sha256::digest(slice);
    i64::from_be_bytes(digest[0..8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tag is the first 8 bytes of the bundle digest: pin it against
    /// the svcmux-bundle-v1 preimage for `svc-a` — the same preimage whose
    /// full digest (6820fb2a…) svcmux and unfer_measure both pin as their
    /// vector, so all three crates agree on one measurement.
    #[test]
    fn bundle_tag_is_the_first_64_bits_of_the_bundle_digest() {
        let mut preimage: Vec<u8> = Vec::new();
        for (i, part) in [
            "svcmux-bundle-v1",
            "svc-a",
            &"0123456789abcdef".repeat(4),
            &"fedcba9876543210".repeat(4),
            "/run/svc-a",
            "127.0.0.1:8080",
        ]
        .iter()
        .enumerate()
        {
            if i > 0 {
                preimage.push(0);
            }
            preimage.extend_from_slice(part.as_bytes());
        }
        // Full digest, known from the pinned vectors:
        // 6820fb2ab8939587369b84d2a7e6756a468f7532af459a8ef76c7a8a86c31901
        // → first 8 bytes 0x6820fb2ab8939587 as i64.
        let expected = i64::from_be_bytes([
            0x68, 0x20, 0xfb, 0x2a, 0xb8, 0x93, 0x95, 0x87,
        ]);
        unsafe {
            assert_eq!(rs_bundle_tag(preimage.as_ptr(), preimage.len() as i64), expected);
        }
        // A violating caller gets -1, not UB.
        unsafe {
            assert_eq!(rs_bundle_tag(std::ptr::null(), 4), -1);
            assert_eq!(rs_bundle_tag(preimage.as_ptr(), -1), -1);
        }
    }

    #[test]
    fn checksum_matches_canonical_fnv1a_vectors() {
        // Canonical FNV-1a 64 test vectors, so the reference crate and any
        // independent reimplementation (test suites, hosts) must agree.
        unsafe {
            assert_eq!(rs_checksum(b"a".as_ptr(), 1), 0xaf63_dc4c_8601_ec8c_u64 as i64);
            assert_eq!(rs_checksum(b"au!".as_ptr(), 3), 0xe745_8419_0561_f38e_u64 as i64);
            assert_eq!(rs_checksum(b"".as_ptr(), 0), FNV_OFFSET_BASIS as i64);
        }
    }

    #[test]
    fn shape_violations_return_minus_one_not_ub() {
        unsafe {
            assert_eq!(rs_checksum(std::ptr::null(), 4), -1);
            assert_eq!(rs_checksum(b"a".as_ptr(), -1), -1);
        }
    }

    #[test]
    fn version_is_scalar_one() {
        assert_eq!(rs_version(), 1);
    }
}
