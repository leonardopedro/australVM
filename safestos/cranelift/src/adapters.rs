//! U11 — versioned scalar adapters for the 64-bit word transport.
//!
//! The baseline transport the bridge offers is host-native 64-bit words in
//! and one word out. A native function written with *typed* scalar
//! parameters cannot be called by handing it a word: `f64` travels in a
//! vector register, a `bool` is not any integer, and a narrower integer must
//! be extended by whoever marshals it. This module is the one place that
//! decides how each scalar becomes a word and back, and it is versioned
//! (`SCALAR_ABI_VERSION`) so that a compiler-side allowance can name the
//! encoding it means.
//!
//! Rules, all of them deliberate:
//!
//! * integers are **extended** on the way out (sign for `I32`, zero for
//!   `U32`) and **checked** on the way in — a word whose high half is neither
//!   zero nor a copy of the sign bit is not an `Int32` result;
//! * `Bool` is a tag, and only `0`/`1` are valid — a native function that
//!   returns `2` is out of contract, not "true";
//! * floats are **bit-preserving** (`to_bits`/`from_bits`), never a numeric
//!   `as i64` cast, so `-0.0`, infinities and NaN payloads survive;
//! * `Unit` has no word at all: a unit result is the absence of one, not a
//!   zero the caller could mistake for a value.
//!
//! This module does not yet re-admit these types in the compiler: an `rs_*`
//! import still calls the native function *directly*, and a direct call to
//! `f64`-typed Rust from word-shaped Cranelift code is exactly the guess U04
//! refuses. U10's versioned descriptor (and a generated wrapper that uses
//! these adapters) is what makes the allowance safe; until then
//! `lib/TypingPass.ml` keeps rejecting those signatures.

use crate::set_last_error;

/// Version of the scalar encoding defined below. Bump it when any rule in
/// this file changes, because a compiler-side allowance names a version.
pub const SCALAR_ABI_VERSION: u32 = 1;

/// A scalar type at the adapter boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    Unit,
    Bool,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl ScalarType {
    pub fn name(self) -> &'static str {
        match self {
            ScalarType::Unit => "Unit",
            ScalarType::Bool => "Bool",
            ScalarType::I32 => "Int32",
            ScalarType::U32 => "Nat32",
            ScalarType::I64 => "Int64",
            ScalarType::U64 => "Nat64",
            ScalarType::F32 => "Float32",
            ScalarType::F64 => "Float64",
        }
    }

    /// How many transport words this type occupies. `Unit` occupies none.
    pub fn words(self) -> usize {
        match self {
            ScalarType::Unit => 0,
            _ => 1,
        }
    }
}

/// A typed scalar value, already decoded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScalarValue {
    Unit,
    Bool(bool),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
}

impl ScalarValue {
    pub fn ty(self) -> ScalarType {
        match self {
            ScalarValue::Unit => ScalarType::Unit,
            ScalarValue::Bool(_) => ScalarType::Bool,
            ScalarValue::I32(_) => ScalarType::I32,
            ScalarValue::U32(_) => ScalarType::U32,
            ScalarValue::I64(_) => ScalarType::I64,
            ScalarValue::U64(_) => ScalarType::U64,
            ScalarValue::F32(_) => ScalarType::F32,
            ScalarValue::F64(_) => ScalarType::F64,
        }
    }
}

/// Why an adapter refused a value or a word. Every refusal names the type and
/// what was wrong, because a marshalling bug otherwise reads as a wrong value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    /// The logical value does not have the type the signature declared.
    TypeMismatch {
        expected: ScalarType,
        got: ScalarType,
    },
    /// The word is not a valid encoding of this type.
    InvalidWord {
        ty: ScalarType,
        word: u64,
        detail: &'static str,
    },
    /// `Unit` was asked for a word (or a word for `Unit`).
    UnitHasNoWord,
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::TypeMismatch { expected, got } => write!(
                f,
                "adapter: expected {} but the value is {}",
                expected.name(),
                got.name()
            ),
            AdapterError::InvalidWord { ty, word, detail } => write!(
                f,
                "adapter: 0x{word:016x} is not a valid {} word: {detail}",
                ty.name()
            ),
            AdapterError::UnitHasNoWord => f.write_str("adapter: Unit occupies no transport word"),
        }
    }
}

