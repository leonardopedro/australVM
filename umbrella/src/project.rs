//! Versioned project schema (PLAN_umbrella_production U02).
//!
//! The prototype used to take its grant file and module list as loose CLI
//! flags: nothing validated them before `cargo build` ran or a cdylib was
//! dlopen'd. This module is the strict, versioned description the plan asks
//! for: one document that names the deployment profile, the modules, the
//! build/load authorizations and the call grants — and refuses unknown
//! fields, an unsupported schema version or a malformed grant by name.
//!
//! Strictness is the point: an unknown field is not ignored, because a
//! misspelled `"build": {"allowedd": true}` would otherwise read as "not
//! authorized" and silently turn into a different deployment.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The only schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Deployment profile. `untrusted-native` is a reserved name whose rules are
/// U26's work, so it is recognized here and refused by preflight rather than
/// accepted as if it were implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    Dev,
    TrustedNative,
    UntrustedNative,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Dev => "dev",
            Profile::TrustedNative => "trusted-native",
            Profile::UntrustedNative => "untrusted-native",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub name: String,
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPermission {
    #[serde(default)]
    pub allowed: bool,
    /// Crate whose native source may be compiled. Required when `allowed`.
    /// `crate` is a Rust keyword, so the document field is renamed.
    #[serde(default, rename = "crate")]
    pub krate: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadPermission {
    #[serde(default)]
    pub allowed: bool,
    /// cdylibs that may be mapped into the process.
    #[serde(default)]
    pub providers: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// One extern symbol the module may invoke (`rs_*`, `uk_*`, `uz_*`).
    pub symbol: String,
}

/// D09: the artifact identity a build/load authorization is pinned to. Building
/// a crate runs its build scripts and mapping a cdylib runs its initializer, so
/// the document names the exact dependency graph, feature set, library bytes and
/// ABI exports that were approved — not merely "a crate" and "a provider".
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedArtifact {
    /// SHA-256 (hex) of the crate's `Cargo.lock`, checked before `cargo build`.
    pub lock_sha256: String,
    /// The exact feature set to build with; an extra one is a different build.
    #[serde(default)]
    pub features: Vec<String>,
    /// SHA-256 (hex) of the cdylib, checked after the build and before `dlopen`.
    pub library_sha256: String,
    /// Extern symbols the library must define, checked before `dlopen`.
    #[serde(default)]
    pub exports: Vec<String>,
}

/// D09: the capabilities the running instance gets. Stated in the document so a
/// run names its own state directory and peers explicitly instead of inheriting
/// an ambient root, and so a missing block means "no capabilities" rather than
/// "whatever the host can reach".
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// Directory the instance's directory capability is rooted at. A relative
    /// path is resolved against the project document, never against the
    /// developer's shell.
    pub state_dir: PathBuf,
    /// Exactly the peers the instance may address. Empty means none.
    #[serde(default)]
    pub peers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub schema: u32,
    pub profile: Profile,
    pub module: Module,
    #[serde(default)]
    pub build: BuildPermission,
    #[serde(default)]
    pub load: LoadPermission,
    #[serde(default)]
    pub grants: Vec<Grant>,
    /// D09: required whenever the project authorizes a build or a load.
    #[serde(default)]
    pub trusted: Option<TrustedArtifact>,
    /// D09: the instance's capabilities. Absent means the run has none, and every
    /// `rs_cap_*` verb refuses.
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
    /// The features the caller asked cargo for. Not a document field: the
    /// command line supplies it, and the trust gate compares it with the pin.
    #[serde(skip)]
    pub requested_features: Vec<String>,
}

/// Parse and validate a project document. Errors are diagnostics, not a
/// single opaque message: every problem is prefixed with the document path
/// and the field it belongs to.
pub fn load(path: &Path) -> Result<Project, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("project {}: {e}", path.display()))?;
    parse(path, &text)
}

