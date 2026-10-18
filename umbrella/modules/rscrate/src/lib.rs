//! The crate the umbrella builds and dlopens (PLAN_valen §5 P3).
//!
//! Ordinary Rust with no knowledge of australVM: extern "C" functions in
//! the `rs_*` namespace, Tier-1 buffer-protocol shape from PLAN_valen
//! §4 — borrowed pointer + borrowed length in, owned scalar out.
//!
//! `rs_demo_checksum` is deliberately NOT the reference crate's
//! `rs_checksum`: nothing here can resolve through the bridge's baked
//! rs_* symbol table, so a granted run proves the umbrella mechanism
//! itself (cargo artifact → dlopen → dlsym fallback → gated call).

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over `len` bytes at `buf`, returned as the raw 64-bit hash
/// interpreted as i64 (the JIT passes everything as i64).
#[no_mangle]
pub extern "C" fn rs_demo_checksum(buf: *const u8, len: i64) -> i64 {
    if buf.is_null() || len <= 0 {
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(buf, len as usize) };
    let mut h = FNV_OFFSET_BASIS;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "au!" vector the Demo module feeds across the boundary. The
    /// expected value was computed independently (it is the canonical
    /// FNV-1a of those three bytes), so the test pins the algorithm, not
    /// the implementation's own output.
    #[test]
    fn au_vector() {
        assert_eq!(rs_demo_checksum(b"au!".as_ptr(), 3), -1781872834555808882);
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(rs_demo_checksum(std::ptr::null(), 3), 0);
        assert_eq!(rs_demo_checksum(b"au!".as_ptr(), 0), 0);
    }
}