/// Encode a typed value as the word a native call receives.
pub fn encode(ty: ScalarType, value: ScalarValue) -> Result<u64, AdapterError> {
    if ty == ScalarType::Unit {
        return Err(AdapterError::UnitHasNoWord);
    }
    if value.ty() != ty {
        return Err(AdapterError::TypeMismatch {
            expected: ty,
            got: value.ty(),
        });
    }
    Ok(match value {
        ScalarValue::Unit => return Err(AdapterError::UnitHasNoWord),
        ScalarValue::Bool(b) => u64::from(b),
        ScalarValue::I32(v) => v as i64 as u64,
        ScalarValue::U32(v) => u64::from(v),
        ScalarValue::I64(v) => v as u64,
        ScalarValue::U64(v) => v,
        // Bit patterns, not numeric casts: a float is not an integer.
        ScalarValue::F32(v) => u64::from(v.to_bits()),
        ScalarValue::F64(v) => v.to_bits(),
    })
}

/// Decode the word a native call returned into a typed value.
pub fn decode(ty: ScalarType, word: u64) -> Result<ScalarValue, AdapterError> {
    match ty {
        ScalarType::Unit => Err(AdapterError::UnitHasNoWord),
        ScalarType::Bool => match word {
            0 => Ok(ScalarValue::Bool(false)),
            1 => Ok(ScalarValue::Bool(true)),
            other => Err(AdapterError::InvalidWord {
                ty,
                word: other,
                detail: "a boolean word must be 0 or 1",
            }),
        },
        ScalarType::I32 => {
            // A C/Rust `i32` return zeroes or sign-extends the upper half
            // depending on the codegen; both are valid, anything else is not.
            let high = word >> 32;
            if high != 0 && high != 0xffff_ffff {
                return Err(AdapterError::InvalidWord {
                    ty,
                    word,
                    detail: "the upper 32 bits must be zero or a copy of the sign bit",
                });
            }
            Ok(ScalarValue::I32(word as u32 as i32))
        }
        ScalarType::U32 => {
            if word >> 32 != 0 {
                return Err(AdapterError::InvalidWord {
                    ty,
                    word,
                    detail: "a Nat32 word must have zero bits above bit 31",
                });
            }
            Ok(ScalarValue::U32(word as u32))
        }
        ScalarType::I64 => Ok(ScalarValue::I64(word as i64)),
        ScalarType::U64 => Ok(ScalarValue::U64(word)),
        ScalarType::F32 => {
            if word >> 32 != 0 {
                return Err(AdapterError::InvalidWord {
                    ty,
                    word,
                    detail: "a Float32 word must have zero bits above bit 31",
                });
            }
            Ok(ScalarValue::F32(f32::from_bits(word as u32)))
        }
        ScalarType::F64 => Ok(ScalarValue::F64(f64::from_bits(word))),
    }
}

/// Checked narrowing for a compiler-produced integer value that is being
/// passed to a narrower parameter: a value outside the type's range is
/// refused rather than truncated.
pub fn narrow_i64(value: i64, ty: ScalarType) -> Result<ScalarValue, AdapterError> {
    match ty {
        ScalarType::I32 => {
            i32::try_from(value)
                .map(ScalarValue::I32)
                .map_err(|_| AdapterError::InvalidWord {
                    ty,
                    word: value as u64,
                    detail: "the value does not fit in Int32",
                })
        }
        ScalarType::U32 => {
            u32::try_from(value)
                .map(ScalarValue::U32)
                .map_err(|_| AdapterError::InvalidWord {
                    ty,
                    word: value as u64,
                    detail: "the value does not fit in Nat32",
                })
        }
        ScalarType::I64 => Ok(ScalarValue::I64(value)),
        ScalarType::U64 => {
            u64::try_from(value)
                .map(ScalarValue::U64)
                .map_err(|_| AdapterError::InvalidWord {
                    ty,
                    word: value as u64,
                    detail: "the value does not fit in Nat64",
                })
        }
        other => Err(AdapterError::TypeMismatch {
            expected: other,
            got: ScalarType::I64,
        }),
    }
}

