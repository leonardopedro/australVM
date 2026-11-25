//! D18: versioned distributed verification profiles and their evidence gate.
//!
//! `module.toml` may carry a `[verify]` table. The legacy table (L8) only
//! declared `liquid = "off|optional|required"`; D18 adds an optional schema
//! version and the requirements a distributed deployment needs to state:
//! total transition coverage, the protocol and wire-schema identity, the proof
//! tier, and whether the module claims a strict distributed posture.
//!
//! A requirement is not evidence. The gate therefore reads a separate
//! `verification.ok` sidecar produced by the verification run and refuses a
//! required deployment when it is missing, stale, incomplete, or describes a
//! different protocol/schema. Optional mode returns a `[gap]` note rather than
//! silently treating absent evidence as a pass.
//!
//! ## What this is not
//!
//! The sidecar is a claimed result from the tools named in it, not an
//! independently proof-checked certificate. SMT verdicts remain Why3/solver
//! evidence, and a finite transition count covers the cases the producer
//! enumerated. The value here is that a manifest cannot *ask* for a property
//! the host never reads: the fields below are parsed, compared, and refused.

use std::collections::BTreeMap;
use std::path::Path;

/// The profile schema this loader implements. A future schema is refused
/// rather than read with fields silently ignored.
pub const PROFILE_VERSION: u32 = 1;

/// The maximum artifact id length accepted from a sidecar. A cryptographic
/// digest is 64 hex characters; the prefix form is slightly longer.
const MAX_ARTIFACT_ID: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Requirement {
    #[default]
    Off,
    Optional,
    Required,
}

impl Requirement {
    fn parse(field: &str, value: &str) -> Result<Self, String> {
        match value {
            "off" => Ok(Self::Off),
            "optional" => Ok(Self::Optional),
            "required" => Ok(Self::Required),
            other => Err(format!(
                "[verify] {field} must be \"off\", \"optional\" or \"required\", got {other:?}"
            )),
        }
    }

    pub fn is_required(self) -> bool {
        self == Self::Required
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ProofTier {
    #[default]
    None,
    Smt,
    Mm0,
}

impl ProofTier {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "none" => Ok(Self::None),
            "smt" => Ok(Self::Smt),
            "mm0" => Ok(Self::Mm0),
            other => Err(format!(
                "[verify] proof_tier must be \"none\", \"smt\" or \"mm0\", got {other:?}"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Smt => "smt",
            Self::Mm0 => "mm0",
        }
    }
}

/// The `[verify]` requirements a manifest asks the host to enforce.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerificationRequirements {
    /// `None` is the legacy L8 table. Any D18 field requires `version = 1`.
    pub version: Option<u32>,
    pub liquid: Requirement,
    pub totality: Requirement,
    pub protocol: Option<String>,
    pub schema: Option<String>,
    pub proof_tier: Option<ProofTier>,
    pub distributed: Requirement,
}

impl VerificationRequirements {
    /// Parse the `[verify]` table from a full `module.toml` document.
    ///
    /// Unknown keys are refused. A field the loader does not understand must
    /// not look like an enforced requirement.
    pub fn from_toml_str(manifest: &str) -> Result<Self, String> {
        let root: toml::Value =
            toml::from_str(manifest).map_err(|e| format!("TOML parse error: {e}"))?;
        let Some(table) = root.get("verify") else {
            return Ok(Self::default());
        };
        let table = table
            .as_table()
            .ok_or("[verify] must be a table, not a value")?;

        let mut unknown = Vec::new();
        for key in table.keys() {
            match key.as_str() {
                "version" | "liquid" | "totality" | "protocol" | "schema" | "proof_tier"
                | "distributed" | "qualifiers" => {}
                other => unknown.push(other.to_string()),
            }
        }
        if !unknown.is_empty() {
            unknown.sort();
            return Err(format!(
                "[verify] has unknown key(s): {}",
                unknown.join(", ")
            ));
        }

        let version = match table.get("version") {
            None => None,
            Some(value) => {
                let n = value
                    .as_integer()
                    .ok_or("[verify] version must be an integer")?;
                if n != i64::from(PROFILE_VERSION) {
                    return Err(format!(
                        "[verify] version {n} is not supported (this loader implements {PROFILE_VERSION})"
                    ));
                }
                Some(PROFILE_VERSION)
            }
        };

        // These keys belong to the versioned schema. Accepting one without a
        // version would make the table's meaning depend on which loader reads
        // it, so the missing version is an error.
        let versioned_keys = [
            "totality",
            "protocol",
            "schema",
            "proof_tier",
            "distributed",
        ];
        if version.is_none() && versioned_keys.iter().any(|k| table.contains_key(*k)) {
            return Err(format!(
                "[verify] {} requires version = {PROFILE_VERSION}",
                versioned_keys
                    .iter()
                    .find(|k| table.contains_key(**k))
                    .copied()
                    .unwrap_or("this field")
            ));
        }

        let liquid = match table.get("liquid").and_then(|v| v.as_str()) {
            Some(v) => Requirement::parse("liquid", v)?,
            None => Requirement::Off,
        };
        let totality = match table.get("totality").and_then(|v| v.as_str()) {
            Some(v) => Requirement::parse("totality", v)?,
            None => Requirement::Off,
        };
        let distributed = match table.get("distributed").and_then(|v| v.as_str()) {
            Some(v) => Requirement::parse("distributed", v)?,
            None => Requirement::Off,
        };
        let protocol = optional_name(table, "protocol")?;
        let schema = optional_name(table, "schema")?;
        let proof_tier = match table.get("proof_tier") {
            None => None,
            Some(value) => Some(ProofTier::parse(
                value
                    .as_str()
                    .ok_or("[verify] proof_tier must be a string")?,
            )?),
        };

        Ok(Self {
            version,
            liquid,
            totality,
            protocol,
            schema,
            proof_tier,
            distributed,
        })
    }

