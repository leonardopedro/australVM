//! G22b: the reduction artifact (`austral.gpu.reduction.v1`).
//!
//! A reduction bundle is a directory with `reduce.json` and `reduce.wgsl`.
//! It is deliberately separate from the pointwise `kernel.json` schema: the
//! v1 bundle bytes and their validation must not change, and a reduction has
//! two entries plus a program identity the pointwise format does not describe.
//! The per-run launch plan is *not* part of the artifact; the host derives it
//! with [`crate::reduce::schedule`] from validated input.

use crate::reduce;
use crate::schema::{self, KernelIr, Source};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SCHEMA: &str = "austral.gpu.reduction.v1";
pub const BACKEND: &str = "wgsl";
pub const MAPPER_ENTRY: &str = "map_sum";
pub const PARTIAL_ENTRY: &str = "add_partials";
pub const MAPPER_ROLE: &str = "mapper-input";
pub const PARTIAL_ROLE: &str = "partial-input";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub role: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReduceBundle {
    pub schema: String,
    pub backend: String,
    pub operation: String,
    pub reduction_profile: String,
    pub group_size: u32,
    pub scratch_bytes: u32,
    pub max_inputs: u32,
    pub identity: u32,
    pub operator: String,
    pub entries: Vec<Entry>,
    pub program_sha256: String,
    pub program_text: String,
    pub sources: Vec<Source>,
    /// The mapper IR: the scalar function the reduction applies.
    pub kernel_ir: KernelIr,
    pub ir_sha256: String,
    pub wgsl_sha256: String,
    pub evidence: schema::Evidence,
}

pub struct LoadedReduction {
    pub bundle: ReduceBundle,
    pub wgsl: String,
}

impl LoadedReduction {
    /// The layout the artifact declares. `load_reduce` has already refused an
    /// unknown operation, so this is the same value it validated against.
    pub fn strategy(&self) -> reduce::Strategy {
        reduce::Strategy::from_operation(&self.bundle.operation)
            .expect("load_reduce accepts only known operations")
    }
}

