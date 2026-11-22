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
        "usage:\n  avm-gpu validate BUNDLE\n  avm-gpu reference BUNDLE --inputs FILE\n  avm-gpu reflect-wgsl BUNDLE [--out DIR]\n  avm-gpu check-reflect BUNDLE --artifact FILE\n  avm-gpu validate-reduce REDUCTION\n  avm-gpu reference-reduce REDUCTION --inputs FILE\n  avm-gpu probe [--require-hardware] [--list] [--adapter NAME | --adapter-index N]\n  avm-gpu run BUNDLE --inputs FILE [--require-hardware] [--adapter NAME | --adapter-index N]\n  avm-gpu run-reduce REDUCTION --inputs FILE [--require-hardware] [--adapter NAME | --adapter-index N]\n  avm-gpu compare-reduce REDUCTION --map BUNDLE --inputs FILE [--iterations N] [--require-hardware] [--adapter NAME | --adapter-index N]"
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

fn load_reduce(dir: &str) -> Result<avm_gpu::LoadedReduction, String> {
    avm_gpu::reduce_artifact::load_reduce(Path::new(dir))
}

/// A reduction takes exactly one input array; the pointwise input format is
/// reused so the two command families read the same file shape.
fn read_reduce_input(path: &str) -> Result<Vec<u32>, String> {
    let inputs = read_inputs(path, schema::Element::U32)?;
    match inputs.len() {
        1 => Ok(inputs.into_iter().next().unwrap()),
        n => Err(format!(
            "a reduction takes exactly one input array, the request carries {n}"
        )),
    }
}

