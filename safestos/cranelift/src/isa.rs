//! `ISA_REGISTRY` — the instruction subset the CPS machine defines.
//!
//! W8 of `PLAN_sota_kernel_verification.md` claims a model of "the bytes we
//! emit", not of "the ISA". The difference is measurable: [`ISA_REGISTRY`] lists
//! every opcode `cps.rs::emit_expr` (and the statement loop beside it) accepts,
//! [`note`] records the opcodes a compilation actually consumed, and
//! `scripts/isa-census` fails when the two disagree or when an opcode was
//! consumed that the table does not declare.
//!
//! Same mechanism as `SYMBOL_REGISTRY`: the table IS the registry, a test
//! iterates it, and the census is what stops the model from drifting into "the
//! ISA" while the claim stays "the instructions we emit". `mm0/isa.mm0`
//! declares the same table (one `-- op 0x.. name` line per entry) and the census
//! checks that too, so the artifact cannot silently cover less than the decoder.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// One instruction of the emitted subset.
#[derive(Debug, Clone, Copy)]
pub struct IsaInstruction {
    /// The byte that introduces the instruction in the CPS binary.
    pub opcode: u8,
    /// The name the model and the census use.
    pub name: &'static str,
    /// Operand shape, as the decoder reads it (documentation, not parsed).
    pub operands: &'static str,
    /// Coarse class, for reading the table: control, data, arith, cmp, mem.
    pub class: &'static str,
}

/// Every opcode the bridge's CPS decoder accepts (`src/cps.rs`). This is the
/// subset `mm0/isa.mm0` must cover; an opcode added to the decoder without an
/// entry here is a census failure, not a gap.
pub const ISA_REGISTRY: &[IsaInstruction] = &[
    IsaInstruction {
        opcode: 0x01,
        name: "int_lit",
        operands: "i64",
        class: "data",
    },
    IsaInstruction {
        opcode: 0x02,
        name: "var",
        operands: "string",
        class: "data",
    },
    IsaInstruction {
        opcode: 0x03,
        name: "let",
        operands: "string, expr, body",
        class: "data",
    },
    IsaInstruction {
        opcode: 0x04,
        name: "app",
        operands: "string, u32, expr*",
        class: "control",
    },
    IsaInstruction {
        opcode: 0x05,
        name: "add",
        operands: "expr, expr",
        class: "arith",
    },
    IsaInstruction {
        opcode: 0x06,
        name: "sub",
        operands: "expr, expr",
        class: "arith",
    },
    IsaInstruction {
        opcode: 0x07,
        name: "return",
        operands: "expr",
        class: "control",
    },
    IsaInstruction {
        opcode: 0x08,
        name: "if",
        operands: "expr, expr, expr",
        class: "control",
    },
    IsaInstruction {
        opcode: 0x09,
        name: "while",
        operands: "expr, expr",
        class: "control",
    },
    IsaInstruction {
        opcode: 0x0A,
        name: "match",
        operands: "expr, u32, (u8, expr)*, expr",
        class: "control",
    },
    IsaInstruction {
        opcode: 0x10,
        name: "cmp_lt",
        operands: "expr, expr",
        class: "cmp",
    },
    IsaInstruction {
        opcode: 0x11,
        name: "cmp_gt",
        operands: "expr, expr",
        class: "cmp",
    },
    IsaInstruction {
        opcode: 0x12,
        name: "cmp_lte",
        operands: "expr, expr",
        class: "cmp",
    },
    IsaInstruction {
        opcode: 0x13,
        name: "cmp_gte",
        operands: "expr, expr",
        class: "cmp",
    },
    IsaInstruction {
        opcode: 0x14,
        name: "cmp_eq",
        operands: "expr, expr",
        class: "cmp",
    },
    IsaInstruction {
        opcode: 0x15,
        name: "cmp_neq",
        operands: "expr, expr",
        class: "cmp",
    },
    IsaInstruction {
        opcode: 0x16,
        name: "and",
        operands: "expr, expr",
        class: "arith",
    },
    IsaInstruction {
        opcode: 0x17,
        name: "or",
        operands: "expr, expr",
        class: "arith",
    },
    IsaInstruction {
        opcode: 0x18,
        name: "mul",
        operands: "expr, expr",
        class: "arith",
    },
    IsaInstruction {
        opcode: 0x19,
        name: "not",
        operands: "expr",
        class: "arith",
    },
    IsaInstruction {
        opcode: 0x20,
        name: "deref",
        operands: "expr",
        class: "mem",
    },
    IsaInstruction {
        opcode: 0x21,
        name: "string_lit",
        operands: "u32, bytes",
        class: "mem",
    },
    IsaInstruction {
        opcode: 0x30,
        name: "store",
        operands: "expr, expr",
        class: "mem",
    },
];