    /// Does this profile ask for a D18 `verification.ok` sidecar?
    ///
    /// A legacy L8 table (`liquid` only, no version) keeps its existing
    /// `liquid.ok` gate, so adding this loader does not silently change what
    /// an old module must provide.
    pub fn requires_evidence(&self) -> bool {
        self.version.is_some()
            && (self.liquid != Requirement::Off
                || self.totality != Requirement::Off
                || self.distributed != Requirement::Off
                || self.protocol.is_some()
                || self.schema.is_some()
                || self.proof_tier.is_some())
    }

    /// Whether any verdict is required, i.e. an optional request may degrade
    /// to a `[gap]` but a required one may not.
    fn evidence_is_required(&self) -> bool {
        self.liquid.is_required() || self.totality.is_required() || self.distributed.is_required()
    }

    /// Consistency rules that can be checked from the manifest alone.
    /// `profile_strict` is `[module] profile = "strict"`.
    pub fn manifest_errors(&self, profile_strict: bool) -> Vec<String> {
        let mut errors = Vec::new();
        if self.distributed.is_required() {
            if !profile_strict {
                errors.push(
                    "[verify] distributed = \"required\" needs [module] profile = \"strict\""
                        .to_string(),
                );
            }
            if !self.liquid.is_required() {
                errors.push(
                    "[verify] distributed = \"required\" needs liquid = \"required\"".to_string(),
                );
            }
            if !self.totality.is_required() {
                errors.push(
                    "[verify] distributed = \"required\" needs totality = \"required\"".to_string(),
                );
            }
            if self.protocol.is_none() {
                errors.push(
                    "[verify] distributed = \"required\" needs a protocol identity".to_string(),
                );
            }
            if self.schema.is_none() {
                errors.push(
                    "[verify] distributed = \"required\" needs a schema identity".to_string(),
                );
            }
            if !matches!(self.proof_tier, Some(ProofTier::Smt | ProofTier::Mm0)) {
                errors.push(
                    "[verify] distributed = \"required\" needs proof_tier = \"smt\" or \"mm0\""
                        .to_string(),
                );
            }
        }
        if self.totality.is_required() && self.version.is_none() {
            errors.push("[verify] totality requires version = 1".to_string());
        }
        errors
    }

    /// Read and check `verification.ok` against these requirements.
    pub fn check_evidence(
        &self,
        module_dir: &Path,
        profile_strict: bool,
    ) -> Result<String, String> {
        let errors = self.manifest_errors(profile_strict);
        if !errors.is_empty() {
            return Err(format!("VerificationProfileInvalid: {}", errors.join("; ")));
        }
        if !self.requires_evidence() {
            return Ok("verification profile is legacy/off".to_string());
        }

        let path = module_dir.join("verification.ok");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                if self.evidence_is_required() {
                    return Err(format!(
                        "VerificationEvidenceMissing: {} requires verification.ok ({e})",
                        module_dir.display()
                    ));
                }
                return Ok(format!("[gap] no verification.ok ({e})"));
            }
        };
        let evidence = match Evidence::parse(&text) {
            Ok(e) => e,
            Err(e) => {
                if self.distributed.is_required() || self.totality.is_required() {
                    return Err(format!("VerificationEvidenceInvalid: {e}"));
                }
                return Ok(format!("[gap] unreadable verification.ok ({e})"));
            }
        };

