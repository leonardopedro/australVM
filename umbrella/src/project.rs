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

use austral_cranelift_bridge::capstd_mod::caps::{FsRights, RootGrant};
use austral_cranelift_bridge::netstd_mod::{Endpoint, IpNet, NetPolicy};
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

/// One typed module export the caller is willing to admit. The signature is the
/// CPS wire contract, so a name-only pin cannot accept a changed arity or type.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedSignature {
    pub name: String,
    #[serde(default)]
    pub params: Vec<String>,
    pub returns: String,
}

/// D09/U10: the artifact identity a build/load authorization is pinned to.
/// Building a crate runs its build scripts and mapping a cdylib runs its
/// initializer, so the document names the exact dependency graph, feature set,
/// library bytes and ABI exports that were approved — not merely "a crate" and
/// "a provider". U10 adds the compiler-emitted adapter contract and the module's
/// typed export signatures, both checked before the contract is used to build
/// native wrappers.
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
    /// Typed module exports, checked against the compile-only artifact before
    /// any adapter is built or the JIT is invoked. Empty means no signature pin.
    #[serde(default)]
    pub signatures: Vec<TrustedSignature>,
    /// SHA-256 (hex) of the compiler-emitted adapter document, checked before
    /// native wrappers are generated from it. Absent means no descriptor pin.
    #[serde(default)]
    pub descriptor_sha256: Option<String>,
}

/// U19: one approved subtree with the rights its handles carry. `path` is
/// relative to `capabilities.state_dir`, so a grant cannot name a root outside
/// the instance directory.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRoot {
    pub path: PathBuf,
    /// Comma-separated rights (`read`, `write`, `create`, `list`, `delete`,
    /// `rename`, or `all`). Required: a root without stated rights would be an
    /// implicit all-rights grant.
    pub rights: String,
}

/// D09/U19: the capabilities the running instance gets. Stated in the document
/// so a run names its own state directory, roots, rights and peers explicitly
/// instead of inheriting an ambient root, and so a missing block means "no
/// capabilities" rather than "whatever the host can reach".
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// Directory the instance's directory capability is rooted at. A relative
    /// path is resolved against the project document, never against the
    /// developer's shell.
    pub state_dir: PathBuf,
    /// U19: rights for a handle on the state directory itself. Absent keeps the
    /// historical all-rights grant.
    #[serde(default)]
    pub rights: Option<String>,
    /// U19: additional approved roots, each opened only by index and carrying
    /// exactly its own rights.
    #[serde(default)]
    pub roots: Vec<CapabilityRoot>,
    /// U19: cumulative bytes the instance may write through its capabilities.
    /// Absent means no quota.
    #[serde(default)]
    pub quota_bytes: Option<u64>,
    /// Exactly the peers the instance may address. Empty means none.
    #[serde(default)]
    pub peers: Vec<String>,
    /// U20: the network capability. Absent means no network authority at all;
    /// a `Net` handle is never issued.
    #[serde(default)]
    pub network: Option<NetworkCapability>,
}

