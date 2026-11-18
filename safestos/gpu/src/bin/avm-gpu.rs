//! `avm-gpu` — CPU-only bundle checker and KernelIR reference evaluator.
//!
//! Exit codes (KERNEL_V1 §5): 0 success, 1 refused/failed, 2 usage, 77 an
//! optional probe whose adapter is unavailable. `--require-hardware` turns
//! unavailability into exit 1; device execution is G09/G10 and is not built
//! into the default feature set.

use avm_gpu::{artifact, reference, schema, validate};
use std::path::Path;

fn usage() -> i32 {
    eprintln!(
        "usage:\n  avm-gpu validate BUNDLE\n  avm-gpu reference BUNDLE --inputs FILE\n  avm-gpu probe [--require-hardware]\n  avm-gpu run BUNDLE --inputs FILE [--require-hardware]"
    );
    2
}

fn fail(msg: &str) -> i32 {
    eprintln!(
        "{}",
        serde_json::json!({ "status": "error", "message": msg })
    );
    1
}

fn ok_json(v: serde_json::Value) {
    println!("{v}");
}

fn load(dir: &str) -> Result<avm_gpu::LoadedBundle, String> {
    artifact::load_bundle(Path::new(dir))
}

fn validate_command(dir: &str) -> i32 {
    match load(dir) {
        Err(e) => fail(&e),
        Ok(loaded) => match validate::validate_bundle(&loaded) {
            Err(e) => fail(&e),
            Ok(()) => {
                ok_json(serde_json::json!({
                    "schema": schema::SCHEMA,
                    "status": "ok",
                    "entry": loaded.bundle.entry,
                    "input_arity": loaded.bundle.input_arity,
                    "ir_sha256": loaded.bundle.ir_sha256,
                    "wgsl_sha256": loaded.bundle.wgsl_sha256,
                }));
                0
            }
        },
    }
}

fn read_inputs(path: &str) -> Result<Vec<Vec<u32>>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("cannot read inputs {path}: {e}"))?;
    if metadata.len() > artifact::MAX_INPUT_JSON_BYTES {
        return Err(format!(
            "inputs file is {} bytes, over the {}-byte limit",
            metadata.len(),
            artifact::MAX_INPUT_JSON_BYTES
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read inputs {path}: {e}"))?;
    artifact::strict_json_check(&bytes)
        .map_err(|e| format!("inputs JSON is not strict: {e}"))?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Inputs {
        inputs: Vec<Vec<serde_json::Number>>,
    }
    let parsed: Inputs = serde_json::from_slice(&bytes)
        .map_err(|e| format!("inputs JSON does not match {{\"inputs\": [[..]]}}: {e}"))?;
    let mut out = Vec::new();
    for (i, array) in parsed.inputs.iter().enumerate() {
        let mut values = Vec::with_capacity(array.len());
        for (j, n) in array.iter().enumerate() {
            let v = n
                .as_u64()
                .ok_or_else(|| format!("inputs[{i}][{j}] is not an unsigned integer"))?;
            if v > u32::MAX as u64 {
                return Err(format!(
                    "inputs[{i}][{j}] = {v} is outside [0,4294967295]"
                ));
            }
            values.push(v as u32);
        }
        out.push(values);
    }
    Ok(out)
}

fn reference_command(dir: &str, inputs_path: &str) -> i32 {
    let loaded = match load(dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_bundle(&loaded) {
        return fail(&e);
    }
    let inputs = match read_inputs(inputs_path) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let mut evaluator = match reference::Evaluator::new(
        &loaded.bundle.kernel_ir,
        reference::Limits::default(),
    ) {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    match evaluator.run(&inputs) {
        Err(e) => fail(&e),
        Ok(output) => {
            ok_json(serde_json::json!({
                "status": "ok",
                "backend": "reference",
                "numeric_profile": schema::NUMERIC_PROFILE,
                "entry": loaded.bundle.entry,
                "input_arity": loaded.bundle.input_arity,
                "ir_sha256": loaded.bundle.ir_sha256,
                "wgsl_sha256": loaded.bundle.wgsl_sha256,
                "dispatch_count": if inputs.first().map(|v| v.is_empty()).unwrap_or(true) { 0 } else { 1 },
                "output": output,
            }));
            0
        }
    }
}

#[cfg(feature = "device")]
fn probe_command(require_hardware: bool) -> i32 {
    use avm_gpu::device::{self, ProbeError};
    match device::probe() {
        Ok(evidence) => {
            ok_json(serde_json::json!({
                "status": "ok",
                "device_kind": "physical",
                "device": device::to_json(&evidence),
            }));
            0
        }
        Err(ProbeError::Unavailable(message)) => {
            if require_hardware {
                fail(&format!("no physical adapter is available: {message}"))
            } else {
                ok_json(serde_json::json!({
                    "status": "gap",
                    "device_kind": "unavailable",
                    "message": message,
                }));
                77
            }
        }
        Err(ProbeError::NotPhysical(evidence)) => {
            if require_hardware {
                fail(&format!(
                    "adapter is {}, not a physical GPU",
                    evidence.device_type
                ))
            } else {
                ok_json(serde_json::json!({
                    "status": "ok",
                    "device_kind": "software",
                    "note": "a software adapter cannot satisfy --require-hardware",
                    "device": device::to_json(&evidence),
                }));
                0
            }
        }
        Err(ProbeError::Failed(message)) => fail(&message),
    }
}

#[cfg(not(feature = "device"))]
fn probe_command(require_hardware: bool) -> i32 {
    if require_hardware {
        fail("no physical adapter is available to this build (feature `device`, G09)")
    } else {
        ok_json(serde_json::json!({
            "status": "gap",
            "device_kind": "unavailable",
            "message": "device support is not built into this crate (feature `device`, G09)"
        }));
        77
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.as_slice() {
        [cmd] if cmd == "probe" => probe_command(false),
        [cmd, flag] if cmd == "probe" && flag == "--require-hardware" => probe_command(true),
        _ => match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
            ["validate", dir] => validate_command(dir),
            ["reference", dir, "--inputs", inputs] => reference_command(dir, inputs),
            _ => usage(),
        },
    };
    std::process::exit(code);
}