// ── C ABI ────────────────────────────────────────────────────────────────────
//
// Generated wrappers are written by umbrella and compiled by rustc, but the
// bridge's own callers (and that generator's fixtures) need the same encoding
// without linking Rust `Result`/`enum` types. These three functions are the
// whole surface: a version, an encode and a decode, over a small numeric type
// tag. Values travel as bit patterns, so a Float64 payload is its `to_bits`
// word — never a numeric cast.

const ABI_OK: i32 = 0;
const ABI_REFUSED: i32 = 1;

fn scalar_type_from_tag(tag: u32) -> Option<ScalarType> {
    Some(match tag {
        0 => ScalarType::Unit,
        1 => ScalarType::Bool,
        2 => ScalarType::I32,
        3 => ScalarType::U32,
        4 => ScalarType::I64,
        5 => ScalarType::U64,
        6 => ScalarType::F32,
        7 => ScalarType::F64,
        _ => return None,
    })
}

fn scalar_type_tag(ty: ScalarType) -> u32 {
    match ty {
        ScalarType::Unit => 0,
        ScalarType::Bool => 1,
        ScalarType::I32 => 2,
        ScalarType::U32 => 3,
        ScalarType::I64 => 4,
        ScalarType::U64 => 5,
        ScalarType::F32 => 6,
        ScalarType::F64 => 7,
    }
}

/// The scalar encoding version a wrapper was generated for.
#[no_mangle]
pub extern "C" fn au_scalar_abi_version() -> u32 {
    SCALAR_ABI_VERSION
}

/// Encode a logical value as its transport word.
///
/// `payload` is the value's bits: the integer for `Bool` (0/1), `I32`/`I64`
/// (`u32`/`u64` bit patterns), `F32`/`F64` (`to_bits`), and nothing for
/// `Unit`. Returns 0 on success, 1 on refusal (reason on the last-error
/// channel), and writes the word to `out` only on success.
#[no_mangle]
pub extern "C" fn au_scalar_encode(ty: u32, payload: u64, out: *mut u64) -> i32 {
    if out.is_null() {
        set_last_error("au_scalar_encode: null output pointer");
        return ABI_REFUSED;
    }
    let Some(kind) = scalar_type_from_tag(ty) else {
        set_last_error(&format!("au_scalar_encode: unknown scalar type tag {ty}"));
        return ABI_REFUSED;
    };
    let value = match kind {
        ScalarType::Unit => ScalarValue::Unit,
        ScalarType::Bool => match payload {
            0 => ScalarValue::Bool(false),
            1 => ScalarValue::Bool(true),
            other => {
                set_last_error(&format!(
                    "au_scalar_encode: a boolean payload must be 0 or 1, got {other}"
                ));
                return ABI_REFUSED;
            }
        },
        ScalarType::I32 => ScalarValue::I32(payload as u32 as i32),
        ScalarType::U32 => ScalarValue::U32(payload as u32),
        ScalarType::I64 => ScalarValue::I64(payload as i64),
        ScalarType::U64 => ScalarValue::U64(payload),
        ScalarType::F32 => ScalarValue::F32(f32::from_bits(payload as u32)),
        ScalarType::F64 => ScalarValue::F64(f64::from_bits(payload)),
    };
    match encode(kind, value) {
        Ok(word) => {
            unsafe { *out = word };
            ABI_OK
        }
        Err(e) => {
            set_last_error(&format!("au_scalar_encode: {e}"));
            ABI_REFUSED
        }
    }
}

