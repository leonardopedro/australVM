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
        Ok(Self {
            name,
            version,
            archetypes,
            archetype,
            entry,
            grants,
            effects,
            max_ms,
        })
    }
}

pub struct ModuleHandle {
    pub manifest: ModuleManifest,
    pub functions: HashMap<String, usize>,
    pub cps_data: Vec<u8>,
    pub call_count: u64,
}

impl ModuleHandle {
    pub fn entrypoint_ptr(&self) -> Option<usize> {
        self.functions
            .get(&self.manifest.entry)
            .or_else(|| self.functions.get("run"))
            .or_else(|| self.functions.get("main"))
            .or_else(|| self.functions.values().next())
            .copied()
    }

    pub fn function_ptr(&self, name: &str) -> Option<usize> {
        self.functions.get(name).copied()
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

        let cps_data = std::fs::read(&cps_path)
            .map_err(|e| format!("cannot read {}: {e}", cps_path.display()))?;

        let engine = ManifestAuthEngine::from_toml_str(&toml_str)
            .map_err(|e| format!("manifest auth error: {e}"))?;
        auth::set_auth_engine(Box::new(engine));

        let functions = compile_cps_binary(&cps_data)?;

        let name = manifest.name.clone();
        let handle = ModuleHandle {
            manifest,
            functions,
            cps_data,
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
        let ptr = {
            let handle = self
                .modules
                .get(module_name)
                .ok_or_else(|| format!("module '{module_name}' not loaded"))?;
            handle
                .function_ptr(entrypoint)
                .ok_or_else(|| {
                    format!(
                        "entrypoint '{entrypoint}' not found in module '{module_name}'"
                    )
                })?
        };
        let result = unsafe { call_jit_function(ptr, args) };
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

        let new_cps_data = std::fs::read(&cps_path)
            .map_err(|e| format!("cannot read {}: {e}", cps_path.display()))?;

        let engine = ManifestAuthEngine::from_toml_str(&toml_str)
            .map_err(|e| format!("manifest auth error: {e}"))?;
        auth::set_auth_engine(Box::new(engine));

        let functions = compile_cps_binary(&new_cps_data)?;

        let handle = ModuleHandle {
            manifest: new_manifest,
            functions,
            cps_data: new_cps_data,
            call_count: 0,
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
