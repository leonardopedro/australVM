//! `cps2cell` — the bridge half of the `.avm` pipeline (spec §3.4,
//! Sprint 3 Task 3.2): CPS binary IR in, `AVCE` cell container out.
//! `avm-exec`'s cell builder parses the container and serializes the
//! `.avm` layout (magic `AVMC`, derivation hash, capability table, page-
//! aligned text, relocation table, Ed25519 signature).
//!
//! Deliberately a *separate process*: `safestos/pkg` stays a pure-crate
//! workspace (it must resolve offline without the bridge's unfer/cedar
//! path-dep graph — spec §6.2's workspace separation), so `avm-exec`
//! spawns this helper instead of linking the bridge.
//!
//! Usage: `cps2cell <input.cps> <output.avce>`
//! Exit: 0 on success; 1 with a diagnostic on stderr otherwise.

use austral_cranelift_bridge::capture::CaptureModule;
use austral_cranelift_bridge::cps;
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("cps2cell: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: cps2cell <input.cps> <output.avce>";
    let input = PathBuf::from(args.next().ok_or(usage)?);
    let output = PathBuf::from(args.next().ok_or(usage)?);

    let ir = std::fs::read(&input).map_err(|e| format!("read {}: {e}", input.display()))?;
    let mut cap = CaptureModule::new()?;
    cps::compile_cps_to_clif(&mut cap, &ir).map_err(|e| format!("cps compile: {e}"))?;

    let defined = cap.defined();
    if defined.is_empty() {
        return Err("compilation produced no defined functions".into());
    }
    let avce = cap.encode_avce();
    std::fs::write(&output, &avce).map_err(|e| format!("write {}: {e}", output.display()))?;
    let relocs: usize = defined.iter().map(|(_, b)| b.relocs.len()).sum();
    eprintln!(
        "cps2cell: {} function(s), {} import(s), {relocs} relocation(s), {} bytes of text total, arch {}",
        defined.len(),
        cap.imports().len(),
        defined.iter().map(|(_, b)| b.bytes.len()).sum::<usize>(),
        cap.arch_name()
    );
    Ok(())
}