fn validate_reduce_command(dir: &str) -> i32 {
    let loaded = match load_reduce(dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_reduction(&loaded) {
        return fail(&e);
    }
    let bundle = &loaded.bundle;
    ok_json(serde_json::json!({
        "schema": bundle.schema,
        "status": "ok",
        "operation": bundle.operation,
        "reduction_profile": bundle.reduction_profile,
        "group_size": bundle.group_size,
        "scratch_bytes": bundle.scratch_bytes,
        "identity": bundle.identity,
        "operator": bundle.operator,
        "entries": bundle.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        "program_sha256": bundle.program_sha256,
        "ir_sha256": bundle.ir_sha256,
        "wgsl_sha256": bundle.wgsl_sha256,
    }));
    0
}

fn reference_reduce_command(dir: &str, inputs_path: &str) -> i32 {
    let loaded = match load_reduce(dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_reduction(&loaded) {
        return fail(&e);
    }
    let input = match read_reduce_input(inputs_path) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    match avm_gpu::reduce_run::cpu_reduce(
        loaded.strategy(),
        &loaded.bundle.kernel_ir,
        &input,
    ) {
        Err(e) => fail(&e),
        Ok(output) => {
            ok_json(serde_json::json!({
                "status": "ok",
                "backend": "reference",
                "operation": loaded.bundle.operation,
                "reduction_profile": loaded.bundle.reduction_profile,
                "elements": input.len(),
                "dispatch_count": avm_gpu::reduce_run::preflight(
                    loaded.strategy(),
                    input.len(),
                    &avm_gpu::run::LimitsView {
                        max_buffer_size: u64::MAX,
                        max_storage_buffer_binding_size: u64::MAX,
                        max_compute_workgroups_per_dimension: u32::MAX,
                        max_compute_workgroup_size_x: u32::MAX,
                    },
                ).map(|p| p.groups.len()).unwrap_or(0),
                "output": output,
            }));
            0
        }
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

#[cfg(feature = "device")]
fn compare_reduce_command(
    dir: &str,
    map_dir: &str,
    inputs_path: &str,
    require_hardware: bool,
    name: Option<String>,
    index: Option<usize>,
    iterations: u32,
) -> i32 {
    use avm_gpu::reduce_run::{run_with_iterations, ReduceFailure};
    let selection = avm_gpu::device::Selection { name, index };
    let loaded = match load_reduce(dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_reduction(&loaded) {
        return fail(&e);
    }
    let map = match load(map_dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_bundle(&map) {
        return fail(&e);
    }
    if map.bundle.ir_sha256 != loaded.bundle.ir_sha256 {
        return fail("the map bundle and the reduction do not use the same mapper IR");
    }
    let input = match read_reduce_input(inputs_path) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    let iterations = iterations.max(1);

    let reduction = match run_with_iterations(
        &loaded,
        &input,
        require_hardware,
        &selection,
        iterations,
    ) {
        Ok(r) => r,
        Err(ReduceFailure::Unavailable(message)) => {
            if require_hardware {
                return fail(&format!("no physical adapter is available: {message}"));
            }
            ok_json(serde_json::json!({
                "status": "gap",
                "device_kind": "unavailable",
                "message": message,
            }));
            return 77;
        }
        Err(ReduceFailure::Refused(message)) => return fail(&message),
    };
    let mapped = match avm_gpu::run::run_with_iterations(
        &map,
        &[input.clone()],
        require_hardware,
        &selection,
        iterations,
    ) {
        Ok(r) => r,
        Err(avm_gpu::run::RunFailure::Unavailable(message)) => {
            return fail(&format!("map run found no adapter after the reduction: {message}"))
        }
        Err(avm_gpu::run::RunFailure::Refused(message)) => return fail(&message),
    };
    let fold_start = std::time::Instant::now();
    let folded = mapped
        .output
        .iter()
        .fold(avm_gpu::reduce::IDENTITY, |acc, v| acc.wrapping_add(*v));
    let fold_ms = fold_start.elapsed().as_secs_f64() * 1000.0;

    let input_bytes = input.len() as u64 * 4;
    let reduction_output_bytes: u64 = reduction.preflight.bytes.iter().sum();
    let agreement = folded == reduction.output;
    ok_json(serde_json::json!({
        "status": if agreement { "ok" } else { "disagree" },
        "backend": "wgpu",
        "operation": loaded.bundle.operation,
        "reduction_profile": loaded.bundle.reduction_profile,
        "program_sha256": loaded.bundle.program_sha256,
        "ir_sha256": loaded.bundle.ir_sha256,
        "elements": input.len(),
        "iterations": iterations,
        "device": avm_gpu::device::to_json(&reduction.adapter),
        "reduction": {
            "output": reduction.output,
            "passes": reduction.preflight.groups,
            "dispatch_count": reduction.dispatch_count,
            "bytes": {
                "upload": input_bytes,
                "device_output": reduction_output_bytes,
                "readback": 4,
                "total": input_bytes + reduction_output_bytes + 4,
            },
            "timing_ms": {
                "prepare": reduction.prepare_ms,
                "upload": reduction.upload_ms,
                "submit_total": reduction.submit_ms,
                "warm_submit": reduction.submit_ms / f64::from(iterations),
                "readback": reduction.readback_ms,
            },
        },
        "map_and_fold": {
            "folded_output": folded,
            "agree": agreement,
            "bytes": {
                "upload": input_bytes,
                "device_output": input_bytes,
                "readback": input_bytes,
                "total": input_bytes * 3,
            },
            "timing_ms": {
                "prepare": mapped.prepare_ms,
                "upload": mapped.upload_ms,
                "submit_total": mapped.submit_ms,
                "warm_submit": mapped.submit_ms / f64::from(iterations),
                "readback": mapped.readback_ms,
                "cpu_fold": fold_ms,
            },
        },
    }));
    if agreement { 0 } else { 1 }
}

#[cfg(not(feature = "device"))]
fn compare_reduce_command(
    _dir: &str,
    _map_dir: &str,
    _inputs_path: &str,
    require_hardware: bool,
    _name: Option<String>,
    _index: Option<usize>,
    _iterations: u32,
) -> i32 {
    if require_hardware {
        fail("device execution is not built into this crate (feature `device`, G22c)")
    } else {
        ok_json(serde_json::json!({
            "status": "gap",
            "device_kind": "unavailable",
            "message": "device execution is not built into this crate (feature `device`, G22c)",
        }));
        77
    }
}

#[cfg(feature = "device")]
fn run_reduce_command(
    dir: &str,
    inputs_path: &str,
    require_hardware: bool,
    name: Option<String>,
    index: Option<usize>,
) -> i32 {
    use avm_gpu::reduce_run::{run, ReduceFailure};
    let selection = avm_gpu::device::Selection { name, index };
    let loaded = match load_reduce(dir) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_reduction(&loaded) {
        return fail(&e);
    }
    let input = match read_reduce_input(inputs_path) {
        Ok(i) => i,
        Err(e) => return fail(&e),
    };
    match run(&loaded, &input, require_hardware, &selection) {
        Ok(result) => {
            let device = avm_gpu::device::to_json(&result.adapter);
            ok_json(serde_json::json!({
                "status": "ok",
                "backend": "wgpu",
                "operation": loaded.bundle.operation,
                "reduction_profile": loaded.bundle.reduction_profile,
                "program_sha256": loaded.bundle.program_sha256,
                "ir_sha256": loaded.bundle.ir_sha256,
                "wgsl_sha256": loaded.bundle.wgsl_sha256,
                "device": device,
                "dispatch_count": result.dispatch_count,
                "iterations": result.iterations,
                "elements": input.len(),
                "passes": result.preflight.groups,
                "pass_bytes": result.preflight.bytes,
                "total_bytes": result.preflight.total_bytes,
                "timing_ms": {
                    "prepare": result.prepare_ms,
                    "upload": result.upload_ms,
                    "submit": result.submit_ms,
                    "readback": result.readback_ms,
                },
                "output": result.output,
            }));
            0
        }
        Err(ReduceFailure::Unavailable(message)) => {
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
        Err(ReduceFailure::Refused(message)) => fail(&message),
    }
}

#[cfg(not(feature = "device"))]
fn run_reduce_command(
    _dir: &str,
    _inputs_path: &str,
    require_hardware: bool,
    _name: Option<String>,
    _index: Option<usize>,
) -> i32 {
    if require_hardware {
        fail("device execution is not built into this crate (feature `device`, G22c)")
    } else {
        ok_json(serde_json::json!({
            "status": "gap",
            "device_kind": "unavailable",
            "message": "device execution is not built into this crate (feature `device`, G22c)",
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
            "--inputs" | "--map" | "--iterations" => {
                if args.get(i + 1).is_none() {
                    return Err(format!("{} needs a value", args[i]));
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

/// PC05a: reflect the parsed WGSL of a bundle's clamp into the word model and
/// (with `--out`) emit the witness artifact. The wrapper audit runs first and
/// separately: it is reported as its own fact, not folded into the reflection.
fn reflect_wgsl_command(dir: &str, out: Option<&str>) -> i32 {
    let loaded = match load(dir) {
        Ok(loaded) => loaded,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_bundle(&loaded) {
        return fail(&e);
    }
    let wgsl = match avm_gpu::wgsl_reflect::reflect(&loaded.wgsl, loaded.bundle.input_arity) {
        Ok(w) => w,
        Err(e) => return fail(&e),
    };
    let ir = match avm_gpu::wgsl_reflect::reflect_ir(&loaded.bundle.kernel_ir) {
        Ok(ir) => ir,
        Err(e) => return fail(&e),
    };
    let joined = match avm_gpu::wgsl_reflect::witness(&wgsl, &ir) {
        Ok(joined) => joined,
        Err(e) => return fail(&e),
    };
    let (mm0, mm1) = avm_gpu::wgsl_reflect::render(
        &joined,
        &loaded.bundle.wgsl_sha256,
        &loaded.bundle.ir_sha256,
    );
    if let Some(out) = out {
        if let Err(e) = std::fs::create_dir_all(out) {
            return fail(&format!("cannot create {out}: {e}"));
        }
        for (name, text) in [("clamp_wgsl.mm0", &mm0), ("clamp_wgsl.mm1", &mm1)] {
            let path = Path::new(out).join(name);
            if let Err(e) = std::fs::write(&path, text) {
                return fail(&format!("cannot write {}: {e}", path.display()));
            }
        }
    }
    ok_json(serde_json::json!({
        "status": "ok",
        "operation": "reflect-wgsl",
        "reflect": avm_gpu::wgsl_reflect::describe(&joined),
        "wgsl_sha256": loaded.bundle.wgsl_sha256,
        "ir_sha256": loaded.bundle.ir_sha256,
        "wrapper_audit": "pass",
        "artifact": out,
    }));
    0
}

/// Re-reflect the bundle and compare a committed artifact against it.
fn check_reflect_command(dir: &str, artifact: &str) -> i32 {
    let loaded = match load(dir) {
        Ok(loaded) => loaded,
        Err(e) => return fail(&e),
    };
    if let Err(e) = validate::validate_bundle(&loaded) {
        return fail(&e);
    }
    let fresh = match avm_gpu::wgsl_reflect::reflect(&loaded.wgsl, loaded.bundle.input_arity) {
        Ok(w) => w,
        Err(e) => return fail(&e),
    };
    let ir = match avm_gpu::wgsl_reflect::reflect_ir(&loaded.bundle.kernel_ir) {
        Ok(ir) => ir,
        Err(e) => return fail(&e),
    };
    let fresh = match avm_gpu::wgsl_reflect::witness(&fresh, &ir) {
        Ok(joined) => joined,
        Err(e) => return fail(&e),
    };
    let text = match std::fs::read_to_string(artifact) {
        Ok(text) => text,
        Err(e) => return fail(&format!("cannot read {artifact}: {e}")),
    };
    if let Err(e) = avm_gpu::wgsl_reflect::check_artifact(
        &fresh,
        &loaded.bundle.wgsl_sha256,
        &loaded.bundle.ir_sha256,
        &text,
    ) {
        return fail(&e);
    }
    ok_json(serde_json::json!({
        "status": "ok",
        "operation": "check-reflect",
        "artifact": artifact,
        "reflect": avm_gpu::wgsl_reflect::describe(&fresh),
    }));
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let device_subcommand = matches!(
        args.first().map(String::as_str),
        Some("probe" | "run" | "run-reduce" | "compare-reduce")
    );
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
        Some("reflect-wgsl") if args.len() == 2 => reflect_wgsl_command(&args[1], None),
        Some("reflect-wgsl") if args.len() == 4 && args[2] == "--out" => {
            reflect_wgsl_command(&args[1], Some(&args[3]))
        }
        Some("check-reflect") if args.len() == 4 && args[2] == "--artifact" => {
            check_reflect_command(&args[1], &args[3])
        }
        Some("validate-reduce") if args.len() == 2 => validate_reduce_command(&args[1]),
        Some("reference-reduce") if args.len() == 4 && args[2] == "--inputs" => {
            reference_reduce_command(&args[1], &args[3])
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
        Some(subcommand) if subcommand == "compare-reduce" => {
            let mut positional = Vec::new();
            let mut i = 1;
            let mut inputs = None;
            let mut map_dir = None;
            let mut iterations = 1u32;
            while i < args.len() {
                match args[i].as_str() {
                    "--inputs" => {
                        inputs = args.get(i + 1).cloned();
                        i += 2;
                        continue;
                    }
                    "--map" => {
                        map_dir = args.get(i + 1).cloned();
                        i += 2;
                        continue;
                    }
                    "--iterations" => {
                        match args.get(i + 1).and_then(|v| v.parse::<u32>().ok()) {
                            Some(n) if n >= 1 => iterations = n,
                            _ => {
                                eprintln!(
                                    "{}",
                                    serde_json::json!({"status": "error", "message": "--iterations needs a positive integer"})
                                );
                                std::process::exit(2);
                            }
                        }
                        i += 2;
                        continue;
                    }
                    "--adapter" | "--adapter-index" => {
                        i += 2;
                        continue;
                    }
                    flag if flag.starts_with("--") => {}
                    other => positional.push(other.to_string()),
                }
                i += 1;
            }
            match (positional.as_slice(), inputs, map_dir) {
                ([dir], Some(inputs), Some(map_dir)) => compare_reduce_command(
                    dir,
                    &map_dir,
                    &inputs,
                    require,
                    adapter_name,
                    adapter_index,
                    iterations,
                ),
                _ => usage(),
            }
        }
        Some(subcommand) if subcommand == "run" || subcommand == "run-reduce" => {
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
                ([dir], Some(inputs)) if subcommand == "run" => {
                    run_command(dir, &inputs, require, adapter_name, adapter_index)
                }
                ([dir], Some(inputs)) => {
                    run_reduce_command(dir, &inputs, require, adapter_name, adapter_index)
                }
                _ => usage(),
            }
        }
        _ => usage(),
    };
    std::process::exit(code);
}