/// The program identity text, byte-identical to
/// `Gpu_reduce_wgsl.program_text`. The mapper digest is the canonical KernelIR
/// digest the pointwise path already reproduces, so both artifacts identify
/// the same scalar function. The operation selects the layout, and therefore
/// the profile line; the two artifacts must never share a program digest.
pub fn program_text(operation: &str, ir: &KernelIr) -> Result<String, String> {
    let strategy = reduce::Strategy::from_operation(operation)
        .ok_or_else(|| format!("unknown reduction operation {operation:?}"))?;
    let mapper_ir = schema::sha256_hex(&ir.canonical_bytes()?);
    Ok(format!(
        "operation={}\nprofile={}\nentries={},{}\ngroup={}\nscratch-bytes={}\nmapper-ir={}\n",
        strategy.operation(),
        strategy.profile(),
        MAPPER_ENTRY,
        PARTIAL_ENTRY,
        reduce::TILE,
        reduce::SCRATCH_BYTES,
        mapper_ir
    ))
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if metadata.len() > limit {
        return Err(format!(
            "{} is {} bytes, over the {}-byte limit",
            path.display(),
            metadata.len(),
            limit
        ));
    }
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

pub fn load_reduce(dir: &Path) -> Result<LoadedReduction, String> {
    let json_path = dir.join("reduce.json");
    let wgsl_path = dir.join("reduce.wgsl");
    if !json_path.exists() {
        return Err(format!("no reduce.json in {}", dir.display()));
    }
    if !wgsl_path.exists() {
        return Err(format!("no reduce.wgsl in {}", dir.display()));
    }
    let bytes = read_limited(&json_path, crate::artifact::MAX_ARTIFACT_BYTES)?;
    crate::artifact::strict_json_check(&bytes)
        .map_err(|e| format!("reduce.json is not strict JSON: {e}"))?;
    let bundle: ReduceBundle = serde_json::from_slice(&bytes)
        .map_err(|e| format!("reduce.json does not match {SCHEMA}: {e}"))?;
    let wgsl_bytes = read_limited(&wgsl_path, crate::artifact::MAX_ARTIFACT_BYTES)?;
    let wgsl = String::from_utf8(wgsl_bytes)
        .map_err(|e| format!("reduce.wgsl is not UTF-8: {e}"))?;

    if bundle.schema != SCHEMA {
        return Err(format!("reduce.json schema is {}, expected {SCHEMA}", bundle.schema));
    }
    if bundle.backend != BACKEND {
        return Err(format!("reduce.json backend is {}, expected {BACKEND}", bundle.backend));
    }
    let strategy = reduce::Strategy::from_operation(&bundle.operation).ok_or_else(|| {
        format!(
            "reduce.json operation is {:?}, expected {:?} or {:?}",
            bundle.operation,
            reduce::OPERATION,
            reduce::OPERATION_X4
        )
    })?;
    if bundle.reduction_profile != strategy.profile() {
        return Err(format!(
            "reduce.json reduction_profile is {:?}, expected {:?} for operation {:?}",
            bundle.reduction_profile,
            strategy.profile(),
            strategy.operation()
        ));
    }
    let _ = strategy;
    let scratch = (reduce::TILE * reduce::BYTES_PER_ELEMENT) as u32;
    if bundle.group_size != reduce::TILE as u32 || bundle.scratch_bytes != scratch {
        return Err("reduce.json group/scratch shape disagrees with the v1 schedule".to_string());
    }
    if bundle.max_inputs as usize != reduce::MAX_INPUTS
        || bundle.identity != reduce::IDENTITY
        || bundle.operator != "wrapping-add-u32"
    {
        return Err(
            "reduce.json identity/maximum/operator disagrees with the v1 definition".to_string(),
        );
    }
    if bundle.kernel_ir.input_arity != 1 {
        return Err("a reduction mapper takes exactly one Nat32".to_string());
    }
    if bundle.kernel_ir.element()? != schema::Element::U32 {
        return Err("a reduction mapper must use the u32-modular-v1 profile".to_string());
    }
    let ir_sha = schema::sha256_hex(&bundle.kernel_ir.canonical_bytes()?);
    if bundle.ir_sha256 != ir_sha {
        return Err(format!(
            "ir_sha256 does not match the mapper IR (bundle {}, computed {ir_sha})",
            bundle.ir_sha256
        ));
    }
    let wgsl_sha = schema::sha256_hex(wgsl.as_bytes());
    if bundle.wgsl_sha256 != wgsl_sha {
        return Err(format!(
            "wgsl_sha256 does not match reduce.wgsl (bundle {}, computed {wgsl_sha})",
            bundle.wgsl_sha256
        ));
    }
    let expected_text = program_text(&bundle.operation, &bundle.kernel_ir)?;
    if bundle.program_text != expected_text {
        return Err("reduce.json program_text does not describe this mapper".to_string());
    }
    let program_sha = schema::sha256_hex(expected_text.as_bytes());
    if bundle.program_sha256 != program_sha {
        return Err(format!(
            "program_sha256 does not match the program text (bundle {}, computed {program_sha})",
            bundle.program_sha256
        ));
    }
    let names: Vec<(&str, &str)> = bundle
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.role.as_str()))
        .collect();
    if names != [(MAPPER_ENTRY, MAPPER_ROLE), (PARTIAL_ENTRY, PARTIAL_ROLE)] {
        return Err(format!(
            "reduce.json entries must be {MAPPER_ENTRY}/{MAPPER_ROLE} then {PARTIAL_ENTRY}/{PARTIAL_ROLE}"
        ));
    }
    for entry in &bundle.entries {
        let digest = &entry.digest;
        if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("entry {} digest is not a sha256", entry.name));
        }
    }
    Ok(LoadedReduction { bundle, wgsl })
}
