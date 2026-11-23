//! `avm-target` — PC10: the GPU virtual/native boundary, measured.
//!
//! ```text
//! avm-target claim BUNDLE --tier TIER --out FILE [--gap TEXT]
//! avm-target capture BUNDLE --out DIR [--device N]
//! avm-target identify FILE
//! avm-target check RECORD [--policy TIER] [--blob FILE] [--ptx FILE]
//! avm-target link PTX --out DIR [--device N]
//! ```
//!
//! `claim` records what a published bundle *is* (a virtual artifact at `--tier`,
//! with the digest of that tier's bytes and the reason no native artifact
//! exists). `capture` links the bundle's PTX into a native executable with the
//! driver's own linker, identifies the container and writes the record beside
//! the bytes. `check` enforces a policy tier against a record: a virtual record
//! cannot satisfy `--policy native`, and a native record is only satisfied by
//! the bytes it recorded, re-identified.
//!
//! Exit codes: 0 the claim holds; 1 refused (below policy, unidentifiable,
//! different bytes); 2 usage; 3 uncaptured (no bytes to identify); 77 the
//! driver or device is unavailable.

use avm_gpu::nvlaunch;
use avm_gpu::target::{self, Abi, Blob, BoundaryRecord, Refusal, Tier};
// Only the `--features nvidia` capture path records the device it linked for.
#[cfg(feature = "nvidia")]
use avm_gpu::target::DeviceFacts;
use std::path::{Path, PathBuf};

fn usage() -> i32 {
    eprintln!(
        "usage:\n  avm-target claim BUNDLE --tier TIER --out FILE [--gap TEXT]\n  \
         avm-target capture BUNDLE --out DIR [--device N]\n  \
         avm-target identify FILE\n  \
         avm-target check RECORD [--policy TIER] [--blob FILE] [--ptx FILE]\n  \
         avm-target link PTX --out DIR [--device N]"
    );
    2
}

fn refuse(refusal: &Refusal) -> i32 {
    println!(
        "{}",
        serde_json::json!({
            "status": "refused",
            "refusal": refusal.to_string(),
            "limits": target::LIMITS,
        })
    );
    1
}

fn uncaptured(message: &str) -> i32 {
    println!(
        "{}",
        serde_json::json!({"status": "uncaptured", "message": message, "limits": target::LIMITS})
    );
    3
}

fn unavailable(message: &str) -> i32 {
    eprintln!("{}", serde_json::json!({"status": "unavailable", "message": message}));
    77
}

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// The bundle facts the ABI is built from (`austral.gpu.mlir.v1`).
struct Bundle {
    entry: String,
    arity: usize,
    extent: usize,
    params: usize,
    workgroup: usize,
    ptx_sha256: String,
    ptx: String,
}

