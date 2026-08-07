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
    pub fs_grants: Vec<String>,
    pub net_grants: Vec<String>,
    pub max_ms: Option<u64>,
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
        let max_ms = v
            .get("limits")
            .and_then(|l| l.get("max_ms"))
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
        Ok(Self {
            name,
            version,
            archetypes,
            archetype,
            entry,
            grants,
            effects,
            fs_grants,
            net_grants,
            max_ms,
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
}

impl ModuleHost {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
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
                let sidecar = crate::ecma::EcmaSidecar::spawn(module_dir, &manifest, &paths)?;
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
                let sidecar = crate::ecma::EcmaSidecar::spawn(new_module_dir, &new_manifest, &paths)?;
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