/// Names of every declared opcode, sorted by opcode (for the census' JSON).
pub fn registry_names() -> Vec<(u8, &'static str)> {
    ISA_REGISTRY.iter().map(|i| (i.opcode, i.name)).collect()
}

/// Parse one `-- op 0xNN name` line of an MM0 model's instruction table.
///
/// The census reads the artifact's own table so the model cannot cover less
/// than the decoder accepts while still compiling.
pub fn parse_mm0_table(text: &str) -> Result<Vec<(u8, String)>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("-- op ") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let hex = parts
            .next()
            .ok_or_else(|| format!("malformed op line: {line}"))?;
        let name = parts
            .next()
            .ok_or_else(|| format!("op line without a name: {line}"))?;
        let opcode = u8::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| format!("bad opcode {hex}: {e}"))?;
        out.push((opcode, name.to_string()));
    }
    Ok(out)
}

/// Opcodes consumed since the last [`clear`], as a 64-bit set (opcodes < 64).
static SEEN: Mutex<u64> = Mutex::new(0);
/// How many opcode reads happened at all, so "the census ran on a module" is
/// distinguishable from "the census ran on nothing".
static READS: AtomicU64 = AtomicU64::new(0);

/// Record that `opcode` was consumed. Called by `cps.rs` at each opcode read.
pub fn note(opcode: u8) {
    if opcode < 64 {
        if let Ok(mut set) = SEEN.lock() {
            *set |= 1u64 << opcode;
        }
    }
    READS.fetch_add(1, Ordering::Relaxed);
}

/// Clear the recording (before compiling the module under observation).
pub fn clear() {
    if let Ok(mut set) = SEEN.lock() {
        *set = 0;
    }
    READS.store(0, Ordering::Relaxed);
}

/// The opcodes recorded, ascending, with how many opcode reads happened.
pub fn snapshot() -> (Vec<u8>, u64) {
    let set = SEEN.lock().map(|s| *s).unwrap_or(0);
    let hits: Vec<u8> = (0u8..64).filter(|b| set & (1u64 << b) != 0).collect();
    (hits, READS.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_is_sorted_and_unique() {
        let mut last = 0u8;
        for ins in ISA_REGISTRY {
            assert!(
                ins.opcode > last,
                "out of order or duplicate: 0x{:02x}",
                ins.opcode
            );
            last = ins.opcode;
        }
    }

    #[test]
    fn recording_is_a_set_not_a_counter() {
        clear();
        note(0x05);
        note(0x05);
        note(0x18);
        let (hits, reads) = snapshot();
        assert_eq!(hits, vec![0x05, 0x18]);
        assert_eq!(reads, 3);
        clear();
        assert_eq!(snapshot().0, Vec::<u8>::new());
    }

    #[test]
    fn an_op_line_is_parsed_and_a_bad_one_is_refused() {
        let table = "-- op 0x05 add\n  -- op 0x18 mul\n-- op zz bad\n";
        assert!(parse_mm0_table(table).is_err());
        let ok = parse_mm0_table("-- op 0x05 add\n-- op 0x18 mul\n");
        assert_eq!(
            ok.unwrap(),
            vec![(0x05, "add".to_string()), (0x18, "mul".to_string())]
        );
    }
}