pub fn parse(path: &Path, text: &str) -> Result<Project, String> {
    // serde's `deny_unknown_fields` names the field but not the section it
    // was found in; walk the document first so the diagnostic reads
    // `build.allowedd`, not just `allowedd`.
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("project {}: {e}", path.display()))?;
    let mut unknown = Vec::new();
    reject_unknown(
        &value,
        "",
        &[
            "schema",
            "profile",
            "module",
            "build",
            "load",
            "grants",
            "trusted",
            "capabilities",
        ],
        &mut unknown,
    );
    reject_unknown_in(&value, "module", &["name", "sources"], &mut unknown);
    reject_unknown_in(&value, "build", &["allowed", "crate"], &mut unknown);
    reject_unknown_in(&value, "load", &["allowed", "providers"], &mut unknown);
    reject_unknown_in(&value, "capabilities", &["state_dir", "peers"], &mut unknown);
    reject_unknown_in(
        &value,
        "trusted",
        &["lock_sha256", "features", "library_sha256", "exports"],
        &mut unknown,
    );
    if let Some(grants) = value.get("grants").and_then(|g| g.as_array()) {
        for (i, g) in grants.iter().enumerate() {
            reject_unknown(g, &format!("grants[{i}]."), &["symbol"], &mut unknown);
        }
    }
    if !unknown.is_empty() {
        return Err(unknown
            .into_iter()
            .map(|m| format!("project {}: {m}", path.display()))
            .collect::<Vec<_>>()
            .join("; "));
    }

    let project: Project =
        serde_json::from_str(text).map_err(|e| format!("project {}: {e}", path.display()))?;
    validate(path, &project)?;
    Ok(project)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn reject_unknown(
    value: &serde_json::Value,
    prefix: &str,
    allowed: &[&str],
    problems: &mut Vec<String>,
) {
    let Some(obj) = value.as_object() else { return };
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            problems.push(format!("unknown field `{prefix}{key}`"));
        }
    }
}

fn reject_unknown_in(
    value: &serde_json::Value,
    section: &str,
    allowed: &[&str],
    problems: &mut Vec<String>,
) {
    if let Some(inner) = value.get(section) {
        reject_unknown(inner, &format!("{section}."), allowed, problems);
    }
}

