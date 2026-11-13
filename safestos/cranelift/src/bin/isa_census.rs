//! `isa-census` — which instructions does the emitted CPS IR actually use, and
//! does `mm0/isa.mm0` declare exactly those?
//!
//! Three-way check, because two of the three can agree while both are wrong:
//!
//! 1. **decoder** — `src/cps.rs` accepts these opcodes (`ISA_REGISTRY`);
//! 2. **artifact** — `mm0/isa.mm0` declares these opcodes (`-- op 0xNN name`);
//! 3. **measured** — the opcodes a real compile of a given `.cps` consumed.
//!
//! Compilation happens through `cps::compile_cps_to_clif` with a
//! `CaptureModule`, i.e. the same parser and lowering the JIT uses, so the
//! measurement is taken from the code path that runs, not from a second decoder
//! that could drift from it. Nothing is executed.
//!
//! Usage: `isa-census <module.cps> [<isa.mm0>]`
//! Exit: 0 covered and consistent, 1 uncovered/inconsistent, 2 could not run.

use austral_cranelift_bridge::capture::CaptureModule;
use austral_cranelift_bridge::cps;
use austral_cranelift_bridge::isa::{self, ISA_REGISTRY};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn main() {
    match run() {
        Ok(report) => {
            println!("{report}");
        }
        Err(e) => {
            eprintln!("isa-census: {e}");
            std::process::exit(2);
        }
    }
}

fn run() -> Result<String, String> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: isa-census <module.cps> [<isa.mm0>]";
    let ir_path = PathBuf::from(args.next().ok_or(usage)?);
    let mm0_path = args.next().map(PathBuf::from);
    let ir = std::fs::read(&ir_path).map_err(|e| format!("read {}: {e}", ir_path.display()))?;

    isa::clear();
    let mut cap = CaptureModule::new()?;
    cps::compile_cps_to_clif(&mut cap, &ir).map_err(|e| format!("cps compile: {e}"))?;
    let (hits, reads) = isa::snapshot();
    if reads == 0 {
        return Err("the compile consumed no opcodes — nothing was measured".into());
    }
    let _ = cap.defined();

    let declared: BTreeSet<u8> = ISA_REGISTRY.iter().map(|i| i.opcode).collect();
    let measured: BTreeSet<u8> = hits.iter().copied().collect();
    let uncovered: Vec<u8> = measured.difference(&declared).copied().collect();

    let mut failures = Vec::new();
    if !uncovered.is_empty() {
        failures.push(format!(
            "uncovered opcode(s) in the decoder: {}",
            hex_list(&uncovered)
        ));
    }

    let mut artifact = String::from("(not checked: no mm0/isa.mm0 given)");
    if let Some(path) = &mm0_path {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let table = isa::parse_mm0_table(&text)?;
        let model: BTreeSet<u8> = table.iter().map(|(op, _)| *op).collect();
        let only_decoder: Vec<u8> = declared.difference(&model).copied().collect();
        let only_model: Vec<u8> = model.difference(&declared).copied().collect();
        if !only_decoder.is_empty() {
            failures.push(format!(
                "declared in the decoder but missing from {}: {}",
                path.display(),
                hex_list(&only_decoder)
            ));
        }
        if !only_model.is_empty() {
            failures.push(format!(
                "modelled in {} but not accepted by the decoder: {}",
                path.display(),
                hex_list(&only_model)
            ));
        }
        for (op, name) in &table {
            match ISA_REGISTRY.iter().find(|i| i.opcode == *op) {
                Some(ins) if ins.name == name => {}
                Some(ins) => failures.push(format!(
                    "0x{op:02x}: the model calls it `{name}`, the decoder `{}`",
                    ins.name
                )),
                None => {}
            }
        }
        artifact = format!("{} opcode(s) declared in {}", model.len(), path.display());
    }

    let used: Vec<String> = hits
        .iter()
        .map(|op| {
            let name = ISA_REGISTRY
                .iter()
                .find(|i| i.opcode == *op)
                .map(|i| i.name)
                .unwrap_or("?");
            format!("0x{op:02x} {name}")
        })
        .collect();

    let mut out = String::new();
    out.push_str(&format!(
        "isa-census: {} — {reads} opcode read(s), {} distinct, {} declared, {}\n",
        ir_path.display(),
        hits.len(),
        declared.len(),
        artifact
    ));
    out.push_str(&format!("  used: {}\n", used.join(", ")));
    let unused: Vec<String> = declared
        .difference(&measured)
        .map(|op| {
            let name = ISA_REGISTRY
                .iter()
                .find(|i| i.opcode == *op)
                .map(|i| i.name)
                .unwrap_or("?");
            format!("0x{op:02x} {name}")
        })
        .collect();
    if !unused.is_empty() {
        out.push_str(&format!(
            "  declared but not used by this module (covered by the model, {}): {}\n",
            unused.len(),
            unused.join(", ")
        ));
    }
    if failures.is_empty() {
        let scope = if mm0_path.is_some() {
            "in the decoder and in the artifact"
        } else {
            "in the decoder (artifact not checked)"
        };
        out.push_str(&format!(
            "  result: covered — every opcode used is declared {scope}\n"
        ));
        Ok(out)
    } else {
        for f in &failures {
            eprintln!("isa-census: FAIL {f}");
        }
        std::process::exit(1);
    }
}

fn hex_list(ops: &[u8]) -> String {
    ops.iter()
        .map(|op| format!("0x{op:02x}"))
        .collect::<Vec<_>>()
        .join(", ")
}