fn read_bundle(dir: &Path) -> Result<Bundle, String> {
    let meta = std::fs::read_to_string(dir.join("bundle.json"))
        .map_err(|e| format!("cannot read {}: {e}", dir.join("bundle.json").display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&meta).map_err(|e| format!("bundle.json is not JSON: {e}"))?;
    let schema = v.get("schema").and_then(|s| s.as_str()).unwrap_or("");
    if schema != "austral.gpu.mlir.v1" {
        return Err(format!("bundle.json is schema {schema:?}, not austral.gpu.mlir.v1"));
    }
    let number = |path: &[&str]| -> Result<usize, String> {
        let mut cur = &v;
        for key in path {
            cur = cur
                .get(*key)
                .ok_or_else(|| format!("bundle.json has no {}", path.join(".")))?;
        }
        cur.as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| format!("bundle.json {} is not a number", path.join(".")))
    };
    let entry = v
        .get("entry")
        .and_then(|s| s.as_str())
        .ok_or("bundle.json has no entry")?
        .to_string();
    let ptx_path = dir.join("kernel.ptx");
    let ptx = std::fs::read_to_string(&ptx_path)
        .map_err(|e| format!("cannot read {}: {e}", ptx_path.display()))?;
    let ptx_sha256 = sha256_hex(ptx.as_bytes());
    let recorded = v
        .get("derivatives")
        .and_then(|d| d.as_array())
        .and_then(|rows| rows.iter().find(|r| r.get("stage").and_then(|s| s.as_str()) == Some("ptx")))
        .and_then(|r| r.get("sha256"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    if recorded != ptx_sha256 {
        return Err(format!(
            "kernel.ptx hashes to {ptx_sha256}, the bundle records {recorded}: the artifact on \
             disk is not the one the bundle vouches for"
        ));
    }
    Ok(Bundle {
        entry,
        arity: number(&["input_arity"])?,
        extent: number(&["extent"])?,
        params: number(&["abi", "params"])?,
        workgroup: number(&["workgroup"])?,
        ptx_sha256,
        ptx,
    })
}

/// Preflight the PTX against the bundle and derive the ABI/geometry. The
/// capability is `None` for a claim (no device is consulted).
fn abi_of(bundle: &Bundle, capability: Option<(i32, i32)>) -> Result<Abi, String> {
    let facts = nvlaunch::ptx_facts(&bundle.ptx)?;
    let plan = nvlaunch::plan(
        &facts,
        bundle.params,
        bundle.arity,
        bundle.extent,
        bundle.extent,
        bundle.arity,
        bundle.workgroup,
        None,
        capability,
    )
    .map_err(|r| r.to_string())?;
    Ok(Abi {
        entry: facts.entry,
        params: facts.params,
        extent: plan.extent,
        workgroup: bundle.workgroup,
        block: plan.block,
        grid: plan.grid,
    })
}

fn claim(args: &[String]) -> i32 {
    let (Some(dir), Some(tier), Some(out)) = (args.first(), arg(args, "--tier"), arg(args, "--out")) else {
        return usage();
    };
    let Some(tier) = Tier::parse(tier) else {
        return refuse(&Refusal::UnknownTier { text: arg(args, "--tier").unwrap_or("").to_string() });
    };
    if tier == Tier::Native {
        return refusal_for_native_claim();
    }
    let bundle = match read_bundle(Path::new(dir)) {
        Ok(b) => b,
        Err(e) => return uncaptured(&e),
    };
    let abi = match abi_of(&bundle, None) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status": "error", "message": e}));
            return 1;
        }
    };
    let gap = arg(args, "--gap")
        .unwrap_or("no native artifact is captured for this subject")
        .to_string();
    // A `ptx` claim names the PTX bytes; a `wgsl` claim names the WGSL bundle.
    // This binary only reads MLIR bundles (which publish PTX), so it refuses a
    // claim about bytes it did not read rather than inventing a digest.
    if tier != Tier::Ptx {
        return uncaptured(&format!(
            "avm-target reads MLIR bundles, whose concrete artifact is PTX; a {} claim must \
             name its own bytes",
            tier.as_str()
        ));
    }
    let record = BoundaryRecord::virtual_artifact(
        &bundle.entry,
        tier,
        bundle.ptx_sha256.clone(),
        abi,
        gap,
    );
    match std::fs::write(out, serde_json::to_string_pretty(&record).unwrap() + "\n") {
        Ok(()) => {
            println!(
                "{}",
                serde_json::json!({
                    "status": "ok",
                    "tier": record.tier.as_str(),
                    "artifact_sha256": record.artifact_sha256,
                    "out": out,
                })
            );
            0
        }
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status": "error", "message": format!("cannot write {out}: {e}")}));
            1
        }
    }
}

fn refusal_for_native_claim() -> i32 {
    uncaptured("a native claim needs captured bytes; use `avm-target capture`")
}

#[cfg(not(feature = "nvidia"))]
fn capture(_args: &[String]) -> i32 {
    unavailable("this build has no CUDA driver path (build with --features nvidia)")
}

