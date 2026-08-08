use std::collections::HashMap;
use std::path::Path;

use crate::auth::{self, ManifestAuthEngine};
use crate::cps::CpsModule;

#[derive(Debug, Clone)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub archetypes: Vec<String>,
    pub archetype: String,
    pub entry: String,
    pub grants: Vec<String>,
    pub effects: Vec<String>,
    /// F8 `[grants] observers`: other principals this module may read (actions,
    /// audit entries). A module always observes its own principal.
    pub observers: Vec<String>,
    /// S18 (F17): `[grants] resources` — resource ids introduced to this module this session
    /// (`uk_resource_use`); the loopback installs them on the caller grant set.
    pub resources: Vec<String>,
    pub fs_grants: Vec<String>,
    pub net_grants: Vec<String>,
    /// `[limits] max_ms` — per-call deadline for the host↔sidecar RPC (default 5 s).
    pub max_ms: Option<u64>,
    /// `[limits] memory_bytes` — optional memory cap applied via cgroup where writable.
    pub memory_max_bytes: Option<u64>,
}

impl ModuleManifest {
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let v: toml::Value =
            toml::from_str(s).map_err(|e| format!("TOML parse error: {e}"))?;
        let module = v
            .get("module")
            .ok_or("missing [module] section")?;
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
        let memory_max_bytes = v
            .get("limits")
            .and_then(|l| l.get("memory_bytes"))
            .and_then(|m| m.as_integer())
            .map(|m| m as u64);
        let fs_grants = v
            .get("grants")
            .and_then(|g| g.get("fs"))
            .and_then(|f| f.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
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
        Ok(Self {
            name,
            version,
            archetypes,
            archetype,
            entry,
            grants,
            effects,
            observers,
            resources,
            fs_grants,
            net_grants,
            max_ms,
            memory_max_bytes,
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

impl ModuleHost {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            #[cfg(feature = "ecmascript")]
            instances: HashMap::new(),
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

        let engine = ManifestAuthEngine::from_toml_str(&toml_str)
            .map_err(|e| format!("manifest auth error: {e}"))?;
        auth::set_auth_engine(Box::new(engine));

        // ECMAScript archetype: serve the module's JS via a workerd sidecar (S1) instead of
        // compiling a CPS binary into Cranelift IR.
        if manifest.archetype == "ecmascript" {
            #[cfg(feature = "ecmascript")]
            {
                let paths = crate::ecma::WorkerdPaths::from_env()?;
                let staging = module_dir.join(".unfer-ecma");
                let sidecar =
                    crate::ecma::EcmaSidecar::spawn(module_dir, &staging, &manifest, &paths)?;
                let name = manifest.name.clone();
                let handle = ModuleHandle {
                    manifest,
                    runtime: IrRuntime::Ecma(sidecar),
                    call_count: 0,
                };
                self.modules.insert(name.clone(), handle);
                return Ok(self.modules.get(&name).unwrap());
            }
            #[cfg(not(feature = "ecmascript"))]
            {
                return Err(
                    "archetype 'ecmascript' requires the 'ecmascript' feature (workerd sidecar)"
                        .to_string(),
                );
            }
        }

        let cps_data = std::fs::read(&cps_path)
            .map_err(|e| format!("cannot read {}: {e}", cps_path.display()))?;

        let functions = compile_cps_binary(&cps_data)?;

        let name = manifest.name.clone();
        let handle = ModuleHandle {
            manifest,
            runtime: IrRuntime::Jit {
                functions,
                cps_data,
            },
            call_count: 0,
        };
        self.modules.insert(name.clone(), handle);
        Ok(self.modules.get(&name).unwrap())
    }

    pub fn call(
        &mut self,
        module_name: &str,
        entrypoint: &str,
        args: &[i64],
    ) -> Result<i64, String> {
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
                    "module is ECMAScript but the 'ecmascript' feature is disabled".to_string()
                );
            }
        }

        let ptr = {
            let handle = self
                .modules
                .get(module_name)
                .ok_or_else(|| format!("module '{module_name}' not loaded"))?;
            handle
                .runtime
                .function_ptr(entrypoint)
                .ok_or_else(|| {
                    format!("entrypoint '{entrypoint}' not found in module '{module_name}'")
                })?
        };
        let result = unsafe { call_jit_function(ptr, args) };
        if let Some(handle) = self.modules.get_mut(module_name) {
            handle.call_count += 1;
        }
        Ok(result)
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

    pub fn swap(
        &mut self,
        module_name: &str,
        new_module_dir: &Path,
    ) -> Result<(), String> {
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

        let old_fs: std::collections::HashSet<&String> =
            old_handle.manifest.fs_grants.iter().collect();
        for g in &new_manifest.fs_grants {
            if !old_fs.contains(g) {
                return Err(format!(
                    "UK-4001 swap rejected: new module escalates fs grant '{g}'"
                ));
            }
        }

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

        let old_net: std::collections::HashSet<&String> =
            old_handle.manifest.net_grants.iter().collect();
        for g in &new_manifest.net_grants {
            if !old_net.contains(g) {
                return Err(format!(
                    "UK-4001 swap rejected: new module escalates net grant '{g}'"
                ));
            }
        }

        let new_cps_data = std::fs::read(&cps_path)
            .map_err(|e| format!("cannot read {}: {e}", cps_path.display()))?;

        let engine = ManifestAuthEngine::from_toml_str(&toml_str)
            .map_err(|e| format!("manifest auth error: {e}"))?;
        auth::set_auth_engine(Box::new(engine));

        let handle = if new_manifest.archetype == "ecmascript" {
            #[cfg(feature = "ecmascript")]
            {
                let paths = crate::ecma::WorkerdPaths::from_env()?;
                let staging = new_module_dir.join(".unfer-ecma");
                let sidecar = crate::ecma::EcmaSidecar::spawn(
                    new_module_dir,
                    &staging,
                    &new_manifest,
                    &paths,
                )?;
                ModuleHandle {
                    manifest: new_manifest,
                    runtime: IrRuntime::Ecma(sidecar),
                    call_count: 0,
                }
            }
            #[cfg(not(feature = "ecmascript"))]
            {
                return Err(
                    "archetype 'ecmascript' requires the 'ecmascript' feature (workerd sidecar)"
                        .to_string(),
                );
            }
        } else {
            let functions = compile_cps_binary(&new_cps_data)?;
            ModuleHandle {
                manifest: new_manifest,
                runtime: IrRuntime::Jit {
                    functions,
                    cps_data: new_cps_data,
                },
                call_count: 0,
            }
        };
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
                let handle = unfer_ffi::uk_restore(
                    session_bytes.as_ptr(),
                    session_bytes.len() as i64,
                );
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

    let target_builder =
        cranelift_native::builder().map_err(|e| format!("native builder: {e}"))?;
    let flag_builder = cranelift_codegen::settings::builder();
    let isa = target_builder
        .finish(cranelift_codegen::settings::Flags::new(flag_builder))
        .map_err(|e| format!("ISA finish: {e}"))?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

    extern "C" {
        fn au_print_int(i: i64);
        fn au_exit(code: i64);
        fn au_alloc(size: i64) -> *mut u8;
        fn au_free(ptr: *mut u8);
    }
    builder.symbol("au_print_int", au_print_int as *const u8);
    builder.symbol("au_exit", au_exit as *const u8);
    builder.symbol("au_alloc", au_alloc as *const u8);
    builder.symbol("au_free", au_free as *const u8);
    builder.symbol("__union_new", au_exit as *const u8);
    builder.symbol("__record_new", au_exit as *const u8);
    builder.symbol("__slot_get", au_exit as *const u8);

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
            let f: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 =
                std::mem::transmute(ptr);
            let mut a = [0i64; 4];
            for (i, v) in args.iter().take(4).enumerate() {
                a[i] = *v;
            }
            f(a[0], a[1], a[2], a[3])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ModuleManifest;

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
}