        if evidence.version != PROFILE_VERSION {
            let msg = format!(
                "verification.ok version {} does not match required {PROFILE_VERSION}",
                evidence.version
            );
            if self.evidence_is_required() {
                return Err(format!("VerificationEvidenceInvalid: {msg}"));
            }
            return Ok(format!("[gap] {msg}"));
        }

        let mut notes = Vec::new();
        if self.liquid.is_required() {
            if evidence.liquid != "proved" {
                return Err(format!(
                    "VerificationEvidenceIncomplete: liquid is {:?}, required is \"proved\"",
                    evidence.liquid
                ));
            }
            notes.push("liquid=proved".to_string());
        }

        if self.totality.is_required() {
            if evidence.totality != "covered" {
                return Err(format!(
                    "VerificationEvidenceIncomplete: totality is {:?}, required is \"covered\"",
                    evidence.totality
                ));
            }
            if evidence.transitions == 0 {
                return Err(
                    "VerificationEvidenceIncomplete: totality claims coverage with zero transitions"
                        .to_string(),
                );
            }
            if evidence.covered != evidence.transitions {
                return Err(format!(
                    "VerificationEvidenceIncomplete: {}/{} transitions covered",
                    evidence.covered, evidence.transitions
                ));
            }
            notes.push(format!(
                "totality={}/{}",
                evidence.covered, evidence.transitions
            ));
        }

        if let Some(expected) = &self.protocol {
            if evidence.protocol.as_deref() != Some(expected.as_str()) {
                return Err(format!(
                    "VerificationEvidenceMismatch: protocol is {:?}, manifest requires {expected:?}",
                    evidence.protocol
                ));
            }
            notes.push(format!("protocol={expected}"));
        }
        if let Some(expected) = &self.schema {
            if evidence.schema.as_deref() != Some(expected.as_str()) {
                return Err(format!(
                    "VerificationEvidenceMismatch: schema is {:?}, manifest requires {expected:?}",
                    evidence.schema
                ));
            }
            notes.push(format!("schema={expected}"));
        }

        if let Some(required) = self.proof_tier {
            let actual = evidence.proof_tier.unwrap_or(ProofTier::None);
            if actual < required {
                return Err(format!(
                    "VerificationEvidenceIncomplete: proof tier {} is below required {}",
                    actual.as_str(),
                    required.as_str()
                ));
            }
            notes.push(format!("proof_tier={}", actual.as_str()));
        }

        if self.distributed.is_required() {
            let identity = evidence.identity.as_deref().ok_or(
                "VerificationEvidenceIncomplete: no identity for a strict distributed module",
            )?;
            if identity.starts_with("did:unfer:stub:") {
                return Err(format!(
                    "VerificationIdentityStub: {identity:?} is not a distributed trust anchor"
                ));
            }
            let artifact = evidence
                .artifact
                .as_deref()
                .ok_or("VerificationEvidenceIncomplete: no artifact id")?;
            if !cryptographic_artifact_id(artifact) {
                return Err(format!(
                    "VerificationArtifactId: {artifact:?} is not a SHA-256/CID artifact id"
                ));
            }
            match evidence.authorizer.as_deref() {
                Some("allow-all") => {
                    return Err(
                        "VerificationAuthorizer: strict distributed mode refuses AllowAll"
                            .to_string(),
                    )
                }
                Some("manifest") | Some("cedar") => {}
                other => {
                    return Err(format!(
                        "VerificationAuthorizer: {other:?} is not a named authorizer"
                    ))
                }
            }
            match evidence.deployment.as_deref() {
                Some("local-consensus") => {
                    return Err(
                        "VerificationDeployment: strict distributed mode refuses an implicit \
                         LocalConsensus deployment"
                            .to_string(),
                    )
                }
                Some("network") => {}
                other => {
                    return Err(format!(
                        "VerificationDeployment: {other:?} is not an explicit deployment"
                    ))
                }
            }
            notes.push(format!("identity={identity}"));
        }

        Ok(format!("verification.ok accepted ({})", notes.join(", ")))
    }
}