#[cfg(feature = "nvidia")]
fn capture(args: &[String]) -> i32 {
    let (Some(dir), Some(out)) = (args.first(), arg(args, "--out")) else {
        return usage();
    };
    let device: i32 = arg(args, "--device").and_then(|d| d.parse().ok()).unwrap_or(0);
    let bundle = match read_bundle(Path::new(dir)) {
        Ok(b) => b,
        Err(e) => return uncaptured(&e),
    };
    let driver = match nvlaunch::driver::Driver::open(device) {
        Ok(d) => d,
        Err(e) => return unavailable(&e),
    };
    let abi = match abi_of(&bundle, Some(driver.device.capability)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status": "error", "message": e}));
            return 1;
        }
    };
    let image = match driver.link(&bundle.ptx) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status": "error", "message": format!("the driver refused to link the PTX: {e}")}));
            return 1;
        }
    };
    let blob = match target::identify("native.cubin", &image) {
        Ok(b) => b,
        Err(r) => return refuse(&r),
    };
    let out = PathBuf::from(out);
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("{}", serde_json::json!({"status": "error", "message": format!("cannot create {}: {e}", out.display())}));
        return 1;
    }
    if let Err(e) = std::fs::write(out.join("native.cubin"), &image) {
        eprintln!("{}", serde_json::json!({"status": "error", "message": format!("cannot write the blob: {e}")}));
        return 1;
    }
    let record = BoundaryRecord {
        schema: target::SCHEMA.to_string(),
        entry: bundle.entry.clone(),
        tier: Tier::Native,
        artifact_sha256: sha256_hex(&image),
        ptx_sha256: Some(bundle.ptx_sha256.clone()),
        native_gap: None,
        abi,
        device: Some(DeviceFacts {
            name: driver.device.name.clone(),
            capability: format!("{}.{}", driver.device.capability.0, driver.device.capability.1),
            chip: format!("sm_{}{}", driver.device.capability.0, driver.device.capability.1),
            driver_version: driver.device.driver_version,
        }),
        blob: Some(blob.clone()),
        limits: target::LIMITS.to_string(),
    };
    let text = serde_json::to_string_pretty(&record).unwrap() + "\n";
    if let Err(e) = std::fs::write(out.join("native.json"), text) {
        eprintln!("{}", serde_json::json!({"status": "error", "message": format!("cannot write the record: {e}")}));
        return 1;
    }
    println!(
        "{}",
        serde_json::json!({
            "status": "ok",
            "tier": "native",
            "device": record.device,
            "artifact_sha256": record.artifact_sha256,
            "sass_bytes": blob.sass_bytes,
            "size": blob.size,
            "sections": blob.sections.len(),
            "limits": target::LIMITS,
        })
    );
    0
}

#[cfg(not(feature = "nvidia"))]
fn link(_args: &[String]) -> i32 {
    unavailable("this build has no CUDA driver path (build with --features nvidia)")
}

/// Link PTX text with no bundle behind it. Used by the mutation controls: the
/// bytes are real, identified, and must still fail a record that names other
/// bytes.
#[cfg(feature = "nvidia")]
fn link(args: &[String]) -> i32 {
    use std::path::PathBuf;
    let (Some(ptx), Some(out)) = (args.first(), arg(args, "--out")) else {
        return usage();
    };
    let device: i32 = arg(args, "--device").and_then(|d| d.parse().ok()).unwrap_or(0);
    let text = match std::fs::read_to_string(ptx) {
        Ok(t) => t,
        Err(e) => return uncaptured(&format!("cannot read {ptx}: {e}")),
    };
    let driver = match nvlaunch::driver::Driver::open(device) {
        Ok(d) => d,
        Err(e) => return unavailable(&e),
    };
    let image = match driver.link(&text) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status": "error", "message": format!("the driver refused to link the PTX: {e}")}));
            return 1;
        }
    };
    let blob = match target::identify("native.cubin", &image) {
        Ok(b) => b,
        Err(r) => return refuse(&r),
    };
    let out = PathBuf::from(out);
    if let Err(e) = std::fs::create_dir_all(&out).and_then(|_| std::fs::write(out.join("native.cubin"), &image)) {
        eprintln!("{}", serde_json::json!({"status": "error", "message": format!("cannot write the linked blob: {e}")}));
        return 1;
    }
    println!(
        "{}",
        serde_json::json!({
            "status": "ok",
            "tier": "native",
            "artifact_sha256": sha256_hex(&image),
            "sass_bytes": blob.sass_bytes,
            "size": blob.size,
            "limits": target::LIMITS,
        })
    );
    0
}

