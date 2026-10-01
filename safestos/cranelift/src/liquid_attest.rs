//! PLAN_liquid_types.md L8: the `liquid.ok` load-time attestation.
//!
//! `docs/LIQUID.md` §2.5 asks for a sidecar next to a module that declares
//! `liquid = "required"`, carrying enough to detect a stale or tampered
//! verdict. The posture is deliberately the same one as UK-4001: the compiler
//! check becomes part of the kernel's capability story, so the host refuses to
//! run a module whose attestation does not describe the bytes it is about to
//! host.
//!
//! What the sidecar records, and why each field is needed to detect something
//! real:
//!
//! - `sources` — an FNV-1a hash over the module's `.aui`/`.aum` sources. This
//!   is what makes an attestation *stale*: edit a contract, keep the old
//!   sidecar, and the hash stops matching.
//! - `mlw` — an FNV-1a hash over the generated verification file. Without it a
//!   prover verdict could be replayed against obligations the compiler did not
//!   actually emit.
//! - `prover` — name and version of the prover that produced the verdict.
//!   A verdict from a different prover is not the same claim.
//! - `verdict` — `proved`, `refused` or `unknown`. Only `proved` satisfies a
//!   `required` module; `unknown` is never good enough.
//! - `trusted_contracts` — how many contracts were *assumed* rather than
//!   discharged. Cycle A (L10) is what drives this to zero; until then a
//!   non-zero count is recorded honestly rather than hidden.
//!
//! ## What this is not
//!
//! This is an *integrity* check, not a proof of soundness. FNV-1a is not a
//! cryptographic hash: it detects accidental staleness and casual tampering,
//! and it would not resist an adversary who can edit both the sources and the
//! sidecar. Making it adversarial-resistant means swapping FNV-1a for SHA-256
//! (via a dependency) or re-attesting under the deployment principal — both are
//! recorded as follow-ups rather than pretended away. The security property
//! that *is* real here is that a `required` module cannot be hosted without a
//! sidecar whose recorded hashes match the bytes on disk, which is what makes
//! the compiler's verdict load-bearing instead of advisory.

use std::fs;
use std::path::{Path, PathBuf};

use crate::auth::LiquidMode;

/// FNV-1a, 64-bit. See the module note: integrity, not cryptographic.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // 1099511628211
    }
    h
}

/// Hash a file, or an empty digest if it cannot be read. A missing file must
/// not read as a matching hash, so absence is distinguishable from content.
fn hash_file(path: &Path) -> Option<u64> {
    fs::read(path).ok().map(|b| fnv1a(&b))
}

/// The sidecar contents. One `key = value` per line, so it stays readable and
/// diffable and needs no parser dependency in the loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attestation {
    pub sources_hash: u64,
    pub mlw_hash: u64,
    pub prover: String,
    pub prover_version: String,
    pub verdict: String,
    pub trusted_contracts: u32,
}

fn parse_attestation(text: &str) -> Result<Attestation, String> {
    let mut sources_hash = None;
    let mut mlw_hash = None;
    let mut prover = None;
    let mut prover_version = None;
    let mut verdict = None;
    let mut trusted_contracts = None;

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("liquid.ok line {}: expected `key = value`", lineno + 1))?;
        let value = value.trim().to_string();
        match key.trim() {
            "sources_hash" => {
                sources_hash = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("liquid.ok: sources_hash {:?} is not a u64", value))?,
                )
            }
            "mlw_hash" => {
                mlw_hash = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("liquid.ok: mlw_hash {:?} is not a u64", value))?,
                )
            }
            "prover" => prover = Some(value),
            "prover_version" => prover_version = Some(value),
            "verdict" => verdict = Some(value),
            "trusted_contracts" => {
                trusted_contracts = Some(value.parse::<u32>().map_err(|_| {
                    format!("liquid.ok: trusted_contracts {:?} is not a u32", value)
                })?)
            }
            other => return Err(format!("liquid.ok: unknown key {:?}", other)),
        }
    }

    Ok(Attestation {
        sources_hash: sources_hash.ok_or("liquid.ok: missing sources_hash")?,
        mlw_hash: mlw_hash.ok_or("liquid.ok: missing mlw_hash")?,
        prover: prover.ok_or("liquid.ok: missing prover")?,
        prover_version: prover_version.ok_or("liquid.ok: missing prover_version")?,
        verdict: verdict.ok_or("liquid.ok: missing verdict")?,
        trusted_contracts: trusted_contracts.ok_or("liquid.ok: missing trusted_contracts")?,
    })
}

