//! U11: the scalar adapter as an exported C surface.
//!
//! Generated wrappers are Rust, but they are produced by a generator that also
//! has to describe the encoding to non-Rust callers and fixtures. These tests
//! exercise the three exported functions exactly as a C caller would: numeric
//! type tags, bit-pattern payloads, and a refusal that writes nothing.

use austral_cranelift_bridge::adapters::{
    au_scalar_abi_version, au_scalar_decode, au_scalar_encode, au_scalar_type_tag,
    SCALAR_ABI_VERSION,
};
use austral_cranelift_bridge::cranelift_last_error;

const UNIT: u32 = 0;
const BOOL: u32 = 1;
const I32: u32 = 2;
const U32: u32 = 3;
const I64: u32 = 4;
const U64: u32 = 5;
const F32: u32 = 6;
const F64: u32 = 7;

fn err() -> String {
    let p = cranelift_last_error();
    if p.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    }
}

fn encode(ty: u32, payload: u64) -> Result<u64, String> {
    let mut out = u64::MAX;
    let rc = au_scalar_encode(ty, payload, &mut out);
    if rc == 0 {
        Ok(out)
    } else {
        assert_eq!(out, u64::MAX, "a refused encode must not write its output");
        Err(err())
    }
}

fn decode(ty: u32, word: u64) -> Result<u64, String> {
    let mut out = u64::MAX;
    let rc = au_scalar_decode(ty, word, &mut out);
    if rc == 0 {
        Ok(out)
    } else {
        assert_eq!(out, u64::MAX, "a refused decode must not write its output");
        Err(err())
    }
}

#[test]
fn the_c_surface_reports_the_version_it_was_built_with() {
    assert_eq!(au_scalar_abi_version(), SCALAR_ABI_VERSION);
    assert_eq!(SCALAR_ABI_VERSION, 1);
}

#[test]
fn type_tags_roundtrip_and_unknown_tags_are_refused() {
    for tag in [UNIT, BOOL, I32, U32, I64, U64, F32, F64] {
        assert_eq!(au_scalar_type_tag(tag), tag as i32);
    }
    assert_eq!(au_scalar_type_tag(8), -1);
}

#[test]
fn integers_cross_the_c_surface_with_their_extension_rules() {
    assert_eq!(encode(I32, (-7i64) as u64).expect("encode"), 0xffff_ffff_ffff_fff9);
    assert_eq!(decode(I32, 0xffff_ffff_ffff_fff9).expect("decode"), 0xffff_fff9);
    // Every valid C return shape for an i32: zero-extended and sign-extended.
    assert_eq!(decode(I32, 0xffff_fff9).expect("zero-extended"), 0xffff_fff9);
    let e = decode(I32, 0x0000_0001_0000_0000).expect_err("mixed extension");
    assert!(e.contains("copy of the sign bit"), "{e}");
    assert_eq!(encode(U32, u32::MAX as u64).expect("encode"), 0xffff_ffff);
    assert!(decode(U32, 0x1_0000_0000).is_err());
    assert_eq!(decode(I64, i64::MIN as u64).expect("decode"), i64::MIN as u64);
    assert_eq!(decode(U64, u64::MAX).expect("decode"), u64::MAX);
}

#[test]
fn booleans_are_tags_at_the_c_surface() {
    assert_eq!(encode(BOOL, 1).expect("true"), 1);
    assert_eq!(encode(BOOL, 0).expect("false"), 0);
    let e = encode(BOOL, 2).expect_err("tag 2 is not a bool");
    assert!(e.contains("must be 0 or 1"), "{e}");
    assert!(decode(BOOL, 2).is_err());
}

#[test]
fn floats_cross_as_bit_patterns_and_refuse_garbage() {
    let nan_bits = 0x7ff8_1234_5678_9abc_u64;
    assert_eq!(encode(F64, nan_bits).expect("encode nan"), nan_bits);
    assert_eq!(decode(F64, nan_bits).expect("decode nan"), nan_bits);
    assert_eq!(encode(F64, (-0.0f64).to_bits()).expect("encode -0"), 0x8000_0000_0000_0000);
    assert_eq!(encode(F32, 0x8000_0000).expect("encode f32 -0"), 0x8000_0000);
    assert!(decode(F32, 0x0000_0001_0000_0000).is_err());
}

#[test]
fn unit_and_null_outputs_are_refused() {
    assert!(encode(UNIT, 0).is_err());
    assert!(decode(UNIT, 0).is_err());
    let rc = au_scalar_encode(I64, 1, std::ptr::null_mut());
    assert_eq!(rc, 1);
    assert!(err().contains("null output pointer"), "{}", err());
    let rc = au_scalar_decode(I64, 1, std::ptr::null_mut());
    assert_eq!(rc, 1);
}
