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
        "usage:\n  avm-gpu validate BUNDLE\n  avm-gpu reference BUNDLE --inputs FILE\n  avm-gpu probe [--require-hardware] [--list] [--adapter NAME | --adapter-index N]\n  avm-gpu run BUNDLE --inputs FILE [--require-hardware] [--adapter NAME | --adapter-index N]"
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
                    "numeric_profile": loaded.bundle.numeric_profile,
                    "element": loaded.bundle.kernel_ir.element().ok().map(|e| e.name()).unwrap_or("?"),
                    "ir_sha256": loaded.bundle.ir_sha256,
                    "wgsl_sha256": loaded.bundle.wgsl_sha256,
                }));
                0
            }
        },
    }
}

fn read_inputs(path: &str, element: schema::Element) -> Result<Vec<Vec<u32>>, String> {
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
            match element {
                schema::Element::U32 => {
                    let v = n.as_u64().ok_or_else(|| {
                        format!("inputs[{i}][{j}] is not an unsigned integer")
                    })?;
                    if v > u32::MAX as u64 {
                        return Err(format!(
                            "inputs[{i}][{j}] = {v} is outside [0,4294967295]"
                        ));
                    }
                    values.push(v as u32);
                }
                schema::Element::F32 => {
                    let v = n
                        .as_f64()
                        .ok_or_else(|| format!("inputs[{i}][{j}] is not a number"))?;
                    let f = v as f32;
                    if !f.is_finite() {
                        return Err(format!(
                            "inputs[{i}][{j}] = {v} is not a finite binary32 value"
                        ));
                    }
                    values.push(f.to_bits());
                }
            }
        }
        out.push(values);
    }
    Ok(out)
}

/// Render output words as profile values: u32 integers for the default
/// profile, finite f32 numbers for wgsl-f32-v1. A non-finite f32 result is a
/// profile violation, not something to print as null.
fn output_values(loaded: &avm_gpu::LoadedBundle, words: &[u32]) -> Result<serde_json::Value, String> {
    match loaded.bundle.kernel_ir.element()? {
        schema::Element::U32 => Ok(serde_json::json!(words)),
        schema::Element::F32 => {
            let mut values = Vec::with_capacity(words.len());
            for word in words {
                let v = f32::from_bits(*word);
                if !v.is_finite() {
                    return Err(
                        "output contains a non-finite f32, which wgsl-f32-v1 does not admit"
                            .to_string(),
                    );
                }
                values.push(v as f64);
            }
            Ok(serde_json::json!(values))
        }
    }
}

