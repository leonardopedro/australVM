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
//! Usage: `isa-census [--isa <isa.mm0>] [--allow-all] <module.cps>...`
//!
//! `--allow-all` installs the AllowAll authorizer for the measurement. A module
//! that calls the kernel (`uk_*`) is gated by the Cedar engine's default deny,
//! which would make the census refuse to *parse* bytes it should be counting;
//! nothing is executed here (the module is a `CaptureModule`), so the grant
//! question is not this tool's to answer — `modhost` is where it is asked.
//!
//! Several modules are censused separately and reported together, because "the
//! emitted subset" is a claim about the emitter, not about one kernel: flux1d
//! reads 8 of the 23 opcodes, so a census over it alone never measures `add`,
//! `while`, `store` or `deref` at all. The measured set is the union, and each
//! module's line says what it contributed.
//!
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
    let usage = "usage: isa-census [--isa <isa.mm0>] [--allow-all] <module.cps>...";
    let mut mm0_path: Option<PathBuf> = None;
    let mut modules: Vec<PathBuf> = Vec::new();
    let mut allow_all = false;
    let mut args = std::env::args_os().skip(1).peekable();
    while let Some(arg) = args.next() {
        let s = arg.to_string_lossy().into_owned();
        if s == "--isa" {
            mm0_path = Some(PathBuf::from(args.next().ok_or(usage)?));
        } else if s == "--allow-all" {
            allow_all = true;
        } else if s.starts_with("--") {
            return Err(format!("unknown flag {s}; {usage}"));
        } else {
            modules.push(PathBuf::from(arg));
        }
    }
    // One module and a trailing `.mm0` was the old two-argument form.
    if mm0_path.is_none() && modules.len() == 2 {
        let last = modules[1].to_string_lossy().into_owned();
        if last.ends_with(".mm0") {
            mm0_path = modules.pop();
        }
    }
    if modules.is_empty() {
        return Err(usage.into());
    }
    if allow_all {
        austral_cranelift_bridge::auth::set_allow_all();
    }

    let declared: BTreeSet<u8> = ISA_REGISTRY.iter().map(|i| i.opcode).collect();
    let mut measured: BTreeSet<u8> = BTreeSet::new();
    let mut per_module: Vec<String> = Vec::new();
    let mut reads_total: u64 = 0;

    for ir_path in &modules {
        let ir =
            std::fs::read(ir_path).map_err(|e| format!("read {}: {e}", ir_path.display()))?;
        isa::clear();
        let mut cap = CaptureModule::new()?;
        cps::compile_cps_to_clif(&mut cap, &ir)
            .map_err(|e| format!("cps compile {}: {e}", ir_path.display()))?;
        let (hits, reads) = isa::snapshot();
        if reads == 0 {
            return Err(format!(
                "{}: the compile consumed no opcodes — nothing was measured",
                ir_path.display()
            ));
        }
        let _ = cap.defined();
        reads_total += reads;
        measured.extend(hits.iter().copied());
        let name = ir_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ir_path.display().to_string());
        per_module.push(format!(
            "{name}: {} distinct, {reads} read(s)",
            hits.len()
        ));
    }

    let hits: BTreeSet<u8> = measured.clone();
    let reads = reads_total;
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
    let scope = match (&mm0_path, modules.len()) {
        (Some(_), 1) => "module".to_string(),
        (None, 1) => "module".to_string(),
        (_, n) => format!("{n} modules"),
    };
    out.push_str(&format!(
        "isa-census: {scope} — {reads} opcode read(s), {} distinct, {} declared, {}\n",
        hits.len(),
        declared.len(),
        artifact
    ));
    if modules.len() > 1 {
        out.push_str(&format!("  per module: {}\n", per_module.join("; ")));
    }
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
            "  declared but not used here (covered by the model, {}): {}\n",
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
