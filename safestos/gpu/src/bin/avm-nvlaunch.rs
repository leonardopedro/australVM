//! `avm-nvlaunch` — G21a: run the published NVIDIA artifact on a real device.
//!
//! ```text
//! avm-nvlaunch probe
//! avm-nvlaunch launch BUNDLE --ptx FILE --inputs FILE [--extent N] [--arity N]
//! ```
//!
//! `probe` reports the driver and device the launch would use. `launch` reads
//! the bundle's recorded ABI and extent, checks the PTX against them and
//! against the device ([`avm_gpu::nvlaunch::plan`]), uploads the input buffers,
//! launches, synchronises and downloads — printing the result as words.
//!
//! Exit codes follow the crate's convention: 0 success, 1 refused or failed,
//! 2 usage, 77 the optional probe is unavailable (no driver or no device).
//!
//! This binary is not built by default: it needs `--features nvidia`, and even
//! then it adds nothing to the build but the `dlopen` calls. It never links
//! CUDA and never needs a toolkit.

use avm_gpu::nvlaunch::{self, driver::Driver, Plan, Refusal};
use std::path::Path;

fn usage() -> i32 {
    eprintln!(
        "usage:\n  avm-nvlaunch probe [--device N]\n  avm-nvlaunch launch BUNDLE --ptx FILE --inputs FILE [--extent N] [--arity N] [--device N]"
    );
    2
}

fn json_error(message: &str) -> i32 {
    eprintln!("{}", serde_json::json!({ "status": "error", "message": message }));
    1
}

fn unavailable(message: &str) -> i32 {
    eprintln!(
        "{}",
        serde_json::json!({ "status": "unavailable", "message": message })
    );
    77
}

fn open(device: i32) -> Result<Driver, i32> {
    match Driver::open(device) {
        Ok(d) => Ok(d),
        Err(e) => Err(unavailable(&e)),
    }
}

fn probe(args: &[String], device: i32) -> i32 {
    let _ = args;
    let driver = match open(device) {
        Ok(d) => d,
        Err(code) => return code,
    };
    println!(
        "{}",
        serde_json::json!({
            "status": "ok",
            "backend": "cuda-driver",
            "device": driver.device.name,
            "capability": format!("{}.{}", driver.device.capability.0, driver.device.capability.1),
            "driver_version": driver.device.driver_version,
            "chip": format!("sm_{}{}", driver.device.capability.0, driver.device.capability.1),
        })
    );
    0
}

/// The published `bundle.json` (`austral.gpu.mlir.v1`). Only the fields the
/// launch needs are read; the bundle's own checker owns the rest.
struct Recorded {
    arity: usize,
    extent: usize,
    params: usize,
    workgroup: usize,
}

fn read_bundle(dir: &str) -> Result<Recorded, String> {
    let path = Path::new(dir).join("bundle.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", path.display()))?;
    let schema = v.get("schema").and_then(|s| s.as_str()).unwrap_or("");
    if schema != "austral.gpu.mlir.v1" {
        return Err(format!("{} is schema {schema:?}, not austral.gpu.mlir.v1", path.display()));
    }
    let arity = v.get("input_arity").and_then(|x| x.as_u64()).ok_or("bundle.json has no input_arity")? as usize;
    let extent = v.get("extent").and_then(|x| x.as_u64()).ok_or("bundle.json has no extent")? as usize;
    let params = v
        .get("abi")
        .and_then(|a| a.get("params"))
        .and_then(|x| x.as_u64())
        .ok_or("bundle.json has no abi.params")? as usize;
    let workgroup = v
        .get("workgroup")
        .and_then(|x| x.as_u64())
        .ok_or("bundle.json has no workgroup (the block size the module declares)")?
        as usize;
    Ok(Recorded { arity, extent, params, workgroup })
}

fn read_words(path: &str) -> Result<Vec<Vec<u32>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{path} is not JSON: {e}"))?;
    let rows = value
        .get("inputs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("{path} must hold {{\"inputs\": [[..]]}}"))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let words = row
            .as_array()
            .ok_or_else(|| format!("{path}: every input must be an array of words"))?;
        let mut r = Vec::with_capacity(words.len());
        for w in words {
            r.push(
                w.as_u64()
                    .ok_or_else(|| "input words must be non-negative integers (u32 words)".to_string())?
                    as u32,
            );
        }
        out.push(r);
    }
    Ok(out)
}