fn identify(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        return usage();
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return uncaptured(&format!("cannot read {path}: {e}")),
    };
    match target::identify(path, &bytes) {
        Ok(blob) => {
            println!(
                "{}",
                serde_json::json!({
                    "status": "ok",
                    "sha256": sha256_hex(&bytes),
                    "blob": blob,
                    "limits": target::LIMITS,
                })
            );
            0
        }
        Err(r) => refuse(&r),
    }
}

fn check(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        return usage();
    };
    let required = match arg(args, "--policy").unwrap_or("native") {
        text => match Tier::parse(text) {
            Some(t) => t,
            None => return refuse(&Refusal::UnknownTier { text: text.to_string() }),
        },
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return uncaptured(&format!("cannot read {path}: {e}")),
    };
    let record: BoundaryRecord = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status": "error", "message": format!("{path} is not a boundary record: {e}")}));
            return 1;
        }
    };
    if record.schema != target::SCHEMA {
        eprintln!(
            "{}",
            serde_json::json!({"status": "error", "message": format!("{path} is schema {:?}, not {}", record.schema, target::SCHEMA)})
        );
        return 1;
    }
    // The PTX the record claims to have been linked from, when supplied, must
    // be the one this check reads: the blob is only as good as that edge.
    if let Some(ptx) = arg(args, "--ptx") {
        let recorded = record
            .ptx_sha256
            .as_deref()
            .or(if record.tier == Tier::Ptx {
                Some(record.artifact_sha256.as_str())
            } else {
                None
            });
        let Some(recorded) = recorded else {
            return refuse(&Refusal::NoSubject);
        };
        match std::fs::read(ptx) {
            Ok(bytes) => {
                let got = sha256_hex(&bytes);
                if got != recorded {
                    return refuse(&Refusal::Identity {
                        recorded: recorded.to_string(),
                        got,
                    });
                }
            }
            Err(e) => return uncaptured(&format!("cannot read {ptx}: {e}")),
        }
    }
    let bytes: Option<Vec<u8>> = if record.tier == Tier::Native {
        let blob_path = match arg(args, "--blob") {
            Some(b) => PathBuf::from(b),
            None => {
                let dir = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
                let name = record
                    .blob
                    .as_ref()
                    .map(|b| b.file.clone())
                    .unwrap_or_else(|| "native.cubin".to_string());
                dir.join(name)
            }
        };
        match std::fs::read(&blob_path) {
            Ok(b) => Some(b),
            Err(e) => {
                return uncaptured(&format!("cannot read {}: {e}", blob_path.display()));
            }
        }
    } else {
        None
    };
    match record.verify(required, bytes.as_deref()) {
        Ok(()) => {
            let blob: Option<Blob> = record.blob.clone();
            println!(
                "{}",
                serde_json::json!({
                    "status": "ok",
                    "tier": record.tier.as_str(),
                    "policy": required.as_str(),
                    "artifact_sha256": record.artifact_sha256,
                    "sass_bytes": blob.as_ref().map(|b| b.sass_bytes),
                    "limits": target::LIMITS,
                })
            );
            0
        }
        Err(r) => refuse(&r),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(|s| s.as_str()) {
        Some("claim") => claim(&args[2..]),
        Some("capture") => capture(&args[2..]),
        Some("identify") => identify(&args[2..]),
        Some("check") => check(&args[2..]),
        Some("link") => link(&args[2..]),
        _ => usage(),
    };
    std::process::exit(code);
}
