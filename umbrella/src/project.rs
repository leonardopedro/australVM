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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub name: String,
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPermission {
    #[serde(default)]
    pub allowed: bool,
    /// Crate whose native source may be compiled. Required when `allowed`.
    /// `crate` is a Rust keyword, so the document field is renamed.
    #[serde(default, rename = "crate")]
    pub krate: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadPermission {
    #[serde(default)]
    pub allowed: bool,
    /// cdylibs that may be mapped into the process.
    #[serde(default)]
    pub providers: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// One extern symbol the module may invoke (`rs_*`, `uk_*`, `uz_*`).
    pub symbol: String,
}

#[derive(Debug, Deserialize)]
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
        &["schema", "profile", "module", "build", "load", "grants"],
        &mut unknown,
    );
    reject_unknown_in(&value, "module", &["name", "sources"], &mut unknown);
    reject_unknown_in(&value, "build", &["allowed", "crate"], &mut unknown);
    reject_unknown_in(&value, "load", &["allowed", "providers"], &mut unknown);
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
        "grants": [{"symbol": "rs_demo_checksum"}]
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
}