/// U20: the endpoints, resolved-address ranges, trust anchor and deadline one
/// project instance may dial. The project document is host-authored, so this is
/// the same grant the manifest spells as `[grants] net`/`net_ips`/`net_ca`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkCapability {
    /// Approved `host:port` endpoints (a bare host means port 443).
    pub endpoints: Vec<String>,
    /// Address ranges every resolved address must fall inside (CIDR or a
    /// single address).
    pub allow_ips: Vec<String>,
    /// PEM or DER trust-anchor bundle, relative to the project document. Absent
    /// means TLS is refused rather than downgraded.
    #[serde(default)]
    pub ca: Option<PathBuf>,
    /// Connect and per-IO deadline.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl Capabilities {
    /// The rights a handle on the state directory carries.
    pub fn state_rights(&self) -> Result<FsRights, String> {
        match &self.rights {
            None => Ok(FsRights::ALL),
            Some(text) => FsRights::parse(text),
        }
    }

    /// U20: build the network policy the instance dials under. `base` is the
    /// project document's directory, which a relative `ca` resolves against. A
    /// document with no `network` block grants nothing and yields no policy.
    pub fn net_policy(&self, base: &Path) -> Result<Option<NetPolicy>, String> {
        let Some(net) = &self.network else {
            return Ok(None);
        };
        let anchors = match &net.ca {
            Some(path) => {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    base.join(path)
                };
                let bytes = std::fs::read(&path).map_err(|e| {
                    format!("capabilities.network.ca {}: {e}", path.display())
                })?;
                austral_cranelift_bridge::netstd_mod::certs_from_pem(&bytes)
                    .map_err(|e| format!("capabilities.network.ca: {e}"))?
            }
            None => Vec::new(),
        };
        NetPolicy::new(
            &net.endpoints,
            &net.allow_ips,
            anchors,
            std::time::Duration::from_millis(net.timeout_ms.unwrap_or(5000)),
        )
        .map(Some)
        .map_err(|e| format!("capabilities.network: {e}"))
    }

    /// The extra roots, parsed into the form the capability set opens.
    pub fn root_grants(&self) -> Result<Vec<RootGrant>, String> {
        let mut grants = Vec::new();
        for (index, root) in self.roots.iter().enumerate() {
            let path = root.path.to_str().ok_or_else(|| {
                format!("capabilities.roots[{index}].path is not valid UTF-8")
            })?;
            let rights = FsRights::parse(&root.rights)
                .map_err(|e| format!("capabilities.roots[{index}].rights: {e}"))?;
            grants.push(RootGrant {
                root: path.to_string(),
                rights,
            });
        }
        Ok(grants)
    }
}

