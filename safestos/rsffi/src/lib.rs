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

#[cfg(test)]
mod tests {
    use super::*;

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