/// Rules that a syntactically valid document can still break.
fn validate(path: &Path, p: &Project) -> Result<(), String> {
    let at = |msg: String| format!("project {}: {msg}", path.display());
    let mut problems = Vec::new();

    if p.schema != SCHEMA_VERSION {
        problems.push(format!(
            "unsupported `schema`: {} (this umbrella understands {})",
            p.schema, SCHEMA_VERSION
        ));
    }
    if p.module.name.trim().is_empty() {
        problems.push("`module.name` is empty".to_string());
    }
    if p.module.sources.is_empty() {
        problems.push("`module.sources` is empty".to_string());
    }
    if p.build.allowed && p.build.krate.is_none() {
        problems.push("`build.allowed` is true but `build.crate` is absent".to_string());
    }
    if p.load.allowed && p.load.providers.is_empty() {
        problems.push("`load.allowed` is true but `load.providers` is empty".to_string());
    }
    // D09: an authorization to build or load without a pinned artifact identity
    // would be an authorization to run whatever the tree happens to contain.
    if (p.build.allowed || p.load.allowed) && p.trusted.is_none() {
        problems.push(
            "`build`/`load` is authorized but `trusted` is absent: the artifact identity \
             (lock_sha256, library_sha256) must be pinned before it can run"
                .to_string(),
        );
    }
    if let Some(caps) = &p.capabilities {
        if caps.state_dir.as_os_str().is_empty() {
            problems.push("`capabilities.state_dir` is empty".to_string());
        }
        for (i, peer) in caps.peers.iter().enumerate() {
            if peer.trim().is_empty() {
                problems.push(format!("`capabilities.peers[{i}]` is empty"));
            }
        }
    }
    if let Some(t) = &p.trusted {
        for (field, value) in [
            ("trusted.lock_sha256", &t.lock_sha256),
            ("trusted.library_sha256", &t.library_sha256),
        ] {
            if !is_sha256_hex(value) {
                problems.push(format!("`{field}` must be 64 hex digits, got {value:?}"));
            }
        }
        if t.exports.is_empty() {
            problems.push("`trusted.exports` is empty: pin the ABI the loader expects".to_string());
        }
        for (i, name) in t.exports.iter().enumerate() {
            if name.trim().is_empty() {
                problems.push(format!("`trusted.exports[{i}]` is empty"));
            }
        }
    }
    for (i, g) in p.grants.iter().enumerate() {
        let sym = g.symbol.trim();
        if sym.is_empty() {
            problems.push(format!("grant {i}: `symbol` is empty"));
        } else if !(sym.starts_with("rs_") || sym.starts_with("uk_") || sym.starts_with("uz_")) {
            problems.push(format!(
                "grant {i}: `{}` is not an extern symbol (expected an rs_/uk_/uz_ prefix)",
                sym
            ));
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.into_iter().map(at).collect::<Vec<_>>().join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"{
        "schema": 1,
        "profile": "trusted-native",
        "module": {"name": "demo", "sources": ["Demo.aum"]},
        "build": {"allowed": true, "crate": "modules/rscrate"},
        "load": {"allowed": true, "providers": ["libdemo.so"]},
        "grants": [{"symbol": "rs_demo_checksum"}],
        "trusted": {
            "lock_sha256": "0000000000000000000000000000000000000000000000000000000000000001",
            "features": [],
            "library_sha256": "0000000000000000000000000000000000000000000000000000000000000002",
            "exports": ["rs_demo_checksum"]
        }
    }"#;

    fn p() -> &'static Path {
        Path::new("example.json")
    }

    #[test]
    fn a_complete_project_parses() {
        let doc = parse(p(), BASE).expect("valid project");
        assert_eq!(doc.schema, 1);
        assert_eq!(doc.profile, Profile::TrustedNative);
        assert!(doc.build.allowed);
        assert_eq!(doc.grants.len(), 1);
    }

    #[test]
    fn an_unknown_field_is_refused_by_name() {
        let err = parse(
            p(),
            &BASE.replace("\"allowed\": true", "\"allowedd\": true"),
        )
        .expect_err("unknown field must fail");
        assert!(err.contains("build.allowedd"), "{err}");
    }

    #[test]
    fn an_unsupported_schema_version_is_named() {
        let err = parse(p(), &BASE.replace("\"schema\": 1", "\"schema\": 2"))
            .expect_err("schema 2 must fail");
        assert!(err.contains("unsupported `schema`: 2"), "{err}");
    }

    #[test]
    fn a_malformed_grant_is_named() {
        let err = parse(
            p(),
            &BASE.replace("\"rs_demo_checksum\"", "\"cargo_build\""),
        )
        .expect_err("non-extern grant must fail");
        assert!(err.contains("grant 0"), "{err}");
        assert!(err.contains("cargo_build"), "{err}");
    }

    #[test]
    fn build_without_a_crate_is_refused() {
        let err = parse(
            p(),
            r#"{"schema":1,"profile":"trusted-native",
                "module":{"name":"d","sources":["D.aum"]},
                "build":{"allowed":true},"load":{"allowed":false}}"#,
        )
        .expect_err("build needs a crate");
        assert!(err.contains("`build.crate` is absent"), "{err}");
    }

    /// D09: the pin is what a build authorization is *for*, so it is not optional
    /// and it is not free-form: a wrong-length digest or an empty export list is
    /// a document error, not a weaker trust decision.
    #[test]
    fn a_build_without_a_pinned_artifact_is_refused() {
        let no_pin = r#"{"schema":1,"profile":"trusted-native",
            "module":{"name":"d","sources":["D.aum"]},
            "build":{"allowed":true,"crate":"modules/rscrate"},
            "load":{"allowed":false}}"#;
        let err = parse(p(), no_pin).expect_err("no pin must be refused");
        assert!(err.contains("`trusted` is absent"), "{err}");

        let short_digest = BASE.replace(
            "0000000000000000000000000000000000000000000000000000000000000001",
            "abc123",
        );
        let err = parse(p(), &short_digest).expect_err("a short digest must be refused");
        assert!(err.contains("trusted.lock_sha256"), "{err}");

        let no_exports = BASE.replace("\"exports\": [\"rs_demo_checksum\"]", "\"exports\": []");
        let err = parse(p(), &no_exports).expect_err("an empty export list must be refused");
        assert!(err.contains("`trusted.exports` is empty"), "{err}");
    }

    #[test]
    fn an_unknown_trusted_field_is_named() {
        let err = parse(p(), &BASE.replace("\"features\": []", "\"feature\": []"))
            .expect_err("unknown trusted field must be refused");
        assert!(err.contains("trusted.feature"), "{err}");
    }
}