/// U09: the user on whose behalf the instance acts. The module stays the grant
/// principal (`[module] name`); this names the delegator in the instance
/// identity and its diagnostics. Absent means no user was supplied.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub schema: u32,
    pub profile: Profile,
    pub module: Module,
    /// U09: the delegating user, when the host has one.
    #[serde(default)]
    pub identity: Option<Identity>,
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
            "identity",
        ],
        &mut unknown,
    );
    reject_unknown_in(&value, "module", &["name", "sources"], &mut unknown);
    reject_unknown_in(&value, "identity", &["user"], &mut unknown);
    reject_unknown_in(&value, "build", &["allowed", "crate"], &mut unknown);
    reject_unknown_in(&value, "load", &["allowed", "providers"], &mut unknown);
    reject_unknown_in(
        &value,
        "capabilities",
        &[
            "state_dir",
            "rights",
            "roots",
            "quota_bytes",
            "peers",
            "network",
        ],
        &mut unknown,
    );
    if let Some(network) = value.get("capabilities").and_then(|c| c.get("network")) {
        reject_unknown(
            network,
            "capabilities.network.",
            &["endpoints", "allow_ips", "ca", "timeout_ms"],
            &mut unknown,
        );
    }
    if let Some(roots) = value
        .get("capabilities")
        .and_then(|c| c.get("roots"))
        .and_then(|r| r.as_array())
    {
        for (i, root) in roots.iter().enumerate() {
            reject_unknown(
                root,
                &format!("capabilities.roots[{i}]."),
                &["path", "rights"],
                &mut unknown,
            );
        }
    }
    reject_unknown_in(
        &value,
        "trusted",
        &[
            "lock_sha256",
            "features",
            "library_sha256",
            "exports",
            "signatures",
            "descriptor_sha256",
        ],
        &mut unknown,
    );
    if let Some(signatures) = value
        .get("trusted")
        .and_then(|t| t.get("signatures"))
        .and_then(|s| s.as_array())
    {
        for (i, signature) in signatures.iter().enumerate() {
            reject_unknown(
                signature,
                &format!("trusted.signatures[{i}]."),
                &["name", "params", "returns"],
                &mut unknown,
            );
        }
    }
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
    if p.identity
        .as_ref()
        .is_some_and(|identity| identity.user.trim().is_empty())
    {
        problems.push("`identity.user` is empty".to_string());
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
        if let Err(e) = caps.state_rights() {
            problems.push(format!("`capabilities.rights`: {e}"));
        }
        for (i, root) in caps.roots.iter().enumerate() {
            if root.path.as_os_str().is_empty() {
                problems.push(format!("`capabilities.roots[{i}].path` is empty"));
            } else if root.path.is_absolute() {
                problems.push(format!(
                    "`capabilities.roots[{i}].path` is absolute; it must be relative to \
                     `capabilities.state_dir`"
                ));
            } else if root
                .path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                problems.push(format!(
                    "`capabilities.roots[{i}].path` contains `..`; it must stay inside \
                     `capabilities.state_dir`"
                ));
            }
            match FsRights::parse(&root.rights) {
                Err(e) => problems.push(format!("`capabilities.roots[{i}].rights`: {e}")),
                Ok(FsRights::NONE) => problems.push(format!(
                    "`capabilities.roots[{i}].rights` names no right; a root without a \
                     right is not a capability"
                )),
                Ok(_) => {}
            }
        }
        for (i, peer) in caps.peers.iter().enumerate() {
            if peer.trim().is_empty() {
                problems.push(format!("`capabilities.peers[{i}]` is empty"));
            }
        }
        // U20: a network block is a grant, so it must name something to reach
        // and an address range to reach it through. A half-written block is
        // refused by field rather than silently becoming an empty policy.
        if let Some(net) = &caps.network {
            if net.endpoints.is_empty() {
                problems.push(
                    "`capabilities.network.endpoints` is empty; omit `network` instead of \
                     granting nothing"
                        .to_string(),
                );
            }
            for (i, endpoint) in net.endpoints.iter().enumerate() {
                if let Err(e) = Endpoint::parse(endpoint) {
                    problems.push(format!("`capabilities.network.endpoints[{i}]`: {e}"));
                }
            }
            if net.allow_ips.is_empty() {
                problems.push(
                    "`capabilities.network.allow_ips` is empty; every dial would be refused"
                        .to_string(),
                );
            }
            for (i, range) in net.allow_ips.iter().enumerate() {
                if let Err(e) = IpNet::parse(range) {
                    problems.push(format!("`capabilities.network.allow_ips[{i}]`: {e}"));
                }
            }
            if net.timeout_ms == Some(0) {
                problems.push(
                    "`capabilities.network.timeout_ms` is 0; every dial would time out"
                        .to_string(),
                );
            }
            if net
                .ca
                .as_ref()
                .is_some_and(|path| path.as_os_str().is_empty())
            {
                problems.push("`capabilities.network.ca` is empty".to_string());
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
        if let Some(digest) = &t.descriptor_sha256 {
            if !is_sha256_hex(digest) {
                problems.push(format!(
                    "`trusted.descriptor_sha256` must be 64 hex digits, got {digest:?}"
                ));
            }
        }
        let mut names = std::collections::HashSet::new();
        for (i, signature) in t.signatures.iter().enumerate() {
            if signature.name.trim().is_empty() {
                problems.push(format!("`trusted.signatures[{i}].name` is empty"));
            } else if !names.insert(signature.name.as_str()) {
                problems.push(format!(
                    "`trusted.signatures[{i}].name` repeats `{}`; one exact signature per export",
                    signature.name
                ));
            }
            if signature.returns.trim().is_empty() {
                problems.push(format!("`trusted.signatures[{i}].returns` is empty"));
            }
            for (j, param) in signature.params.iter().enumerate() {
                if param.trim().is_empty() {
                    problems.push(format!("`trusted.signatures[{i}].params[{j}]` is empty"));
                }
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
        "identity": {"user": "did:unfer:alice"},
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
        assert_eq!(
            doc.identity.as_ref().map(|i| i.user.as_str()),
            Some("did:unfer:alice")
        );
    }

    #[test]
    fn an_empty_user_identity_is_named() {
        let err = parse(p(), &BASE.replace("did:unfer:alice", "   "))
            .expect_err("an empty delegator is not an identity");
        assert!(err.contains("`identity.user` is empty"), "{err}");
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

    #[test]
    fn typed_signatures_and_a_descriptor_digest_parse() {
        let doc = BASE.replace(
            "\"exports\": [\"rs_demo_checksum\"]",
            "\"exports\": [\"rs_demo_checksum\"],\n             \"signatures\": [{\"name\": \"main\", \"params\": [], \"returns\": \"Int64\"}],\n             \"descriptor_sha256\": \"0000000000000000000000000000000000000000000000000000000000000003\"",
        );
        let parsed = parse(p(), &doc).expect("valid typed trust record");
        let trusted = parsed.trusted.expect("trusted block");
        assert_eq!(trusted.signatures.len(), 1);
        assert_eq!(trusted.signatures[0].name, "main");
        assert_eq!(trusted.signatures[0].returns, "Int64");
        assert_eq!(
            trusted.descriptor_sha256.as_deref(),
            Some("0000000000000000000000000000000000000000000000000000000000000003")
        );
    }

    #[test]
    fn a_malformed_descriptor_digest_or_repeated_signature_is_named() {
        let short = BASE.replace(
            "\"exports\": [\"rs_demo_checksum\"]",
            "\"exports\": [\"rs_demo_checksum\"], \"descriptor_sha256\": \"abc\"",
        );
        let err = parse(p(), &short).expect_err("short digest");
        assert!(err.contains("trusted.descriptor_sha256"), "{err}");

        let repeated = BASE.replace(
            "\"exports\": [\"rs_demo_checksum\"]",
            "\"exports\": [\"rs_demo_checksum\"], \"signatures\": [\n             {\"name\": \"main\", \"params\": [], \"returns\": \"Int64\"},\n             {\"name\": \"main\", \"params\": [], \"returns\": \"Int32\"}]"
        );
        let err = parse(p(), &repeated).expect_err("repeated signature");
        assert!(err.contains("repeats `main`"), "{err}");
    }

    const WITH_CAPS: &str = r#"{
        "schema": 1,
        "profile": "trusted-native",
        "module": {"name": "demo", "sources": ["Demo.aum"]},
        "capabilities": {
            "state_dir": ".state",
            "rights": "read,list",
            "roots": [
                {"path": "data", "rights": "read"},
                {"path": "cache", "rights": "read,write,create"}
            ],
            "quota_bytes": 4096
        }
    }"#;

    /// U19: the document names each root's rights and the write quota, and the
    /// parsed values are exactly what the host opens handles with.
    #[test]
    fn capability_rights_roots_and_quota_parse() {
        let doc = parse(p(), WITH_CAPS).expect("right-scoped capabilities");
        let caps = doc.capabilities.expect("capabilities block");
        assert_eq!(
            caps.state_rights().unwrap(),
            FsRights::READ.union(FsRights::LIST)
        );
        assert_eq!(caps.quota_bytes, Some(4096));
        let roots = caps.root_grants().unwrap();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[1].root, "cache");
        assert_eq!(
            roots[1].rights,
            FsRights::READ
                .union(FsRights::WRITE)
                .union(FsRights::CREATE)
        );
    }

    #[test]
    fn a_malformed_capability_is_named() {
        for (document, needle) in [
            (
                WITH_CAPS.replace("\"rights\": \"read\"", "\"rghts\": \"read\""),
                "capabilities.roots[0].rghts",
            ),
            (
                WITH_CAPS.replace("\"path\": \"data\"", "\"path\": \"/data\""),
                "is absolute",
            ),
            (
                WITH_CAPS.replace("\"path\": \"data\"", "\"path\": \"../data\""),
                "contains `..`",
            ),
            (
                WITH_CAPS.replace(
                    "\"rights\": \"read,write,create\"",
                    "\"rights\": \"reed\"",
                ),
                "unknown filesystem right",
            ),
            (
                WITH_CAPS.replace(
                    "\"rights\": \"read,write,create\"",
                    "\"rights\": \"\"",
                ),
                "names no right",
            ),
            (
                WITH_CAPS.replace("\"rights\": \"read,list\"", "\"rights\": \"reed\""),
                "`capabilities.rights`",
            ),
        ] {
            let error = parse(p(), &document).expect_err("malformed capability must be refused");
            assert!(error.contains(needle), "{error}");
        }
    }

    #[test]
    fn an_unknown_signature_field_is_named_in_place() {
        let doc = BASE.replace(
            "\"exports\": [\"rs_demo_checksum\"]",
            "\"exports\": [\"rs_demo_checksum\"], \"signatures\": [\n             {\"name\": \"main\", \"params\": [], \"ret\": \"Int64\"}]"
        );
        let err = parse(p(), &doc).expect_err("unknown signature field");
        assert!(err.contains("trusted.signatures[0].ret"), "{err}");
    }

    const WITH_NET: &str = r#"{
        "schema": 1,
        "profile": "trusted-native",
        "module": {"name": "netdemo", "sources": ["Net.aum"]},
        "capabilities": {
            "state_dir": ".state",
            "network": {
                "endpoints": ["127.0.0.1:8787", "*.example.com:443"],
                "allow_ips": ["127.0.0.0/8", "10.0.0.0/8"],
                "ca": "ca.pem",
                "timeout_ms": 1500
            }
        }
    }"#;

    /// U20: the project document names the endpoints, ranges, anchor and
    /// deadline, and the parsed values become the instance's dial policy.
    #[test]
    fn a_network_capability_parses_and_builds_a_policy() {
        let doc = parse(p(), WITH_NET).expect("network capability");
        let caps = doc.capabilities.expect("capabilities block");
        let net = caps.network.as_ref().expect("network block");
        assert_eq!(net.endpoints.len(), 2);
        assert_eq!(net.allow_ips.len(), 2);
        assert_eq!(net.ca.as_deref(), Some(std::path::Path::new("ca.pem")));
        assert_eq!(net.timeout_ms, Some(1500));

        // The anchor is a file, so this document-level test drops it; a policy
        // with endpoints and ranges still constructs (TLS is then refused).
        let no_anchor = WITH_NET.replace(",\n                \"ca\": \"ca.pem\"", "");
        let doc = parse(p(), &no_anchor).expect("network capability without anchor");
        let caps = doc.capabilities.expect("capabilities block");
        let policy = caps
            .net_policy(std::path::Path::new("/nonexistent"))
            .expect("policy constructs")
            .expect("network block grants a policy");
        assert_eq!(policy.endpoints().len(), 2);
        assert!(policy.approves_endpoint("127.0.0.1", 8787));
        assert!(policy.approves_endpoint("api.example.com", 443));
        assert!(!policy.approves_endpoint("example.com", 443));
        assert!(policy.approves_address("10.1.2.3".parse().unwrap()));
        assert!(!policy.approves_address("192.0.2.1".parse().unwrap()));
    }

    #[test]
    fn a_malformed_network_capability_is_named_by_field() {
        for (document, needle) in [
            (
                WITH_NET.replace("\"endpoints\"", "\"endpoint\""),
                "capabilities.network.endpoint",
            ),
            (
                WITH_NET.replace("\"127.0.0.1:8787\"", "\"127.0.0.1:notaport\""),
                "capabilities.network.endpoints[0]",
            ),
            (
                WITH_NET.replace("\"10.0.0.0/8\"", "\"10.0.0.0/33\""),
                "capabilities.network.allow_ips[1]",
            ),
            (
                WITH_NET.replace(
                    "\"endpoints\": [\"127.0.0.1:8787\", \"*.example.com:443\"]",
                    "\"endpoints\": []",
                ),
                "`capabilities.network.endpoints` is empty",
            ),
            (
                WITH_NET.replace(
                    "\"allow_ips\": [\"127.0.0.0/8\", \"10.0.0.0/8\"]",
                    "\"allow_ips\": []",
                ),
                "`capabilities.network.allow_ips` is empty",
            ),
            (
                WITH_NET.replace("\"timeout_ms\": 1500", "\"timeout_ms\": 0"),
                "`capabilities.network.timeout_ms` is 0",
            ),
            (
                WITH_NET.replace("\"ca\": \"ca.pem\"", "\"ca\": \"\""),
                "`capabilities.network.ca` is empty",
            ),
        ] {
            let error = parse(p(), &document).expect_err("malformed network capability");
            assert!(error.contains(needle), "{error}");
        }
    }
}