fn launch(args: &[String], device: i32) -> i32 {
    let mut bundle = None;
    let mut ptx = None;
    let mut inputs = None;
    let mut extent = None;
    let mut arity = None;
    let mut block = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ptx" if i + 1 < args.len() => {
                ptx = Some(args[i + 1].clone());
                i += 2;
            }
            "--inputs" if i + 1 < args.len() => {
                inputs = Some(args[i + 1].clone());
                i += 2;
            }
            "--extent" if i + 1 < args.len() => {
                match args[i + 1].parse::<usize>() {
                    Ok(n) => extent = Some(n),
                    Err(_) => return json_error("--extent must be a non-negative integer"),
                }
                i += 2;
            }
            "--arity" if i + 1 < args.len() => {
                match args[i + 1].parse::<usize>() {
                    Ok(n) => arity = Some(n),
                    Err(_) => return json_error("--arity must be a non-negative integer"),
                }
                i += 2;
            }
            "--block" if i + 1 < args.len() => {
                match args[i + 1].parse::<usize>() {
                    Ok(n) => block = Some(n),
                    Err(_) => return json_error("--block must be a non-negative integer"),
                }
                i += 2;
            }
            other if !other.starts_with("--") && bundle.is_none() => {
                bundle = Some(other.to_string());
                i += 1;
            }
            other => return json_error(&format!("unexpected argument {other}")),
        }
    }
    let (bundle, ptx, inputs) = match (bundle, ptx, inputs) {
        (Some(b), Some(p), Some(i)) => (b, p, i),
        _ => return usage(),
    };

    let recorded = match read_bundle(&bundle) {
        Ok(r) => r,
        Err(e) => return json_error(&e),
    };
    let recorded_arity = recorded.arity;
    let recorded_extent = recorded.extent;
    let ptx_text = match std::fs::read_to_string(&ptx) {
        Ok(t) => t,
        Err(e) => return json_error(&format!("cannot read {ptx}: {e}")),
    };
    let facts = match nvlaunch::ptx_facts(&ptx_text) {
        Ok(f) => f,
        Err(e) => return json_error(&e),
    };

    let driver = match open(device) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let requested_extent = extent.unwrap_or(recorded_extent);
    let requested_arity = arity.unwrap_or(recorded_arity);
    let plan: Plan = match nvlaunch::plan(
        &facts,
        recorded.params,
        recorded_arity,
        recorded_extent,
        requested_extent,
        requested_arity,
        recorded.workgroup,
        block,
        Some(driver.device.capability),
    ) {
        Ok(p) => p,
        Err(e @ Refusal::TargetNewerThanDevice { .. }) => {
            // Measured, not assumed: the driver's own verdict on a module whose
            // target is newer than the device. It must not load.
            let verdict = match driver.load_only(&ptx_text) {
                Ok(()) => "the driver loaded it anyway".to_string(),
                Err(e) => e,
            };
            eprintln!(
                "{}",
                serde_json::json!({
                    "status": "refused",
                    "refusal": "target-newer-than-device",
                    "message": e.to_string(),
                    "driver_verdict": verdict,
                })
            );
            return 1;
        }
        Err(e) => {
            eprintln!(
                "{}",
                serde_json::json!({
                    "status": "refused",
                    "refusal": refusal_code(&e),
                    "message": e.to_string(),
                })
            );
            return 1;
        }
    };

    let buffers = match read_words(&inputs) {
        Ok(b) => b,
        Err(e) => return json_error(&e),
    };
    if buffers.len() != plan.descriptors {
        return json_error(&format!(
            "the plan expects {} buffer(s), the input file holds {}",
            plan.descriptors,
            buffers.len()
        ));
    }
    for (index, words) in buffers.iter().enumerate() {
        if words.len() != plan.extent {
            return json_error(&format!(
                "buffer {index} holds {} word(s); the launch extent is {}",
                words.len(),
                plan.extent
            ));
        }
    }
    let launched = match driver.launch(&ptx_text, &plan, &buffers) {
        Ok(out) => out,
        Err(e) => return json_error(&e),
    };
    println!(
        "{}",
        serde_json::json!({
            "status": "ok",
            "backend": "cuda-driver",
            "device": driver.device.name,
            "capability": format!("{}.{}", driver.device.capability.0, driver.device.capability.1),
            "driver_version": driver.device.driver_version,
            "chip": facts.target_chip,
            "entry": facts.entry,
            "ptx_params": facts.params,
            "extent": plan.extent,
            "block": plan.block,
            "grid": plan.grid,
            "workgroup": recorded.workgroup,
            "output": launched,
        })
    );
    0
}

fn refusal_code(r: &Refusal) -> &'static str {
    match r {
        Refusal::EntryMissing { .. } => "entry-missing",
        Refusal::AddressSize { .. } => "address-size",
        Refusal::Arity { .. } => "wrong-arity",
        Refusal::ParamCount { .. } => "wrong-param-count",
        Refusal::ExtentMismatch { .. } => "extent-mismatch",
        Refusal::ExtentZero => "zero-extent",
        Refusal::BlockZero => "block-zero",
        Refusal::BlockTooLarge { .. } => "block-too-large",
        Refusal::GridTooLarge { .. } => "grid-too-large",
        Refusal::NeedsBlockIndex { .. } => "needs-block-index",
        Refusal::NoAddressConversion => "no-address-conversion",
        Refusal::TargetNewerThanDevice { .. } => "target-newer-than-device",
        Refusal::TargetUnreadable { .. } => "target-unreadable",
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let device = argv
        .iter()
        .position(|a| a == "--device")
        .and_then(|i| argv.get(i + 1))
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let rest: Vec<String> = {
        let mut out = Vec::new();
        let mut skip = false;
        for a in &argv {
            if skip {
                skip = false;
                continue;
            }
            if a == "--device" {
                skip = true;
                continue;
            }
            out.push(a.clone());
        }
        out
    };
    let code = match rest.first().map(String::as_str) {
        Some("probe") => probe(&rest[1..], device),
        Some("launch") => launch(&rest[1..], device),
        _ => usage(),
    };
    std::process::exit(code);
}