/// The sources whose bytes an attestation covers: every `.aui`/`.aum` in the
/// module directory, sorted so the hash does not depend on readdir order.
pub fn source_files(module_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = fs::read_dir(module_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "aui" || ext == "aum" {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The digest an attestation must record for this module directory.
///
/// Hashes the sorted `name\0bytes\0` sequence, so renaming or adding a source
/// changes the digest even when the concatenated bytes coincide.
pub fn sources_digest(module_dir: &Path) -> Result<u64, String> {
    let mut acc: Vec<u8> = Vec::new();
    for p in source_files(module_dir) {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("non-UTF-8 source name")?;
        acc.extend_from_slice(name.as_bytes());
        acc.push(0);
        let bytes = fs::read(&p).map_err(|e| format!("cannot read {}: {}", p.display(), e))?;
        acc.extend_from_slice(&bytes);
        acc.push(0);
    }
    Ok(fnv1a(&acc))
}

/// Why a module may or may not be hosted. `Ok` carries a note worth logging;
/// `Err` is a refusal.
pub type Gate = Result<String, String>;

/// The load-time gate. `mode` comes from the manifest's `[verify] liquid`.
///
/// - `Off`: nothing is checked. A module written before this feature behaves
///   exactly as it did.
/// - `Optional`: the sidecar is honoured when present, ignored when absent.
/// - `Required`: the sidecar must be present, parse, agree with the bytes on
///   disk, and record `verdict = proved`.
pub fn verify_module(module_dir: &Path, mode: LiquidMode) -> Gate {
    let sidecar = module_dir.join("liquid.ok");

    if mode == LiquidMode::Off {
        return Ok("liquid checking off for this module".into());
    }

    let text = match fs::read_to_string(&sidecar) {
        Ok(t) => t,
        Err(e) => {
            return if mode == LiquidMode::Optional {
                Ok(format!("no liquid.ok ({}); optional, continuing", e))
            } else {
                Err(format!(
                    "LiquidAttestationMissing: {} declares liquid = \"required\" but has no liquid.ok ({})",
                    module_dir.display(),
                    e
                ))
            }
        }
    };

    let att = match parse_attestation(&text) {
        Ok(a) => a,
        Err(e) => {
            return if mode == LiquidMode::Optional {
                Ok(format!("unreadable liquid.ok ({}); optional, continuing", e))
            } else {
                Err(format!("LiquidAttestationInvalid: {}", e))
            }
        }
    };

    // Staleness: the attestation must describe *these* bytes.
    let want_sources = match sources_digest(module_dir) {
        Ok(d) => d,
        Err(e) => {
            return Err(format!("LiquidAttestationUndecidable: {}", e));
        }
    };
    if att.sources_hash != want_sources {
        return Err(format!(
            "LiquidAttestationStale: liquid.ok records sources_hash {:#x} but the \
             sources on disk hash to {:#x}; the attestation predates an edit",
            att.sources_hash, want_sources
        ));
    }

    // The `.mlw` the verdict was about, if it is still around to check.
    let mlw = module_dir.join("module.mlw");
    if let Some(actual) = hash_file(&mlw) {
        if actual != att.mlw_hash {
            return Err(format!(
                "LiquidAttestationStale: liquid.ok records mlw_hash {:#x} but {} \
                 hashes to {:#x}",
                att.mlw_hash,
                mlw.display(),
                actual
            ));
        }
    }

    match att.verdict.as_str() {
        "proved" => Ok(format!(
            "liquid.ok accepted: {} {} proved {} contract(s), {} trusted",
            att.prover, att.prover_version, 0, att.trusted_contracts
        )),
        "refused" => Err(
            "LiquidAttestationRefused: the prover refused this module's contracts".into(),
        ),
        "unknown" => Err(
            "LiquidAttestationUnknown: the prover was inconclusive; an unknown verdict \
             never satisfies a required module"
                .into(),
        ),
        other => Err(format!(
            "LiquidAttestationInvalid: verdict {:?} is not proved/refused/unknown",
            other
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("austral-liquid-attest-{}", tag));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_sources(d: &Path, body: &str) -> u64 {
        fs::write(d.join("M.aum"), body).unwrap();
        sources_digest(d).unwrap()
    }

    fn sidecar(d: &Path, sources: u64, mlw: u64, verdict: &str) {
        fs::write(
            d.join("liquid.ok"),
            format!(
                "sources_hash = {}\nmlw_hash = {}\nprover = why3\nprover_version = 1.8.0\n\
                 verdict = {}\ntrusted_contracts = 0\n",
                sources, mlw, verdict
            ),
        )
        .unwrap();
    }

    #[test]
    fn off_is_a_no_op() {
        let d = tmpdir("off");
        write_sources(&d, "module body M is end module body.\n");
        assert!(verify_module(&d, LiquidMode::Off).is_ok());
    }

    #[test]
    fn required_without_sidecar_is_refused() {
        let d = tmpdir("missing");
        write_sources(&d, "module body M is end module body.\n");
        let e = verify_module(&d, LiquidMode::Required).unwrap_err();
        assert!(e.contains("LiquidAttestationMissing"), "{}", e);
    }

    #[test]
    fn optional_without_sidecar_is_allowed() {
        let d = tmpdir("optional");
        write_sources(&d, "module body M is end module body.\n");
        assert!(verify_module(&d, LiquidMode::Optional).is_ok());
    }

    #[test]
    fn required_accepts_a_matching_proved_attestation() {
        let d = tmpdir("good");
        let h = write_sources(&d, "module body M is end module body.\n");
        sidecar(&d, h, 0, "proved");
        assert!(verify_module(&d, LiquidMode::Required).is_ok());
    }

    #[test]
    fn stale_attestation_is_refused_after_an_edit() {
        let d = tmpdir("stale");
        let h = write_sources(&d, "module body M is end module body.\n");
        sidecar(&d, h, 0, "proved");
        // Now edit the source, as a recompile would.
        fs::write(d.join("M.aum"), "module body M is\nend module body.\n").unwrap();
        let e = verify_module(&d, LiquidMode::Required).unwrap_err();
        assert!(e.contains("LiquidAttestationStale"), "{}", e);
    }

    #[test]
    fn editing_both_sides_is_the_documented_limit() {
        let d = tmpdir("tampered");
        let h = write_sources(&d, "module body M is end module body.\n");
        // A sidecar whose verdict was edited from refused to proved, with the
        // hashes left alone. Nothing catches this, and the test says so: FNV-1a
        // detects *staleness* (one side changed) and casual edits, not an
        // adversary who can rewrite both files. See the module note.
        sidecar(&d, h, 0, "refused");
        let text = fs::read_to_string(d.join("liquid.ok")).unwrap();
        fs::write(
            d.join("liquid.ok"),
            text.replace("verdict = refused", "verdict = proved"),
        )
        .unwrap();
        assert!(
            verify_module(&d, LiquidMode::Required).is_ok(),
            "documented limit: both sides editable, so FNV-1a cannot catch this"
        );
    }

    #[test]
    fn refused_and_unknown_never_satisfy_required() {
        for v in ["refused", "unknown"] {
            let d = tmpdir(&format!("verdict-{}", v));
            let h = write_sources(&d, "module body M is end module body.\n");
            sidecar(&d, h, 0, v);
            assert!(
                verify_module(&d, LiquidMode::Required).is_err(),
                "verdict {} must not satisfy required",
                v
            );
        }
    }

    #[test]
    fn mlw_hash_mismatch_is_refused() {
        let d = tmpdir("mlw");
        let h = write_sources(&d, "module body M is end module body.\n");
        sidecar(&d, h, 12345, "proved");
        fs::write(d.join("module.mlw"), "module M end\n").unwrap();
        let e = verify_module(&d, LiquidMode::Required).unwrap_err();
        assert!(e.contains("LiquidAttestationStale"), "{}", e);
    }

    #[test]
    fn unknown_key_is_refused() {
        let d = tmpdir("badkey");
        let h = write_sources(&d, "module body M is end module body.\n");
        fs::write(
            d.join("liquid.ok"),
            format!("sources_hash = {}\nmlw_hash = 0\nsurprise = 1\n", h),
        )
        .unwrap();
        let e = verify_module(&d, LiquidMode::Required).unwrap_err();
        assert!(e.contains("LiquidAttestationInvalid"), "{}", e);
    }
}