fn optional_name(table: &toml::Table, field: &str) -> Result<Option<String>, String> {
    match table.get(field) {
        None => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| format!("[verify] {field} must be a string"))?
                .trim();
            if text.is_empty() {
                return Err(format!("[verify] {field} must not be empty"));
            }
            if text.contains('\0') {
                return Err(format!("[verify] {field} must not contain NUL"));
            }
            Ok(Some(text.to_string()))
        }
    }
}

/// A SHA-256 digest is 64 lowercase hex characters. A CID may carry the
/// `sha256:` prefix used by `federation::artifact_cid` when the federation
/// feature is not compiled in.
pub fn cryptographic_artifact_id(id: &str) -> bool {
    let body = id.strip_prefix("sha256:").unwrap_or(id);
    id.len() <= MAX_ARTIFACT_ID && body.len() == 64 && body.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Evidence {
    version: u32,
    liquid: String,
    totality: String,
    transitions: u64,
    covered: u64,
    protocol: Option<String>,
    schema: Option<String>,
    proof_tier: Option<ProofTier>,
    identity: Option<String>,
    artifact: Option<String>,
    authorizer: Option<String>,
    deployment: Option<String>,
}

impl Evidence {
    fn parse(text: &str) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        for (lineno, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {}: expected `key = value`", lineno + 1))?;
            values.insert(key.trim().to_string(), value.trim().to_string());
        }
        let get = |key: &str| values.get(key).cloned();
        let required = |key: &str| get(key).ok_or_else(|| format!("missing {key}"));
        let version: u32 = required("version")?
            .parse()
            .map_err(|_| "version is not a u32".to_string())?;
        let transitions: u64 = get("transitions")
            .unwrap_or_else(|| "0".to_string())
            .parse()
            .map_err(|_| "transitions is not a u64".to_string())?;
        let covered: u64 = get("covered")
            .unwrap_or_else(|| "0".to_string())
            .parse()
            .map_err(|_| "covered is not a u64".to_string())?;
        let proof_tier = match get("proof_tier") {
            None => None,
            Some(value) => Some(
                ProofTier::parse(&value)
                    .map_err(|e| e.replace("[verify] proof_tier", "verification.ok proof_tier"))?,
            ),
        };
        for key in [
            "protocol",
            "schema",
            "identity",
            "artifact",
            "authorizer",
            "deployment",
        ] {
            if let Some(value) = get(key) {
                if value.len() > MAX_ARTIFACT_ID {
                    return Err(format!("{key} is longer than {MAX_ARTIFACT_ID} bytes"));
                }
            }
        }
        Ok(Self {
            version,
            liquid: get("liquid").unwrap_or_else(|| "skipped".to_string()),
            totality: get("totality").unwrap_or_else(|| "not-run".to_string()),
            transitions,
            covered,
            protocol: get("protocol"),
            schema: get("schema"),
            proof_tier,
            identity: get("identity"),
            artifact: get("artifact"),
            authorizer: get("authorizer"),
            deployment: get("deployment"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("austral-verification-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn strict_manifest(extra: &str) -> String {
        format!(
            r#"
[module]
name = "M"
profile = "strict"

[verify]
version = 1
liquid = "required"
totality = "required"
protocol = "unfer-consensus/v1"
schema = "distributed-request/v1"
proof_tier = "smt"
distributed = "required"
{extra}
"#
        )
    }

    fn evidence(overrides: &[(&str, &str)]) -> String {
        let mut fields = BTreeMap::from([
            ("version", "1"),
            ("liquid", "proved"),
            ("totality", "covered"),
            ("transitions", "9"),
            ("covered", "9"),
            ("protocol", "unfer-consensus/v1"),
            ("schema", "distributed-request/v1"),
            ("proof_tier", "smt"),
            (
                "identity",
                "did:unfer:test:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            ),
            (
                "artifact",
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            ),
            ("authorizer", "manifest"),
            ("deployment", "network"),
        ]);
        for (k, v) in overrides {
            fields.insert(*k, *v);
        }
        fields
            .iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn write_evidence(dir: &Path, body: &str) {
        std::fs::write(dir.join("verification.ok"), body).unwrap();
    }

    #[test]
    fn legacy_liquid_only_keeps_its_old_gate() {
        let req = VerificationRequirements::from_toml_str(
            r#"
[module]
name = "M"
[verify]
liquid = "required"
"#,
        )
        .unwrap();
        assert!(!req.requires_evidence());
        assert!(req.manifest_errors(false).is_empty());
    }

    #[test]
    fn versioned_fields_require_a_version() {
        let e = VerificationRequirements::from_toml_str(
            r#"
[module]
name = "M"
[verify]
totality = "required"
"#,
        )
        .unwrap_err();
        assert!(e.contains("requires version"), "{e}");
    }

    #[test]
    fn unknown_verify_keys_are_refused() {
        let e =
            VerificationRequirements::from_toml_str(&strict_manifest("not_a_requirement = \"x\""))
                .unwrap_err();
        assert!(e.contains("unknown key"), "{e}");
    }

    #[test]
    fn distributed_requires_strict_liquid_totality_and_identity() {
        let req = VerificationRequirements::from_toml_str(
            r#"
[module]
name = "M"
[verify]
version = 1
distributed = "required"
"#,
        )
        .unwrap();
        let errors = req.manifest_errors(false);
        assert!(
            errors.iter().any(|e| e.contains("profile = \"strict\"")),
            "{errors:?}"
        );
        assert!(errors.iter().any(|e| e.contains("liquid")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("totality")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("protocol")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("schema")), "{errors:?}");
        assert!(
            errors.iter().any(|e| e.contains("proof_tier")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_complete_strict_profile_accepts_complete_evidence() {
        let req = VerificationRequirements::from_toml_str(&strict_manifest("")).unwrap();
        assert!(req.manifest_errors(true).is_empty());
        let d = tmpdir("accept");
        write_evidence(&d, &evidence(&[]));
        let note = req.check_evidence(&d, true).unwrap();
        assert!(note.contains("9/9"), "{note}");
    }

    #[test]
    fn missing_evidence_fails_required_and_gaps_optional() {
        let d = tmpdir("missing");
        let required = VerificationRequirements::from_toml_str(&strict_manifest("")).unwrap();
        let e = required.check_evidence(&d, true).unwrap_err();
        assert!(e.contains("VerificationEvidenceMissing"), "{e}");
        let optional = VerificationRequirements::from_toml_str(
            r#"
[module]
name = "M"
[verify]
version = 1
liquid = "optional"
"#,
        )
        .unwrap();
        let note = optional.check_evidence(&d, false).unwrap();
        assert!(note.starts_with("[gap]"), "{note}");
    }

    #[test]
    fn an_incomplete_transition_cover_is_refused() {
        let req = VerificationRequirements::from_toml_str(&strict_manifest("")).unwrap();
        let d = tmpdir("incomplete");
        write_evidence(&d, &evidence(&[("covered", "8")]));
        let e = req.check_evidence(&d, true).unwrap_err();
        assert!(e.contains("8/9"), "{e}");
    }

    #[test]
    fn stale_protocol_schema_and_proof_tier_are_refused() {
        let req = VerificationRequirements::from_toml_str(&strict_manifest("")).unwrap();
        for (key, value) in [
            ("protocol", "unfer-consensus/v2"),
            ("schema", "distributed-request/v2"),
            ("proof_tier", "none"),
        ] {
            let d = tmpdir(&format!("mismatch-{key}"));
            write_evidence(&d, &evidence(&[(key, value)]));
            let e = req.check_evidence(&d, true).unwrap_err();
            assert!(
                e.contains("Mismatch") || e.contains("Incomplete"),
                "{key}: {e}"
            );
        }
    }

    #[test]
    fn strict_distributed_refuses_stub_ids_allow_all_and_local_consensus() {
        let req = VerificationRequirements::from_toml_str(&strict_manifest("")).unwrap();
        for (key, value) in [
            ("identity", "did:unfer:stub:M"),
            ("artifact", "0123456789abcdef"),
            ("authorizer", "allow-all"),
            ("deployment", "local-consensus"),
        ] {
            let d = tmpdir(&format!("strict-{key}"));
            write_evidence(&d, &evidence(&[(key, value)]));
            let e = req.check_evidence(&d, true).unwrap_err();
            assert!(
                e.contains("Stub")
                    || e.contains("ArtifactId")
                    || e.contains("Authorizer")
                    || e.contains("Deployment"),
                "{key}: {e}"
            );
        }
    }

    #[test]
    fn cryptographic_ids_are_sha256_sized() {
        assert!(cryptographic_artifact_id(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!cryptographic_artifact_id("0123456789abcdef"));
        assert!(!cryptographic_artifact_id(
            "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"
        ));
    }
}