/// Decode a transport word into a logical value: the tag is returned in `out`
/// (in the same numeric encoding as `ty`), the value's bits in `word`.
#[no_mangle]
pub extern "C" fn au_scalar_decode(ty: u32, word: u64, out: *mut u64) -> i32 {
    if out.is_null() {
        set_last_error("au_scalar_decode: null output pointer");
        return ABI_REFUSED;
    }
    let Some(kind) = scalar_type_from_tag(ty) else {
        set_last_error(&format!("au_scalar_decode: unknown scalar type tag {ty}"));
        return ABI_REFUSED;
    };
    match decode(kind, word) {
        Ok(value) => {
            let payload = match value {
                ScalarValue::Unit => 0,
                ScalarValue::Bool(b) => u64::from(b),
                ScalarValue::I32(v) => v as u32 as u64,
                ScalarValue::U32(v) => u64::from(v),
                ScalarValue::I64(v) => v as u64,
                ScalarValue::U64(v) => v,
                ScalarValue::F32(v) => u64::from(v.to_bits()),
                ScalarValue::F64(v) => v.to_bits(),
            };
            unsafe { *out = payload };
            ABI_OK
        }
        Err(e) => {
            set_last_error(&format!("au_scalar_decode: {e}"));
            ABI_REFUSED
        }
    }
}

