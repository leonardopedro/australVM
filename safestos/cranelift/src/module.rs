use std::collections::HashMap;
use std::path::Path;

use crate::auth::{self, ManifestAuthEngine};
use crate::cps::CpsModule;
use crate::verification::VerificationRequirements;

/// S19 (F18): gatekeeper provisioning modes, a replica of cloudflare's `disabled|optional|
/// `enabled`. A side-effecting module archetype with `[gatekeeper] mode = "disabled"` has its
/// submissions refused up front; anything else defers the mediation to the human console.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GatekeeperMode {
    #[default]
    Enabled,
    Optional,
    Disabled,
}

impl GatekeeperMode {
    pub fn from_toml(v: &str) -> Self {
        match v {
            "disabled" => Self::Disabled,
            "optional" => Self::Optional,
            _ => Self::Enabled,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub archetypes: Vec<String>,
    pub archetype: String,
    pub entry: String,
    pub grants: Vec<String>,
    pub effects: Vec<String>,
    /// S21 (F20): `[grants] effects` entries may be tables `{ name = "x", effect_kind = "observe" }`
    /// carrying a trust annotation (`observe` never queues; everything else is a `mutate`). Plain
    /// string entries grant the effect without an annotation (the kernel defaults them to mutate).
    pub effect_kinds: Vec<(String, String)>,
    /// F8 `[grants] observers`: other principals this module may read (actions,
    /// audit entries). A module always observes its own principal.
    pub observers: Vec<String>,
    /// S18 (F17): `[grants] resources` — resource ids introduced to this module this session
    /// (`uk_resource_use`); the loopback installs them on the caller grant set.
    pub resources: Vec<String>,
    pub fs_grants: Vec<String>,
    /// U19: the same entries with the rights each one carries. A plain string
    /// entry is the historical all-rights grant; a `{ root, rights }` table
    /// names a narrower one. Capability handles are opened with exactly these
    /// rights, so `read` cannot imply `write` or `create`.
    pub fs_root_rights: Vec<(String, crate::capstd_mod::caps::FsRights)>,
    pub net_grants: Vec<String>,
    /// U20: the address ranges a dialed name must resolve into. Every resolved
    /// address is checked, so an endpoint name alone is not a permission to
    /// reach whatever it currently points at. Empty means every dial is
    /// refused even when an endpoint is listed.
    pub net_ranges: Vec<String>,
    /// U20: the trust anchor bundle (PEM or DER, relative to the module
    /// directory) TLS dials verify against. Absent means TLS is refused rather
    /// than downgraded.
    pub net_ca: Option<String>,
    /// `[limits] net_timeout_ms` — connect and per-IO deadline for a dialed
    /// connection (default 5 s).
    pub net_timeout_ms: Option<u64>,
    /// `[limits] max_ms` — per-call deadline for the host↔sidecar RPC (default 5 s).
    pub max_ms: Option<u64>,
    /// U19: `[limits] max_write_bytes` — cumulative bytes this instance may add
    /// through its filesystem capabilities. Absent means no quota.
    pub max_write_bytes: Option<u64>,
    /// `[limits] memory_bytes` — optional memory cap applied via cgroup where writable.
    pub memory_max_bytes: Option<u64>,
    /// S19 (F18): `[gatekeeper] mode` — side-effect provisioning for gatekeeper modules.
    pub gatekeeper_mode: GatekeeperMode,
    /// D08: `[module] profile = "strict"`. In the strict profile a gated call is
    /// admitted only when the symbol is a declared ABI registry entry *and* the
    /// manifest grants it; no prefix is an ambient grant. Absent or any other
    /// value means the historical (grant-list) profile.
    pub strict: bool,
    /// D08: `[module] node` — the remote node this module serves. A context with
    /// a node is the only kind a receiver will accept across the distributed
    /// boundary, and the value is configuration: a peer never supplies it.
    pub node: Option<String>,
    /// D18: the optional versioned `[verify]` requirements. A legacy L8 table
    /// (`liquid` only) keeps its old meaning; a versioned table is enforced
    /// against `verification.ok` at load.
    pub verification: VerificationRequirements,
}

impl ModuleManifest {
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let v: toml::Value = toml::from_str(s).map_err(|e| format!("TOML parse error: {e}"))?;
        let module = v.get("module").ok_or("missing [module] section")?;
        let name = module
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or("missing module.name")?
            .to_string();
        let version = module
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0")
            .to_string();
        let archetypes = module
            .get("archetypes")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let entry = module
            .get("entry")
            .and_then(|e| e.as_str())
            .unwrap_or("run")
            .to_string();
        let grants = v
            .get("grants")
            .and_then(|g| g.get("kernel"))
            .and_then(|k| k.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let archetype = module
            .get("archetype")
            .and_then(|a| a.as_str())
            .unwrap_or("austral_cps")
            .to_string();
        let effects = v
            .get("grants")
            .and_then(|g| g.get("effects"))
            .and_then(|e| e.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // S21 (F20): annotated effect entries `{ name = "...", effect_kind = "observe" }` grant the
        // name AND carry the trust annotation; plain strings grant without annotation (kernel
        // treats them as mutations). The annotation is host metadata — never a vetted claim.
        let effect_kinds = v
            .get("grants")
            .and_then(|g| g.get("effects"))
            .and_then(|e| e.as_array())
            .map(|a| {
                let mut out = Vec::new();
                for item in a {
                    if let toml::Value::Table(t) = item {
                        if let Some(name) = t.get("name").and_then(|n| n.as_str()) {
                            let kind = t
                                .get("effect_kind")
                                .and_then(|k| k.as_str())
                                .unwrap_or("mutate");
                            out.push((name.to_string(), kind.to_string()));
                        }
                    }
                }
                out
            })
            .unwrap_or_default();
        let observers = v
            .get("grants")
            .and_then(|g| g.get("observers"))
            .and_then(|o| o.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let max_ms = v
            .get("limits")
            .and_then(|l| l.get("max_ms"))
            .and_then(|m| m.as_integer())
            .map(|m| m as u64);
        let max_write_bytes = v
            .get("limits")
            .and_then(|l| l.get("max_write_bytes"))
            .and_then(|m| m.as_integer())
            .map(|m| m as u64);
        let memory_max_bytes = v
            .get("limits")
            .and_then(|l| l.get("memory_bytes"))
            .and_then(|m| m.as_integer())
            .map(|m| m as u64);
        let gatekeeper_mode = v
            .get("gatekeeper")
            .and_then(|g| g.get("mode"))
            .and_then(|m| m.as_str())
            .map(GatekeeperMode::from_toml)
            .unwrap_or_default();
        // An unknown profile is not silently downgraded to the permissive one: a
        // manifest that asks for a posture this loader does not implement is an
        // error, because silently ignoring it would disable exactly the check the
        // author asked for.
        let strict = match module.get("profile").and_then(|p| p.as_str()) {
            None | Some("default") => false,
            Some("strict") => true,
            Some(other) => {
                return Err(format!(
                    "[module] profile must be \"default\" or \"strict\", got {other:?}"
                ))
            }
        };
        let node = match module.get("node") {
            None => None,
            Some(v) => match v.as_str() {
                Some(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => return Err("[module] node must be a non-empty string".to_string()),
            },
        };
        // U19: `[grants] fs` entries are either a path string (the historical
        // all-rights grant) or a `{ root, rights }` table. Unknown keys, a table
        // without `rights`, and an unknown right name are errors: a typo must
        // not silently widen or narrow what the module can do.
        let mut fs_grants: Vec<String> = Vec::new();
        let mut fs_root_rights: Vec<(String, crate::capstd_mod::caps::FsRights)> = Vec::new();
        if let Some(entries) = v
            .get("grants")
            .and_then(|g| g.get("fs"))
            .and_then(|f| f.as_array())
        {
            for (index, entry) in entries.iter().enumerate() {
                let (root, rights) = match entry {
                    toml::Value::String(root) => {
                        (root.clone(), crate::capstd_mod::caps::FsRights::ALL)
                    }
                    toml::Value::Table(table) => {
                        for key in table.keys() {
                            if key != "root" && key != "rights" {
                                return Err(format!(
                                    "[grants] fs[{index}] has unknown key {key:?} \
                                     (expected root, rights)"
                                ));
                            }
                        }
                        let root = table
                            .get("root")
                            .and_then(|r| r.as_str())
                            .ok_or_else(|| {
                                format!("[grants] fs[{index}] needs a string `root`")
                            })?
                            .to_string();
                        let text = table.get("rights").and_then(|r| r.as_str()).ok_or_else(
                            || {
                                format!(
                                    "[grants] fs[{index}] ({root}) needs `rights`; a root \
                                     without rights would be an implicit all-rights grant"
                                )
                            },
                        )?;
                        let rights = crate::capstd_mod::caps::FsRights::parse(text)
                            .map_err(|e| format!("[grants] fs[{index}] ({root}): {e}"))?;
                        (root, rights)
                    }
                    other => {
                        return Err(format!(
                            "[grants] fs[{index}] must be a path string or a \
                             {{ root, rights }} table, got {other:?}"
                        ))
                    }
                };
                fs_grants.push(root.clone());
                fs_root_rights.push((root, rights));
            }
        }
        let net_grants = v
            .get("grants")
            .and_then(|g| g.get("net"))
            .and_then(|n| n.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // U20: the address ranges and trust anchor complete the network grant.
        // A typo is refused when the policy is built, not ignored here.
        let net_ranges = v
            .get("grants")
            .and_then(|g| g.get("net_ips"))
            .and_then(|n| n.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let net_ca = v
            .get("grants")
            .and_then(|g| g.get("net_ca"))
            .and_then(|n| n.as_str())
            .map(String::from);
        let net_timeout_ms = v
            .get("limits")
            .and_then(|l| l.get("net_timeout_ms"))
            .and_then(|m| m.as_integer())
            .map(|m| m as u64);
        let resources = v
            .get("grants")
            .and_then(|g| g.get("resources"))
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // D18: parse the profile with its own strict reader. Unknown keys and
        // versioned fields without a version are errors, not ignored data.
        let verification = VerificationRequirements::from_toml_str(s)?;
        let profile_errors = verification.manifest_errors(strict);
        if !profile_errors.is_empty() {
            return Err(format!(
                "[verify] profile is inconsistent: {}",
                profile_errors.join("; ")
            ));
        }
        Ok(Self {
            name,
            version,
            archetypes,
            archetype,
            entry,
            grants,
            effects,
            effect_kinds,
            observers,
            resources,
            fs_grants,
            fs_root_rights,
            net_grants,
            net_ranges,
            net_ca,
            net_timeout_ms,
            max_ms,
            max_write_bytes,
            memory_max_bytes,
            gatekeeper_mode,
            strict,
            node,
            verification,
        })
    }
}

/// Active execution backend for a loaded module. S1 upgrades this from the bare
/// `functions`/`cps_data`/`ecma` fields into a single runtime abstraction so a
/// `ModuleHandle` carries exactly one backend.
pub enum IrRuntime {
    /// CPS binary compiled to native IR via the Cranelift JIT (the default/legacy path).
    Jit {
        /// `name -> finalized function pointer` for each exported CPS function.
        functions: HashMap<String, usize>,
        /// Raw compiled CPS bytes (retained for introspection / rebuilds).
        cps_data: Vec<u8>,
    },
    /// ECMAScript module served by a workerd sidecar (S1). Calls are JSON-RPC over a
    /// capability loopback rather than a native function pointer.
    #[cfg(feature = "ecmascript")]
    Ecma(crate::ecma::EcmaSidecar),
}

impl IrRuntime {
    pub fn is_ecmascript(&self) -> bool {
        #[cfg(feature = "ecmascript")]
        {
            matches!(self, IrRuntime::Ecma(_))
        }
        #[cfg(not(feature = "ecmascript"))]
        {
            false
        }
    }

    pub fn function_ptr(&self, name: &str) -> Option<usize> {
        match self {
            IrRuntime::Jit { functions, .. } => functions.get(name).copied(),
            #[cfg(feature = "ecmascript")]
            IrRuntime::Ecma(_) => None,
        }
    }

    pub fn entrypoint_ptr(&self, entry: &str) -> Option<usize> {
        self.function_ptr(entry)
            .or_else(|| self.function_ptr("run"))
            .or_else(|| self.function_ptr("main"))
            .or_else(|| match self {
                IrRuntime::Jit { functions, .. } => functions.values().next().copied(),
                #[cfg(feature = "ecmascript")]
                IrRuntime::Ecma(_) => None,
            })
    }
}

pub struct ModuleHandle {
    pub manifest: ModuleManifest,
    pub runtime: IrRuntime,
    pub call_count: u64,
    /// D08: this instance's immutable authority identity, bound at load. The
    /// host checks it before every call, so a revoked or replaced instance stops
    /// being callable even though its compiled code is still in memory.
    pub context: auth::InstanceContext,
}

impl ModuleHandle {
    #[cfg(feature = "ecmascript")]
    pub fn ecma(&self) -> Option<&crate::ecma::EcmaSidecar> {
        match &self.runtime {
            IrRuntime::Ecma(sidecar) => Some(sidecar),
            IrRuntime::Jit { .. } => None,
        }
    }

    /// True if this module is served by the workerd ECMAScript backend (S1).
    pub fn is_ecmascript(&self) -> bool {
        self.runtime.is_ecmascript()
    }

    /// Number of `uk_*` symbols / entrypoints in this module (JIT function count; 0 for
    /// ECMAScript, whose call surface is the granted capability set).
    pub fn function_count(&self) -> usize {
        match &self.runtime {
            IrRuntime::Jit { functions, .. } => functions.len(),
            #[cfg(feature = "ecmascript")]
            IrRuntime::Ecma(_) => 0,
        }
    }
}

pub struct ModuleHost {
    modules: HashMap<String, ModuleHandle>,
    /// Per-instance registry (S5/F3): each key is `"{module_name}@{instance_id}"`. Every
    /// instance owns its own workerd sidecar (own staging dir + unix sockets) and an optional
    /// kernel `Session` handle (from `instantiate_from_blueprint` / `restore_instance`).
    #[cfg(feature = "ecmascript")]
    instances: HashMap<String, ModuleInstance>,
}

/// One isolated instance of a module: a dedicated workerd sidecar + (optionally) a kernel
/// session handle. Created by [`ModuleHost::instantiate`] or [`ModuleHost::instantiate_from_blueprint`].
#[cfg(feature = "ecmascript")]
pub struct ModuleInstance {
    pub key: String,
    pub manifest: ModuleManifest,
    pub sidecar: crate::ecma::EcmaSidecar,
    pub dir: std::path::PathBuf,
    pub session: Option<i64>,
    pub call_count: u64,
}

#[cfg(feature = "ecmascript")]
impl ModuleInstance {
    pub fn session_handle(&self) -> Option<i64> {
        self.session
    }
}

impl Default for ModuleHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleHost {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            #[cfg(feature = "ecmascript")]
            instances: HashMap::new(),
        }
    }

    /// H8: resolve the module's runtime archetype through the registered
    /// harness resolver. `approved` is the module's own `[module] archetypes`
    /// allowlist (operator config may add to it); the module's `[module]
    /// archetype` is the request. Returns the resolved archetype id, or a
    /// non-retryable rejection (UK-4001 family) when the request is unapproved.
    pub fn resolve_runtime_choice(&self, manifest: &ModuleManifest) -> Result<String, String> {
        let approved: Vec<&str> = manifest.archetypes.iter().map(|s| s.as_str()).collect();
        let org = std::env::var("UNFER_HARNESS_ORG")
            .ok()
            .filter(|s| !s.is_empty());
        // No scope override wired in the loader yet; the module request is the
        // operative candidate, with `austral_cps` as the default fallback.
        match unfer_protocol::resolve_runtime_choice(
            &approved,
            org.as_deref(),
            None,
            "austral_cps",
            Some(&manifest.archetype),
        ) {
            unfer_protocol::RuntimeChoice::Profile(id) => Ok(id.to_string()),
            unfer_protocol::RuntimeChoice::Rejected { requested } => Err(format!(
                "UK-4001: archetype '{requested}' is not approved for module '{}'",
                manifest.name
            )),
        }
    }

    /// Instance key format: `"{module_name}@{instance_id}"`.
    #[cfg(feature = "ecmascript")]
    pub fn instance_key(module_name: &str, instance_id: &str) -> String {
        format!("{module_name}@{instance_id}")
    }

    pub fn load(&mut self, module_dir: &Path) -> Result<&ModuleHandle, String> {
        let manifest_path = module_dir.join("module.toml");
        let cps_path = module_dir.join("module.cps");

        let toml_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
        let manifest = ModuleManifest::from_toml_str(&toml_str)?;
        // D18: a versioned distributed profile is load-bearing. A required
        // profile without matching `verification.ok` fails before any code is
        // compiled, and an optional one leaves a loggable gap.
        let _verification_note = manifest
            .verification
            .check_evidence(module_dir, manifest.strict)?;

        let engine = ManifestAuthEngine::from_toml_str(&toml_str)
            .map_err(|e| format!("manifest auth error: {e}"))?;

        // D08: this module gets its own immutable identity and its grants are
        // merged with the modules already loaded, not swapped in over them. The
        // compile below runs inside this identity's caller scope, so the JIT gate
        // attributes each `uk_*` call to *this* module rather than to whichever
        // manifest happened to load last. The process-global deployment
        // principal is deliberately left alone: loading a module must not
        // redefine the authority of code compiled outside it.
        let context = auth::bind_instance_with_policy(
            manifest.name.clone(),
            manifest.strict,
            manifest.node.clone(),
            engine.clone(),
        );
        let previous_grants = auth::install_module_grants(engine);

        let instance = context.instance;
        let loaded = {
            let _scope = auth::enter_caller(&context);
            // H8: resolve the runtime archetype through the registered harness
            // resolver (approval + org floor + fallback); reject the unapproved one
            // (UK-4001 family) instead of silently falling back.
            match self.resolve_runtime_choice(&manifest) {
                Err(e) => Err(e),
                Ok(resolved) if resolved != manifest.archetype => Err(format!(
                    "UK-4001: archetype '{}' resolved to '{resolved}' which the loader does not \
                     map; add an adapter for the resolved profile",
                    manifest.archetype
                )),
                Ok(_) if manifest.archetype == "ecmascript" => {
                    // ECMAScript archetype: serve the module's JS via a workerd
                    // sidecar (S1) instead of compiling a CPS binary into Cranelift IR.
                    #[cfg(feature = "ecmascript")]
                    {
                        let paths = crate::ecma::WorkerdPaths::from_env();
                        match paths {
                            Ok(paths) => {
                                let staging = module_dir.join(".unfer-ecma");
                                crate::ecma::EcmaSidecar::spawn(
                                    module_dir, &staging, &manifest, &paths,
                                )
                                .map(|sidecar| {
                                    let name = manifest.name.clone();
                                    (
                                        name,
                                        ModuleHandle {
                                            manifest,
                                            runtime: IrRuntime::Ecma(sidecar),
                                            call_count: 0,
                                            context,
                                        },
                                    )
                                })
                            }
                            Err(e) => Err(e),
                        }
                    }
                    #[cfg(not(feature = "ecmascript"))]
                    {
                        Err("archetype 'ecmascript' requires the 'ecmascript' feature \
                             (workerd sidecar)"
                            .to_string())
                    }
                }
                Ok(_) => std::fs::read(&cps_path)
                    .map_err(|e| format!("cannot read {}: {e}", cps_path.display()))
                    .and_then(|cps_data| {
                        compile_cps_binary(&cps_data).map(|functions| (cps_data, functions))
                    })
                    .map(|(cps_data, functions)| {
                        let name = manifest.name.clone();
                        (
                            name,
                            ModuleHandle {
                                manifest,
                                runtime: IrRuntime::Jit {
                                    functions,
                                    cps_data,
                                },
                                call_count: 0,
                                context,
                            },
                        )
                    }),
            }
        };

        match loaded {
            Ok((name, handle)) => {
                // D09: the host builds this instance's capabilities — a directory
                // rooted in its own state directory and the single peer it serves
                // — and installs them under its identity. A module receives
                // handles from here; it never receives a path it could re-root or
                // a network capability that can dial an arbitrary host. A load
                // that cannot get its capability set is a failed load.
                let state_dir = module_dir.join(".state");
                let peers: Vec<String> = handle.manifest.node.clone().into_iter().collect();
                // U19: the state-dir handle gets the union of the rights on the
                // grants rooted at it (`.`/`""`); with no `[grants] fs` at all it
                // keeps the historical all-rights grant. Extra roots are opened
                // only by index and carry exactly the rights the grant names.
                let state_rights = if handle.manifest.fs_root_rights.is_empty() {
                    crate::capstd_mod::caps::FsRights::ALL
                } else {
                    handle
                        .manifest
                        .fs_root_rights
                        .iter()
                        .filter(|(root, _)| {
                            let root = root.trim();
                            root.is_empty() || root == "."
                        })
                        .fold(
                            crate::capstd_mod::caps::FsRights::NONE,
                            |acc, (_, rights)| acc.union(*rights),
                        )
                };
                let root_grants: Vec<crate::capstd_mod::caps::RootGrant> = handle
                    .manifest
                    .fs_root_rights
                    .iter()
                    .filter(|(root, _)| {
                        let root = root.trim();
                        !(root.is_empty() || root == ".")
                    })
                    .map(|(root, rights)| crate::capstd_mod::caps::RootGrant {
                        root: root.trim().to_string(),
                        rights: *rights,
                    })
                    .collect();
                // U20: the network grant becomes a policy, not an ambient
                // socket. An empty `[grants] net` issues no policy at all; a
                // listed endpoint without ranges still refuses every dial, and
                // an unreadable or malformed trust anchor is a failed load.
                let net_policy = if handle.manifest.net_grants.is_empty() {
                    None
                } else {
                    let built = (|| -> Result<crate::netstd_mod::NetPolicy, String> {
                        let ca = match &handle.manifest.net_ca {
                            Some(name) => {
                                let path = module_dir.join(name);
                                let bytes = std::fs::read(&path).map_err(|e| {
                                    format!("cannot read network trust anchor {}: {e}", path.display())
                                })?;
                                crate::netstd_mod::certs_from_pem(&bytes)?
                            }
                            None => Vec::new(),
                        };
                        crate::netstd_mod::NetPolicy::new(
                            &handle.manifest.net_grants,
                            &handle.manifest.net_ranges,
                            ca,
                            std::time::Duration::from_millis(
                                handle.manifest.net_timeout_ms.unwrap_or(5000),
                            ),
                        )
                    })();
                    match built {
                        Ok(policy) => Some(policy),
                        Err(error) => {
                            crate::auth::restore_manifest_engine(previous_grants);
                            auth::revoke_instance(instance);
                            return Err(format!(
                                "UK-4001: cannot construct network policy: {error}"
                            ));
                        }
                    }
                };
                match crate::capstd_mod::caps::InstanceCaps::new_with_grants(
                    handle.manifest.name.clone(),
                    &state_dir,
                    peers,
                    state_rights,
                    root_grants,
                    handle.manifest.max_write_bytes,
                ) {
                    Ok(caps) => {
                        let caps = match net_policy {
                            Some(policy) => caps.with_net_policy(policy),
                            None => caps,
                        };
                        crate::capstd_mod::caps::install(instance, caps)
                    }
                    Err(error) => {
                        crate::auth::restore_manifest_engine(previous_grants);
                        auth::revoke_instance(instance);
                        return Err(format!("UK-4001: cannot construct capabilities: {error}"));
                    }
                }
                self.modules.insert(name.clone(), handle);
                Ok(self.modules.get(&name).unwrap())
            }
            Err(error) => {
                // A failed load must leave the previous authority exactly as it
                // was: the grants this manifest added are rolled back and its
                // identity never becomes live.
                auth::restore_manifest_engine(previous_grants);
                auth::revoke_instance(instance);
                crate::capstd_mod::caps::uninstall(instance);
                Err(error)
            }
        }
    }

    pub fn call(
        &mut self,
        module_name: &str,
        entrypoint: &str,
        args: &[i64],
    ) -> Result<i64, String> {
        // D08: authority is checked at the host boundary before every call. The
        // compiled code cannot re-check itself, so this is where a revocation or
        // a hot-swap is enforced for a module that is already in memory.
        let context = {
            let handle = self
                .modules
                .get(module_name)
                .ok_or_else(|| format!("module '{module_name}' not loaded"))?;
            if let Some(refusal) = auth::context_refusal(&handle.context) {
                return Err(refusal);
            }
            handle.context.clone()
        };
        // Entered for the call so any gated helper invoked from compiled code (and
        // any nested host call) sees this module's identity, not a neighbour's.
        let _scope = auth::enter_caller(&context);

        // Resolve the runtime backend once; the ECMAScript path routes i64 args to the
        // sidecar's JSON RPC, otherwise we invoke the JIT function pointer directly.
        let is_ecma = {
            let handle = self
                .modules
                .get(module_name)
                .ok_or_else(|| format!("module '{module_name}' not loaded"))?;
            handle.is_ecmascript()
        };

        if is_ecma {
            #[cfg(feature = "ecmascript")]
            {
                // For the ECMAScript backend, `call` uses the JSON RPC path (args are JSON).
                // Convert the i64 slice to a JSON array for compatibility with the C ABI.
                let args_json = format!(
                    "[{}]",
                    args.iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                let result = self.call_json(module_name, entrypoint, &args_json)?;
                // The kernel RPC returns a JSON body; extract the numeric `result` field.
                let v: serde_json::Value = serde_json::from_str(&result)
                    .map_err(|e| format!("bad kernel response: {e}"))?;
                if let Some(handle) = self.modules.get_mut(module_name) {
                    handle.call_count += 1;
                }
                return v
                    .get("result")
                    .and_then(|r| r.as_i64())
                    .ok_or_else(|| format!("kernel response has no numeric result: {result}"));
            }
            #[cfg(not(feature = "ecmascript"))]
            {
                return Err(
                    "module is ECMAScript but the 'ecmascript' feature is disabled".to_string(),
                );
            }
        }

        let ptr = {
            let handle = self
                .modules
                .get(module_name)
                .ok_or_else(|| format!("module '{module_name}' not loaded"))?;
            handle.runtime.function_ptr(entrypoint).ok_or_else(|| {
                format!("entrypoint '{entrypoint}' not found in module '{module_name}'")
            })?
        };
        let result = unsafe { call_jit_function(ptr, args) };
        if let Some(handle) = self.modules.get_mut(module_name) {
            handle.call_count += 1;
        }
        Ok(result)
    }

    /// A call arriving across the distributed boundary.
    ///
    /// `peer` is the identity the transport authenticated for this connection —
    /// it is an argument of this method, never a field of the request, so a
    /// caller cannot name itself. `auth::check_remote_use` then requires the
    /// loaded instance to be bound to that same node (`[module] node`), to be
    /// live at this receiver, and to be granted the entrypoint here, before the
    /// call is dispatched. A local module (no node) is refused: it has no remote
    /// authority to lend.
    pub fn call_from_peer(
        &mut self,
        peer: &str,
        module_name: &str,
        entrypoint: &str,
        args: &[i64],
    ) -> Result<i64, String> {
        let context = self
            .modules
            .get(module_name)
            .ok_or_else(|| format!("module '{module_name}' not loaded"))?
            .context
            .clone();
        auth::check_remote_use(peer, &context, "Call", entrypoint)?;
        self.call(module_name, entrypoint, args)
    }

    /// ECMAScript-backend call path: RPC the workerd sidecar's entrypoint with a JSON payload.
    /// Returns the raw JSON response body from the worker.
    #[cfg(feature = "ecmascript")]
    pub fn call_json(
        &mut self,
        module_name: &str,
        entrypoint: &str,
        args_json: &str,
    ) -> Result<String, String> {
        let sidecar = {
            let handle = self
                .modules
                .get(module_name)
                .ok_or_else(|| format!("module '{module_name}' not loaded"))?;
            handle
                .ecma()
                .ok_or_else(|| format!("module '{module_name}' is not an ECMAScript module"))?
        };
        let result = sidecar.call(entrypoint, args_json)?;
        if let Some(handle) = self.modules.get_mut(module_name) {
            handle.call_count += 1;
        }
        Ok(result)
    }

    pub fn swap(&mut self, module_name: &str, new_module_dir: &Path) -> Result<(), String> {
        let old_handle = self
            .modules
            .get(module_name)
            .ok_or_else(|| format!("module '{module_name}' not loaded"))?;
        let old_name = old_handle.manifest.name.clone();

        let manifest_path = new_module_dir.join("module.toml");
        let cps_path = new_module_dir.join("module.cps");

        let toml_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
        let new_manifest = ModuleManifest::from_toml_str(&toml_str)?;
        new_manifest
            .verification
            .check_evidence(new_module_dir, new_manifest.strict)?;

        if new_manifest.name != old_name {
            return Err(format!(
                "swap rejected: module name mismatch ('{}' vs '{}')",
                old_name, new_manifest.name
            ));
        }

        let old_grants: std::collections::HashSet<&String> =
            old_handle.manifest.grants.iter().collect();
        for g in &new_manifest.grants {
            if !old_grants.contains(g) {
                return Err(format!(
                    "UK-4001 swap rejected: new module escalates grant '{g}'"
                ));
            }
        }

        // A grant is a path *and* the rights it carries; both must not grow.
        fs_grants_do_not_escalate(&old_handle.manifest, &new_manifest)?;

        // S4: the `effects` grant namespace must not escalate on swap either.
        let old_effects: std::collections::HashSet<&String> =
            old_handle.manifest.effects.iter().collect();
        for g in &new_manifest.effects {
            if !old_effects.contains(g) {
                return Err(format!(
                    "UK-4001 swap rejected: new module escalates effect grant '{g}'"
                ));
            }
        }

        // U20: the network grant is the endpoint list, the address ranges, the
        // trust anchor and the deadline. A swap may narrow any of them, never
        // widen: a new endpoint or range is a new authority, a dropped or
        // lengthened deadline is a looser one, and a different anchor is a
        // different set of peers trusted under the same name.
        net_grants_do_not_escalate(&old_handle.manifest, &new_manifest)?;

        // D08: a hot-swap may not relax the authority profile either. The new
        // binary keeps the identity it was loaded under, so a manifest that asks
        // for a different profile is a re-load, not a swap.
        if new_manifest.strict != old_handle.manifest.strict {
            return Err(
                "UK-4001 swap rejected: the replacement changes the module's authority profile \
                 (unload and load it again instead)"
                    .to_string(),
            );
        }
        // The served node is part of the identity too: a swap that points the
        // same instance at a different peer would let a remote caller change
        // which node it is talking to.
        if new_manifest.node != old_handle.manifest.node {
            return Err(
                "UK-4001 swap rejected: the replacement changes the module's served node \
                 (unload and load it again instead)"
                    .to_string(),
            );
        }
        // D18: evidence requirements are part of the module's posture, not a
        // property a hot-swap may silently widen.
        if new_manifest.verification != old_handle.manifest.verification {
            return Err(
                "UK-4001 swap rejected: the replacement changes the verification profile \
                 (unload and load it again instead)"
                    .to_string(),
            );
        }

        let new_cps_data = std::fs::read(&cps_path)
            .map_err(|e| format!("cannot read {}: {e}", cps_path.display()))?;

        let engine = ManifestAuthEngine::from_toml_str(&toml_str)
            .map_err(|e| format!("manifest auth error: {e}"))?;
        // D08: the replacement compiles under the module's *current* identity
        // (same name, same profile), so the gate sees this module's grants. On a
        // failed swap the grants are rolled back and the old handle — and its
        // live identity — stay in place.
        let current_context = old_handle.context.clone();
        // U09: compile under the replacement's snapshot, so a narrowing swap
        // cannot have a call admitted by the old grants, and a widening swap
        // does not hand the old code the new authority.
        let compile_context = auth::replace_policy(&current_context, engine.clone());
        let policy = engine.clone();
        let previous_grants = auth::install_module_grants(engine);
        let built = {
            let _scope = auth::enter_caller(&compile_context);
            if new_manifest.archetype == "ecmascript" {
                #[cfg(feature = "ecmascript")]
                {
                    crate::ecma::WorkerdPaths::from_env().and_then(|paths| {
                        let staging = new_module_dir.join(".unfer-ecma");
                        crate::ecma::EcmaSidecar::spawn(
                            new_module_dir,
                            &staging,
                            &new_manifest,
                            &paths,
                        )
                        .map(|sidecar| ModuleHandle {
                            manifest: new_manifest,
                            runtime: IrRuntime::Ecma(sidecar),
                            call_count: 0,
                            context: current_context.clone(),
                        })
                    })
                }
                #[cfg(not(feature = "ecmascript"))]
                {
                    Err(
                        "archetype 'ecmascript' requires the 'ecmascript' feature (workerd \
                         sidecar)"
                            .to_string(),
                    )
                }
            } else {
                compile_cps_binary(&new_cps_data).map(|functions| ModuleHandle {
                    manifest: new_manifest,
                    runtime: IrRuntime::Jit {
                        functions,
                        cps_data: new_cps_data,
                    },
                    call_count: 0,
                    context: current_context.clone(),
                })
            }
        };

        let mut handle = match built {
            Ok(handle) => handle,
            Err(error) => {
                auth::restore_manifest_engine(previous_grants);
                return Err(error);
            }
        };
        // The swap really happened: the old identity stops being live, so a
        // request that was already past its check under the previous code cannot
        // present that authority again.
        handle.context = auth::rebind_instance_with_policy(
            &current_context,
            handle.manifest.name.clone(),
            Some(policy),
        );
        self.modules.insert(module_name.to_string(), handle);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ModuleHandle> {
        self.modules.get(name)
    }

    pub fn loaded_modules(&self) -> Vec<&str> {
        self.modules.keys().map(|s| s.as_str()).collect()
    }

    // ── S5: per-instance isolation + blueprints ────────────────────────────

    /// Spawn a fresh, isolated instance of an ECMAScript module: its own workerd sidecar with a
    /// private staging dir + unix sockets, plus a new entry in the instance registry. Returns
    /// the instance key `"{name}@{instance_id}"`.
    #[cfg(feature = "ecmascript")]
    pub fn instantiate(
        &mut self,
        module_dir: &std::path::Path,
        instance_id: &str,
    ) -> Result<String, String> {
        let manifest_path = module_dir.join("module.toml");
        let toml_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
        let manifest = ModuleManifest::from_toml_str(&toml_str)?;
        if manifest.archetype != "ecmascript" {
            return Err(format!(
                "instance isolation requires the 'ecmascript' archetype, got '{}'",
                manifest.archetype
            ));
        }

        let key = Self::instance_key(&manifest.name, instance_id);
        let staging = module_dir.join(format!(".unfer-ecma-{instance_id}"));
        let paths = crate::ecma::WorkerdPaths::from_env()?;
        let sidecar = crate::ecma::EcmaSidecar::spawn(module_dir, &staging, &manifest, &paths)?;

        self.instances.insert(
            key.clone(),
            ModuleInstance {
                key: key.clone(),
                manifest,
                sidecar,
                dir: module_dir.to_path_buf(),
                session: None,
                call_count: 0,
            },
        );
        Ok(key)
    }

    /// Call an instance's entrypoint over its own sidecar's JSON RPC (raw JSON response body).
    #[cfg(feature = "ecmascript")]
    pub fn call_json_instance(
        &mut self,
        key: &str,
        entrypoint: &str,
        args_json: &str,
    ) -> Result<String, String> {
        let instance = self
            .instances
            .get_mut(key)
            .ok_or_else(|| format!("instance '{key}' not found"))?;
        let result = instance.sidecar.call(entrypoint, args_json)?;
        instance.call_count += 1;
        Ok(result)
    }

    /// The kernel `Session` handle bound to an instance (from `instantiate_from_blueprint` or
    /// `restore_instance`). `None` for instances created by `instantiate` (worker-side models).
    #[cfg(feature = "ecmascript")]
    pub fn session_handle(&self, key: &str) -> Option<i64> {
        self.instances.get(key).and_then(|i| i.session)
    }

    /// Snapshot a kernel `Session` handle to its `SessionBlob` JSON string (F3 durable
    /// suspension). The handle may be host-held (an instance session) or worker-created.
    #[cfg(feature = "ecmascript")]
    pub fn snapshot_session(&self, handle: i64) -> Result<String, String> {
        let needed = unfer_ffi::uk_snapshot(handle, std::ptr::null_mut(), 0);
        if needed < 0 {
            return Err(read_kernel_error(needed));
        }
        let mut buf = vec![0u8; needed as usize];
        let n = unfer_ffi::uk_snapshot(handle, buf.as_mut_ptr(), needed);
        if n < 0 {
            return Err(read_kernel_error(n));
        }
        String::from_utf8(buf).map_err(|e| format!("snapshot is not UTF-8: {e}"))
    }

    /// Bind an existing session snapshot (a `SessionBlob` JSON string from [`Self::snapshot_session`])
    /// to an instance, so it can resume a suspended computation. Returns the kernel handle.
    #[cfg(feature = "ecmascript")]
    pub fn restore_instance(&mut self, key: &str, session_json: &str) -> Result<i64, String> {
        let handle = unfer_ffi::uk_restore(session_json.as_ptr(), session_json.len() as i64);
        if handle < 0 {
            return Err(read_kernel_error(handle));
        }
        let instance = self
            .instances
            .get_mut(key)
            .ok_or_else(|| format!("instance '{key}' not found"))?;
        instance.session = Some(handle);
        Ok(handle)
    }

    /// Instantiate a module from a `.cell` blueprint archive (S5/F4, `initialize_from_blueprint`).
    /// Materializes the archived files into `parent_dir/{name}-{instance_id}`, spawns a fresh
    /// per-instance sidecar, and — if the archive carries a session snapshot — restores it as the
    /// instance's session. Returns `(instance_key, session_handle)`.
    #[cfg(feature = "ecmascript")]
    pub fn instantiate_from_blueprint(
        &mut self,
        cell: &[u8],
        parent_dir: &std::path::Path,
        instance_id: &str,
    ) -> Result<(String, Option<i64>), String> {
        let parsed = unfer_protocol::Cell::parse(cell)
            .map_err(|e| format!("UK-4100: bad blueprint archive: {e}"))?;

        // Materialize the archived module files into a per-instance directory.
        let name = parsed.metadata().name.clone();
        let dir = parent_dir.join(format!("{name}-{instance_id}"));
        std::fs::create_dir_all(&dir).map_err(|e| format!("materialize dir: {e}"))?;
        for (rel, bytes) in parsed.files() {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.split('/').any(|c| c == "..") {
                return Err(format!(
                    "UK-4001 blueprint rejected: path traversal in archive entry '{rel}'"
                ));
            }
            let dest = dir.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("materialize {rel}: {e}"))?;
            }
            std::fs::write(&dest, bytes).map_err(|e| format!("materialize {rel}: {e}"))?;
        }
        if !dir.join("module.toml").exists() {
            return Err("UK-4100 blueprint rejected: no module.toml in archive".to_string());
        }

        let toml_str = std::fs::read_to_string(dir.join("module.toml"))
            .map_err(|e| format!("cannot read materialized module.toml: {e}"))?;
        let manifest = ModuleManifest::from_toml_str(&toml_str)?;
        if manifest.archetype != "ecmascript" {
            return Err(format!(
                "instance isolation requires the 'ecmascript' archetype, got '{}'",
                manifest.archetype
            ));
        }

        let key = Self::instance_key(&name, instance_id);
        let staging = dir.join(".unfer-ecma");
        let paths = crate::ecma::WorkerdPaths::from_env()?;
        let sidecar = crate::ecma::EcmaSidecar::spawn(&dir, &staging, &manifest, &paths)?;

        let session = match parsed.session() {
            Some(session_bytes) => {
                let handle =
                    unfer_ffi::uk_restore(session_bytes.as_ptr(), session_bytes.len() as i64);
                if handle < 0 {
                    return Err(read_kernel_error(handle));
                }
                Some(handle)
            }
            None => None,
        };

        self.instances.insert(
            key.clone(),
            ModuleInstance {
                key: key.clone(),
                manifest,
                sidecar,
                dir,
                session,
                call_count: 0,
            },
        );
        Ok((key, session))
    }

    /// Access an instance (for tests: inspect staging dir / sidecar PID / session).
    #[cfg(feature = "ecmascript")]
    pub fn instance(&self, key: &str) -> Option<&ModuleInstance> {
        self.instances.get(key)
    }

    /// Drop an instance, killing its sidecar and releasing its kernel session.
    #[cfg(feature = "ecmascript")]
    pub fn drop_instance(&mut self, key: &str) -> Result<(), String> {
        let instance = self
            .instances
            .remove(key)
            .ok_or_else(|| format!("instance '{key}' not found"))?;
        if let Some(h) = instance.session {
            let ret = unfer_ffi::uk_model_free(h);
            if ret < 0 {
                return Err(read_kernel_error(ret));
            }
        }
        Ok(())
    }
}

#[cfg(feature = "ecmascript")]
fn read_kernel_error(ret: i64) -> String {
    let code = (-ret) as u32;
    let needed = unfer_ffi::uk_last_error(std::ptr::null_mut(), 0);
    if needed > 0 {
        let mut buf = vec![0u8; needed as usize];
        unfer_ffi::uk_last_error(buf.as_mut_ptr(), needed);
        if let Ok(s) = String::from_utf8(buf) {
            return format!("UK-{code}: {s}");
        }
    }
    format!("UK-{code}")
}

fn compile_cps_binary(data: &[u8]) -> Result<HashMap<String, usize>, String> {
    use cranelift_jit::{JITBuilder, JITModule};

    let target_builder = cranelift_native::builder().map_err(|e| format!("native builder: {e}"))?;
    let flag_builder = cranelift_codegen::settings::builder();
    let isa = target_builder
        .finish(cranelift_codegen::settings::Flags::new(flag_builder))
        .map_err(|e| format!("ISA finish: {e}"))?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    crate::use_arena_memory(&mut builder)?;

    extern "C" {
        fn au_print_int(i: i64);
        fn au_exit(code: i64);
        fn au_alloc(size: i64) -> *mut u8;
        fn au_calloc(elem_size: i64, count: i64) -> *mut u8;
        fn au_free(ptr: *mut u8);
    }
    builder.symbol("au_print_int", au_print_int as *const u8);
    builder.symbol("au_exit", au_exit as *const u8);
    builder.symbol("au_alloc", au_alloc as *const u8);
    builder.symbol("au_calloc", au_calloc as *const u8);
    builder.symbol("au_free", au_free as *const u8);
    builder.symbol("__union_new", au_exit as *const u8);
    builder.symbol("__record_new", au_exit as *const u8);
    builder.symbol("__slot_get", au_exit as *const u8);
    crate::register_float_symbols(&mut builder);
    // CPS-lowered Int64 `+ - * /` call the Austral.Pervasive::trapping*
    // helpers by name; without them a module using arithmetic cannot
    // finalize (the compiler's own JIT path already registers them).
    crate::register_trapping_symbols(&mut builder);

    #[cfg(feature = "unfer-kernel")]
    {
        for sym in crate::UNFER_SYMBOLS {
            builder.symbol(sym.name, sym.addr);
        }
    }
    #[cfg(feature = "zenodo-store")]
    {
        for sym in crate::ZENODO_SYMBOLS {
            builder.symbol(sym.name, sym.addr);
        }
    }
    // `rs_*` was reachable only from the compiler's own builder, so a hosted
    // module calling an `rs_*` symbol failed to resolve (`can't resolve symbol`)
    // even when the manifest granted it. The D09 capability verbs are `rs_*`, so
    // this is where the host has to know them; registration is not permission —
    // the manifest gate still decides by name.
    #[cfg(feature = "tier1-rs")]
    {
        for sym in crate::RS_FFI_SYMBOLS {
            builder.symbol(sym.name, sym.addr);
        }
    }

    let mut jit = JITModule::new(builder);
    let module: CpsModule = crate::cps::compile_cps_to_clif(&mut jit, data)?;
    jit.finalize_definitions()
        .map_err(|e| format!("finalize: {e}"))?;

    let mut functions = HashMap::new();
    for (name, fid) in &module.name_map {
        let ptr = jit.get_finalized_function(*fid) as usize;
        functions.insert(name.clone(), ptr);
    }

    // Leak the JIT module so function pointers remain valid.
    std::mem::forget(jit);

    Ok(functions)
}

unsafe fn call_jit_function(ptr: usize, args: &[i64]) -> i64 {
    match args.len() {
        0 => {
            let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(ptr);
            f()
        }
        1 => {
            let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(ptr);
            f(args[0])
        }
        2 => {
            let f: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ptr);
            f(args[0], args[1])
        }
        _ => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 = std::mem::transmute(ptr);
            let mut a = [0i64; 4];
            for (i, v) in args.iter().take(4).enumerate() {
                a[i] = *v;
            }
            f(a[0], a[1], a[2], a[3])
        }
    }
}

/// U19: a replacement may narrow the filesystem grants and the write quota, never
/// widen them. A path that was not granted before is a new grant; a right the old
/// grant did not carry is an escalation. `None` quota means unlimited, so moving
/// from a quota to `None` widens it.
fn net_grants_do_not_escalate(
    old: &ModuleManifest,
    new: &ModuleManifest,
) -> Result<(), String> {
    let old_endpoints: std::collections::HashSet<&String> = old.net_grants.iter().collect();
    for endpoint in &new.net_grants {
        if !old_endpoints.contains(endpoint) {
            return Err(format!(
                "UK-4001 swap rejected: new module escalates net grant '{endpoint}'"
            ));
        }
    }
    let old_ranges: std::collections::HashSet<&String> = old.net_ranges.iter().collect();
    for range in &new.net_ranges {
        if !old_ranges.contains(range) {
            return Err(format!(
                "UK-4001 swap rejected: new module escalates network range '{range}'"
            ));
        }
    }
    if new.net_ca != old.net_ca {
        return Err(
            "UK-4001 swap rejected: the replacement changes the network trust anchor \
             (unload and load it again instead)"
                .to_string(),
        );
    }
    match (old.net_timeout_ms, new.net_timeout_ms) {
        (Some(old), None) => Err(format!(
            "UK-4001 swap rejected: new module drops the {old}-ms network deadline"
        )),
        (Some(old), Some(new)) if new > old => Err(format!(
            "UK-4001 swap rejected: new module widens the network deadline from {old} to \
             {new} ms"
        )),
        _ => Ok(()),
    }
}

fn fs_grants_do_not_escalate(
    old: &ModuleManifest,
    new: &ModuleManifest,
) -> Result<(), String> {
    let old_fs: std::collections::HashMap<&str, crate::capstd_mod::caps::FsRights> = old
        .fs_root_rights
        .iter()
        .map(|(root, rights)| (root.as_str(), *rights))
        .collect();
    for (root, rights) in &new.fs_root_rights {
        match old_fs.get(root.as_str()) {
            None => {
                return Err(format!(
                    "UK-4001 swap rejected: new module escalates fs grant '{root}'"
                ))
            }
            Some(old_rights) if !old_rights.contains(*rights) => {
                return Err(format!(
                    "UK-4001 swap rejected: new module escalates fs rights on '{root}' \
                     from [{old_rights}] to [{rights}]"
                ))
            }
            Some(_) => {}
        }
    }
    match (old.max_write_bytes, new.max_write_bytes) {
        (Some(old), None) => Err(format!(
            "UK-4001 swap rejected: new module drops the {old}-byte write quota"
        )),
        (Some(old), Some(new)) if new > old => Err(format!(
            "UK-4001 swap rejected: new module widens the write quota from {old} to \
             {new} bytes"
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ModuleHost, ModuleManifest};

    #[test]
    fn parses_limits_table() {
        let toml = r#"
[module]
name = "m"
[grants]
kernel = ["uk_audit"]
[limits]
max_ms = 2500
memory_bytes = 134217728
"#;
        let m = ModuleManifest::from_toml_str(toml).expect("parse");
        assert_eq!(m.max_ms, Some(2500));
        assert_eq!(m.memory_max_bytes, Some(134217728));
    }

    #[test]
    fn a_swap_may_narrow_but_not_widen_fs_grants_and_quota() {
        use super::fs_grants_do_not_escalate;
        use crate::capstd_mod::caps::FsRights;

        let manifest = |grants: &str, quota: &str| {
            ModuleManifest::from_toml_str(&format!(
                "[module]\nname = \"m\"\n\n[grants]\nfs = [{grants}]\n\n[limits]\n{quota}"
            ))
            .expect("manifest parses")
        };
        let broad = manifest(
            "{ root = \"data\", rights = \"read,write\" }",
            "max_write_bytes = 4096",
        );
        let narrow = manifest("{ root = \"data\", rights = \"read\" }", "max_write_bytes = 1024");
        assert!(fs_grants_do_not_escalate(&broad, &narrow).is_ok());
        assert!(fs_grants_do_not_escalate(&narrow, &broad).is_err());

        // A new right on an existing root is an escalation, and so is a new root.
        let extra_right = manifest(
            "{ root = \"data\", rights = \"read,list\" }",
            "max_write_bytes = 1024",
        );
        let error = fs_grants_do_not_escalate(&narrow, &extra_right).unwrap_err();
        assert!(error.contains("escalates fs rights"), "{error}");

        let extra_root = manifest(
            "{ root = \"data\", rights = \"read\" }, \
             { root = \"cache\", rights = \"read\" }",
            "max_write_bytes = 1024",
        );
        let error = fs_grants_do_not_escalate(&narrow, &extra_root).unwrap_err();
        assert!(error.contains("escalates fs grant 'cache'"), "{error}");

        // Dropping the quota makes the grant unlimited.
        let unlimited = manifest("{ root = \"data\", rights = \"read\" }", "");
        let error = fs_grants_do_not_escalate(&narrow, &unlimited).unwrap_err();
        assert!(error.contains("drops the 1024-byte write quota"), "{error}");
        assert_eq!(
            manifest("{ root = \"data\", rights = \"read\" }", "").fs_root_rights[0].1,
            FsRights::READ
        );
    }

    /// U20: the manifest carries the endpoint list, the address ranges, the
    /// trust anchor and the deadline, and a swap may narrow them but not widen
    /// any of them.
    #[test]
    fn a_swap_may_narrow_but_not_widen_network_grants() {
        use super::net_grants_do_not_escalate;

        let manifest = |net: &str, ips: &str, ca: &str, timeout: &str| {
            ModuleManifest::from_toml_str(&format!(
                "[module]\nname = \"m\"\n\n[grants]\nnet = {net}\nnet_ips = {ips}{ca}\n\n\
                 [limits]\n{timeout}"
            ))
            .expect("manifest parses")
        };
        let broad = manifest(
            "[\"api.example.com:443\", \"127.0.0.1:8787\"]",
            "[\"10.0.0.0/8\", \"127.0.0.0/8\"]",
            "\nnet_ca = \"ca.pem\"",
            "net_timeout_ms = 4000",
        );
        assert_eq!(broad.net_grants.len(), 2);
        assert_eq!(broad.net_ranges.len(), 2);
        assert_eq!(broad.net_ca.as_deref(), Some("ca.pem"));
        assert_eq!(broad.net_timeout_ms, Some(4000));

        let narrow = manifest(
            "[\"127.0.0.1:8787\"]",
            "[\"127.0.0.0/8\"]",
            "\nnet_ca = \"ca.pem\"",
            "net_timeout_ms = 1000",
        );
        assert!(net_grants_do_not_escalate(&broad, &narrow).is_ok());
        assert!(net_grants_do_not_escalate(&narrow, &broad).is_err());

        let extra_endpoint = manifest(
            "[\"127.0.0.1:8787\", \"other.example.com:443\"]",
            "[\"127.0.0.0/8\"]",
            "\nnet_ca = \"ca.pem\"",
            "net_timeout_ms = 1000",
        );
        let error = net_grants_do_not_escalate(&narrow, &extra_endpoint).unwrap_err();
        assert!(error.contains("escalates net grant"), "{error}");

        let extra_range = manifest(
            "[\"127.0.0.1:8787\"]",
            "[\"127.0.0.0/8\", \"10.0.0.0/8\"]",
            "\nnet_ca = \"ca.pem\"",
            "net_timeout_ms = 1000",
        );
        let error = net_grants_do_not_escalate(&narrow, &extra_range).unwrap_err();
        assert!(error.contains("escalates network range"), "{error}");

        let other_anchor = manifest(
            "[\"127.0.0.1:8787\"]",
            "[\"127.0.0.0/8\"]",
            "\nnet_ca = \"other.pem\"",
            "net_timeout_ms = 1000",
        );
        let error = net_grants_do_not_escalate(&narrow, &other_anchor).unwrap_err();
        assert!(error.contains("changes the network trust anchor"), "{error}");

        let longer = manifest(
            "[\"127.0.0.1:8787\"]",
            "[\"127.0.0.0/8\"]",
            "\nnet_ca = \"ca.pem\"",
            "net_timeout_ms = 9000",
        );
        let error = net_grants_do_not_escalate(&narrow, &longer).unwrap_err();
        assert!(error.contains("widens the network deadline"), "{error}");

        let unlimited = manifest(
            "[\"127.0.0.1:8787\"]",
            "[\"127.0.0.0/8\"]",
            "\nnet_ca = \"ca.pem\"",
            "",
        );
        let error = net_grants_do_not_escalate(&narrow, &unlimited).unwrap_err();
        assert!(error.contains("drops the 1000-ms network deadline"), "{error}");
    }

    #[test]
    fn limits_default_absent() {
        let toml = r#"
[module]
name = "m"
[grants]
kernel = ["uk_audit"]
"#;
        let m = ModuleManifest::from_toml_str(toml).expect("parse");
        assert_eq!(m.max_ms, None);
        assert_eq!(m.memory_max_bytes, None);
    }

    // ── H8: archetype resolver (module harness registry) ────────────────

    #[test]
    fn resolve_archetype_per_scope() {
        let host = ModuleHost::new();
        let mut m = ModuleManifest::from_toml_str(
            r#"
[module]
name = "scoped"
archetypes = ["austral_cps", "ecmascript"]
archetype = "austral_cps"
"#,
        )
        .expect("parse");
        // The module request resolves to the approved archetype.
        assert_eq!(host.resolve_runtime_choice(&m).unwrap(), "austral_cps");
        // A scope override (per-scope selection) wins over the module request.
        m.archetype = "ecmascript".to_string();
        assert_eq!(host.resolve_runtime_choice(&m).unwrap(), "ecmascript");
    }

    #[test]
    fn unapproved_archetype_is_rejected() {
        let host = ModuleHost::new();
        let m = ModuleManifest::from_toml_str(
            r#"
[module]
name = "unapproved"
archetypes = ["austral_cps"]
archetype = "tidepool"
"#,
        )
        .expect("parse");
        let err = host.resolve_runtime_choice(&m).unwrap_err();
        assert!(
            err.contains("UK-4001"),
            "unapproved archetype must be rejected (UK-4001 family), got {err}"
        );
        assert!(
            err.contains("tidepool"),
            "rejection names the requested archetype"
        );
    }

    #[test]
    fn fallback_used_when_module_names_nothing() {
        let host = ModuleHost::new();
        let m = ModuleManifest::from_toml_str(
            r#"
[module]
name = "bare"
"#,
        )
        .expect("parse");
        // No `[module] archetype` → the default `austral_cps` fallback wins.
        assert_eq!(host.resolve_runtime_choice(&m).unwrap(), "austral_cps");
    }

    #[test]
    fn cold_restart_reuses_recorded_choice() {
        let host = ModuleHost::new();
        let toml = r#"
[module]
name = "restart"
archetypes = ["austral_cps", "ecmascript"]
archetype = "ecmascript"
"#;
        let m1 = ModuleManifest::from_toml_str(toml).expect("parse");
        let choice1 = host.resolve_runtime_choice(&m1).unwrap();
        // A cold restart re-parses the same manifest → the same resolved choice
        // (the resolver is pure/deterministic; the choice is recorded on the
        // handle via `load`).
        let m2 = ModuleManifest::from_toml_str(toml).expect("parse");
        let choice2 = host.resolve_runtime_choice(&m2).unwrap();
        assert_eq!(choice1, choice2);
        assert_eq!(choice1, "ecmascript");
    }
}