fn reference_command(dir: &str, inputs_path: &str) -> i32 {
    let loaded = match load(dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_bundle(&loaded) {
        return fail(&e);
    }
    let element = match loaded.bundle.kernel_ir.element() {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    let inputs = match read_inputs(inputs_path, element) {
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
            let values = match output_values(&loaded, &output) {
                Ok(v) => v,
                Err(e) => return fail(&e),
            };
            ok_json(serde_json::json!({
                "status": "ok",
                "backend": "reference",
                "numeric_profile": loaded.bundle.numeric_profile,
                "element": element.name(),
                "entry": loaded.bundle.entry,
                "input_arity": loaded.bundle.input_arity,
                "ir_sha256": loaded.bundle.ir_sha256,
                "wgsl_sha256": loaded.bundle.wgsl_sha256,
                "dispatch_count": if inputs.first().map(|v| v.is_empty()).unwrap_or(true) { 0 } else { 1 },
                "output": values,
            }));
            0
        }
    }
}

#[cfg(feature = "device")]
fn probe_command(
    require_hardware: bool,
    name: Option<String>,
    index: Option<usize>,
    list: bool,
) -> i32 {
    use avm_gpu::device::{self, ProbeError};
    let selection = &device::Selection { name, index };
    if list {
        let adapters = device::enumerate();
        ok_json(serde_json::json!({
            "status": "ok",
            "adapters": adapters.iter().map(device::to_json).collect::<Vec<_>>(),
        }));
        return 0;
    }
    match device::probe(selection) {
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
fn probe_command(
    require_hardware: bool,
    _name: Option<String>,
    _index: Option<usize>,
    _list: bool,
) -> i32 {
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

#[cfg(feature = "device")]
fn run_command(
    dir: &str,
    inputs_path: &str,
    require_hardware: bool,
    name: Option<String>,
    index: Option<usize>,
) -> i32 {
    use avm_gpu::run::{run, RunFailure};
    let selection = avm_gpu::device::Selection { name, index };
    let loaded = match load(dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_bundle(&loaded) {
        return fail(&e);
    }
    let element = match loaded.bundle.kernel_ir.element() {
        Ok(e) => e,
        Err(e) => return fail(&e),
    };
    let inputs = match read_inputs(inputs_path, element) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    match run(&loaded, &inputs, require_hardware, &selection) {
        Ok(result) => {
            let device = avm_gpu::device::to_json(&result.adapter);
            let values = match output_values(&loaded, &result.output) {
                Ok(v) => v,
                Err(e) => return fail(&e),
            };
            ok_json(serde_json::json!({
                "status": "ok",
                "backend": "wgpu",
                "numeric_profile": loaded.bundle.numeric_profile,
                "element": element.name(),
                "entry": loaded.bundle.entry,
                "input_arity": loaded.bundle.input_arity,
                "ir_sha256": loaded.bundle.ir_sha256,
                "wgsl_sha256": loaded.bundle.wgsl_sha256,
                "device": device,
                "dispatch_count": result.dispatch_count,
                "elements": result.preflight.n,
                "timing_ms": {
                    "prepare": result.prepare_ms,
                    "upload": result.upload_ms,
                    "submit": result.submit_ms,
                    "readback": result.readback_ms,
                },
                "output": values,
            }));
            0
        }
        Err(RunFailure::Unavailable(message)) => {
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
        Err(RunFailure::Refused(message)) => fail(&message),
    }
}

#[cfg(not(feature = "device"))]
fn run_command(
    _dir: &str,
    _inputs_path: &str,
    require_hardware: bool,
    _name: Option<String>,
    _index: Option<usize>,
) -> i32 {
    if require_hardware {
        fail("device execution is not built into this crate (feature `device`, G10)")
    } else {
        ok_json(serde_json::json!({
            "status": "gap",
            "device_kind": "unavailable",
            "message": "device execution is not built into this crate (feature `device`, G10)",
        }));
        77
    }
}

/// Parses the flags shared by `probe` and `run`. Returns
/// `(require_hardware, adapter_name, adapter_index, list)`. `--inputs` is
/// accepted here so `run` can reuse the parser; its value is skipped.
type DeviceFlags = (bool, Option<String>, Option<usize>, bool, Vec<String>);

fn parse_device_flags(args: &[String]) -> Result<DeviceFlags, String> {
    let mut require = false;
    let mut list = false;
    let mut name: Option<String> = None;
    let mut index: Option<usize> = None;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--require-hardware" => require = true,
            "--list" => list = true,
            "--inputs" => {
                if args.get(i + 1).is_none() {
                    return Err("--inputs needs a value".to_string());
                }
                i += 1;
            }
            "--adapter" => {
                i += 1;
                let value = args.get(i).ok_or("--adapter needs a value")?;
                if index.is_some() {
                    return Err("--adapter and --adapter-index are exclusive".to_string());
                }
                if value.trim().is_empty() {
                    return Err("adapter name selector is empty".to_string());
                }
                name = Some(value.clone());
            }
            "--adapter-index" => {
                i += 1;
                let raw = args.get(i).ok_or("--adapter-index needs a value")?;
                if name.is_some() {
                    return Err("--adapter and --adapter-index are exclusive".to_string());
                }
                index = Some(
                    raw.parse::<usize>()
                        .map_err(|_| format!("--adapter-index {raw:?} is not an index"))?,
                );
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other:?}")),
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    Ok((require, name, index, list, positional))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let device_subcommand = matches!(args.first().map(String::as_str), Some("probe" | "run"));
    let (require, adapter_name, adapter_index, list, extra_positional) = if device_subcommand {
        match parse_device_flags(&args[1..]) {
            Ok(flags) => flags,
            Err(message) => {
                eprintln!(
                    "{}",
                    serde_json::json!({ "status": "error", "message": message })
                );
                std::process::exit(2);
            }
        }
    } else {
        (false, None, None, false, Vec::new())
    };
    let code = match args.first().map(String::as_str) {
        Some("validate") if args.len() == 2 => validate_command(&args[1]),
        Some("reference") if args.len() == 4 && args[2] == "--inputs" => {
            reference_command(&args[1], &args[3])
        }
        Some("probe") => {
            if !extra_positional.is_empty() {
                usage()
            } else if list && require {
                eprintln!(
                    "{}",
                    serde_json::json!({"status": "error", "message": "--list and --require-hardware are exclusive"})
                );
                2
            } else {
                probe_command(require, adapter_name, adapter_index, list)
            }
        }
        Some("run") => {
            let mut positional = Vec::new();
            let mut i = 1;
            let mut inputs = None;
            while i < args.len() {
                match args[i].as_str() {
                    "--inputs" | "--adapter" | "--adapter-index" => {
                        if args[i].as_str() == "--inputs" {
                            inputs = args.get(i + 1).cloned();
                        }
                        i += 2;
                        continue;
                    }
                    flag if flag.starts_with("--") => {}
                    other => positional.push(other.to_string()),
                }
                i += 1;
            }
            match (positional.as_slice(), inputs) {
                ([dir], Some(inputs)) => {
                    run_command(dir, &inputs, require, adapter_name, adapter_index)
                }
                _ => usage(),
            }
        }
        _ => usage(),
    };
    std::process::exit(code);
}