/// The numeric tag for a scalar type, exposed for generators that must write
/// it into a descriptor without duplicating the numbering.
#[no_mangle]
pub extern "C" fn au_scalar_type_tag(ty: u32) -> i32 {
    match scalar_type_from_tag(ty) {
        Some(kind) => scalar_type_tag(kind) as i32,
        None => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── The native side: ordinary Rust functions, compiled by rustc. Calling
    //    them directly is the oracle; the adapter is compared against it.

    extern "C" fn scale_i32(v: i32) -> i32 {
        v.wrapping_mul(3)
    }

    extern "C" fn negate_bool(v: bool) -> bool {
        !v
    }

    extern "C" fn halve(v: f64) -> f64 {
        v / 2.0
    }

    extern "C" fn mixed(count: i32, scale: f64) -> f64 {
        (count as f64) * scale
    }

    /// The shape a generated adapter has: one word in, one word out. The
    /// closure decodes its argument with this module and encodes the native
    /// function's result with it, so the test exercises the real rules rather
    /// than a hand-written conversion.
    fn word_call(f: fn(u64) -> u64, word: u64) -> u64 {
        f(word)
    }

    #[test]
    fn the_encoding_is_versioned() {
        assert_eq!(
            SCALAR_ABI_VERSION, 1,
            "bump the version when a rule changes"
        );
        assert_eq!(ScalarType::Unit.words(), 0);
        assert_eq!(ScalarType::F64.words(), 1);
    }

    #[test]
    fn i32_values_roundtrip_and_match_the_direct_call() {
        for value in [0i32, 1, -1, i32::MIN, i32::MAX, -12345] {
            let word = encode(ScalarType::I32, ScalarValue::I32(value)).expect("encode");
            assert_eq!(
                decode(ScalarType::I32, word).expect("decode"),
                ScalarValue::I32(value)
            );
            // Negative values are sign-extended on the way out.
            assert_eq!(word, (value as i64) as u64);

            let out = word_call(
                |w: u64| {
                    let v = match decode(ScalarType::I32, w).expect("decode arg") {
                        ScalarValue::I32(v) => v,
                        other => panic!("not an i32: {other:?}"),
                    };
                    encode(ScalarType::I32, ScalarValue::I32(scale_i32(v))).expect("encode result")
                },
                word,
            );
            assert_eq!(
                decode(ScalarType::I32, out).expect("decode result"),
                ScalarValue::I32(scale_i32(value)),
                "the adapter and the direct call must agree for {value}"
            );
        }
    }

    #[test]
    fn narrowing_is_checked_not_truncating() {
        assert_eq!(
            narrow_i64(7, ScalarType::I32).expect("fits"),
            ScalarValue::I32(7)
        );
        assert_eq!(
            narrow_i64(i32::MIN as i64, ScalarType::I32).expect("fits"),
            ScalarValue::I32(i32::MIN)
        );
        for too_big in [i32::MAX as i64 + 1, i64::MAX, -1] {
            if too_big == -1 {
                assert!(narrow_i64(too_big, ScalarType::U32).is_err());
                continue;
            }
            assert!(
                narrow_i64(too_big, ScalarType::I32).is_err(),
                "{too_big} must not become an Int32"
            );
        }
        assert_eq!(
            narrow_i64(0, ScalarType::U32).expect("fits"),
            ScalarValue::U32(0)
        );
        assert_eq!(
            narrow_i64(u32::MAX as i64, ScalarType::U32).expect("fits"),
            ScalarValue::U32(u32::MAX)
        );
        assert!(narrow_i64(-1, ScalarType::U64).is_err());
    }

    #[test]
    fn a_bool_is_a_tag_and_other_tags_are_refused() {
        assert_eq!(
            encode(ScalarType::Bool, ScalarValue::Bool(false)).expect("encode"),
            0
        );
        assert_eq!(
            encode(ScalarType::Bool, ScalarValue::Bool(true)).expect("encode"),
            1
        );
        for tag in [2u64, 3, u64::MAX] {
            let err = decode(ScalarType::Bool, tag).unwrap_err();
            assert!(matches!(
                err,
                AdapterError::InvalidWord {
                    ty: ScalarType::Bool,
                    ..
                }
            ));
            assert!(err.to_string().contains("must be 0 or 1"), "{err}");
        }
        // The direct-call oracle: `!b` written by rustc, read through the tag.
        for value in [true, false] {
            let word = encode(ScalarType::Bool, ScalarValue::Bool(value)).expect("encode");
            let out = word_call(
                |w: u64| {
                    let v = match decode(ScalarType::Bool, w).expect("decode arg") {
                        ScalarValue::Bool(v) => v,
                        other => panic!("not a bool: {other:?}"),
                    };
                    encode(ScalarType::Bool, ScalarValue::Bool(negate_bool(v))).expect("encode")
                },
                word,
            );
            assert_eq!(
                decode(ScalarType::Bool, out).expect("decode result"),
                ScalarValue::Bool(!value)
            );
        }
    }

    #[test]
    fn floats_are_bit_preserving_not_numeric_casts() {
        for value in [0.0f64, -0.0, 1.5, -2.25, f64::INFINITY, f64::NEG_INFINITY] {
            let word = encode(ScalarType::F64, ScalarValue::F64(value)).expect("encode");
            assert_eq!(word, value.to_bits());
            assert_eq!(
                decode(ScalarType::F64, word).expect("decode"),
                ScalarValue::F64(value)
            );
            assert_eq!(decode(ScalarType::F64, word).unwrap().ty(), ScalarType::F64);
        }
        // -0.0 is a different bit pattern from 0.0 (and numerically equal).
        let neg_zero = encode(ScalarType::F64, ScalarValue::F64(-0.0)).unwrap();
        assert_eq!(neg_zero, 0x8000_0000_0000_0000);

        // A NaN payload survives untouched, and it is *not* `x as i64`.
        let nan_bits = 0x7ff8_1234_5678_9abc_u64;
        let nan = f64::from_bits(nan_bits);
        let word = encode(ScalarType::F64, ScalarValue::F64(nan)).unwrap();
        assert_eq!(word, nan_bits);
        assert_ne!(
            word, nan as i64 as u64,
            "floats must not go through an i64 cast"
        );
        let back = match decode(ScalarType::F64, word).unwrap() {
            ScalarValue::F64(v) => v,
            other => panic!("not an f64: {other:?}"),
        };
        assert!(back.is_nan());
        assert_eq!(back.to_bits(), nan_bits, "NaN payload must be preserved");
    }

    #[test]
    fn f32_uses_the_low_word_and_refuses_high_bits() {
        let word = encode(ScalarType::F32, ScalarValue::F32(-0.0)).unwrap();
        assert_eq!(word, 0x8000_0000);
        assert_eq!(
            decode(ScalarType::F32, word).unwrap(),
            ScalarValue::F32(-0.0)
        );
        let err = decode(ScalarType::F32, 0x0000_0001_0000_0000).unwrap_err();
        assert!(err.to_string().contains("zero bits above bit 31"), "{err}");
    }

    #[test]
    fn mixed_argument_classes_keep_their_order() {
        // count = 3, scale = 1.5 → 4.5; swapped operands would give 3.0 * 1.5? No:
        // the point is that each word is decoded by its own declared type.
        let args = [
            (ScalarType::I32, ScalarValue::I32(3)),
            (ScalarType::F64, ScalarValue::F64(1.5)),
        ];
        let words: Vec<u64> = args
            .iter()
            .map(|(ty, value)| encode(*ty, *value).expect("encode"))
            .collect();
        assert_eq!(words.len(), 2);
        let count = match decode(ScalarType::I32, words[0]).unwrap() {
            ScalarValue::I32(v) => v,
            other => panic!("{other:?}"),
        };
        let scale = match decode(ScalarType::F64, words[1]).unwrap() {
            ScalarValue::F64(v) => v,
            other => panic!("{other:?}"),
        };
        assert_eq!(count, 3);
        assert_eq!(scale, 1.5);
        assert_eq!(mixed(count, scale), 4.5);
        // Non-commutative on purpose: the same two values in the other roles
        // must not produce the same answer.
        assert_eq!(mixed(1, 3.0), 3.0);
        assert_ne!(mixed(3, 1.5), mixed(1, 3.0));
    }

    #[test]
    fn the_direct_float_call_agrees_with_the_adapter() {
        for value in [1.0f64, -0.0, 7.5, f64::MAX, f64::MIN_POSITIVE] {
            let word = encode(ScalarType::F64, ScalarValue::F64(value)).unwrap();
            let out = word_call(
                |w: u64| {
                    let v = match decode(ScalarType::F64, w).expect("arg") {
                        ScalarValue::F64(v) => v,
                        other => panic!("{other:?}"),
                    };
                    encode(ScalarType::F64, ScalarValue::F64(halve(v))).expect("result")
                },
                word,
            );
            assert_eq!(
                decode(ScalarType::F64, out).unwrap(),
                ScalarValue::F64(halve(value)),
                "direct call and adapter must agree for {value}"
            );
        }
    }

    #[test]
    fn words_that_are_not_their_type_are_refused() {
        // Int32 with garbage in the upper half.
        let err = decode(ScalarType::I32, 0x0000_0001_0000_0000).unwrap_err();
        assert!(err.to_string().contains("copy of the sign bit"), "{err}");
        // Nat32 with a bit above 31.
        let err = decode(ScalarType::U32, 0x1_0000_0000).unwrap_err();
        assert!(err.to_string().contains("above bit 31"), "{err}");
        // Unit has no word in either direction.
        assert_eq!(
            encode(ScalarType::Unit, ScalarValue::Unit).unwrap_err(),
            AdapterError::UnitHasNoWord
        );
        assert_eq!(
            decode(ScalarType::Unit, 0).unwrap_err(),
            AdapterError::UnitHasNoWord
        );
        // A value of one type cannot masquerade as another.
        let err = encode(ScalarType::I32, ScalarValue::Bool(true)).unwrap_err();
        assert!(matches!(err, AdapterError::TypeMismatch { .. }), "{err}");
    }
}
