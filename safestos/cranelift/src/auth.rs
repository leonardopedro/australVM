use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
}

pub trait AuthorizationEngine: Send + Sync {
    fn authorize(&self, principal: &str, action: &str, resource: &str) -> Result<Decision, String>;
}

#[derive(Debug, Clone, Default)]
pub struct AllowAll;

impl AuthorizationEngine for AllowAll {
    fn authorize(&self, _principal: &str, _action: &str, _resource: &str) -> Result<Decision, String> {
        Ok(Decision::Allow)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ManifestAuthEngine {
    grants: HashMap<String, HashSet<String>>,
}

impl ManifestAuthEngine {
    pub fn new() -> Self {
        Self { grants: HashMap::new() }
    }

    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let manifest: ManifestToml =
            toml::from_str(s).map_err(|e| format!("TOML parse error: {}", e))?;
        let mut engine = Self::new();
        let callables: HashSet<String> = manifest.grants.kernel.into_iter().collect();
        engine.grants.insert(manifest.module.name, callables);
        Ok(engine)
    }

    pub fn merge(&mut self, other: Self) {
        for (module, callables) in other.grants {
            self.grants.entry(module).or_default().extend(callables);
        }
    }

    pub fn grant(&mut self, module: &str, callable: &str) {
        self.grants
            .entry(module.to_string())
            .or_default()
            .insert(callable.to_string());
    }

    pub fn is_granted(&self, module: &str, callable: &str) -> bool {
        self.grants
            .get(module)
            .map_or(false, |set| set.contains(callable))
    }
}

impl AuthorizationEngine for ManifestAuthEngine {
    fn authorize(&self, principal: &str, action: &str, resource: &str) -> Result<Decision, String> {
        if action != "Call" {
            return Ok(Decision::Deny);
        }
        if self.is_granted(principal, resource) {
            Ok(Decision::Allow)
        } else {
            Ok(Decision::Deny)
        }
    }
}

#[derive(serde::Deserialize)]
struct ManifestToml {
    module: ModuleToml,
    #[serde(default)]
    grants: GrantsToml,
}

#[derive(serde::Deserialize)]
struct ModuleToml {
    name: String,
}

#[derive(serde::Deserialize, Default)]
struct GrantsToml {
    #[serde(default)]
    kernel: Vec<String>,
}

static AUTH_ENGINE: OnceLock<RwLock<Option<Box<dyn AuthorizationEngine>>>> = OnceLock::new();

static MANIFEST_ENGINE: OnceLock<RwLock<ManifestAuthEngine>> = OnceLock::new();

fn auth_lock() -> &'static RwLock<Option<Box<dyn AuthorizationEngine>>> {
    AUTH_ENGINE.get_or_init(|| RwLock::new(None))
}

fn manifest_lock() -> &'static RwLock<ManifestAuthEngine> {
    MANIFEST_ENGINE.get_or_init(|| RwLock::new(ManifestAuthEngine::new()))
}

pub fn set_auth_engine(engine: Box<dyn AuthorizationEngine>) {
    *auth_lock().write().unwrap_or_else(|e| e.into_inner()) = Some(engine);
}

pub fn check(principal: &str, action: &str, resource: &str) -> Result<(), String> {
    if let Ok(read) = auth_lock().read() {
        if let Some(engine) = read.as_ref() {
            return match engine.authorize(principal, action, resource) {
                Ok(Decision::Allow) => Ok(()),
                Ok(Decision::Deny) => Err(format!(
                    "Authorization denied: '{}' cannot '{}' '{}'",
                    principal, action, resource
                )),
                Err(e) => Err(e),
            };
        }
    }

    #[cfg(feature = "cedar")]
    {
        crate::policy::cedar_check(principal, action, resource)
    }
    #[cfg(not(feature = "cedar"))]
    {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            eprintln!(
                "safestos: no authorization engine installed, falling back to AllowAll"
            );
        });
        Ok(())
    }
}

#[no_mangle]
pub extern "C" fn safestos_load_auth_manifest(ptr: *const u8, len: usize) -> i64 {
    if ptr.is_null() || len == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let toml_str = match std::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let new_engine = match ManifestAuthEngine::from_toml_str(toml_str) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    {
        let mut engine = manifest_lock().write().unwrap_or_else(|e| e.into_inner());
        engine.merge(new_engine);
        let cloned = engine.clone();
        set_auth_engine(Box::new(cloned));
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &str = r#"
[module]
name = "demo_module"
version = "0.1.0"

[grants]
kernel = ["uk_version", "uk_model_create", "uk_evolve"]
"#;

    #[test]
    fn manifest_grant_deny() {
        let engine = ManifestAuthEngine::from_toml_str(VALID_MANIFEST).unwrap();
        assert_eq!(
            engine.authorize("demo_module", "Call", "uk_version").unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine.authorize("demo_module", "Call", "uk_evolve").unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine.authorize("demo_module", "Call", "uk_model_free").unwrap(),
            Decision::Deny
        );
        assert_eq!(
            engine.authorize("evil_module", "Call", "uk_version").unwrap(),
            Decision::Deny
        );
        assert_eq!(
            engine.authorize("demo_module", "Write", "uk_version").unwrap(),
            Decision::Deny
        );
    }

    #[test]
    fn manifest_merge() {
        let mut engine = ManifestAuthEngine::from_toml_str(VALID_MANIFEST).unwrap();
        let other = ManifestAuthEngine::from_toml_str(
            r#"
[module]
name = "other_module"

[grants]
kernel = ["uk_version"]
"#,
        )
        .unwrap();
        engine.merge(other);
        assert_eq!(
            engine.authorize("demo_module", "Call", "uk_version").unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine.authorize("other_module", "Call", "uk_version").unwrap(),
            Decision::Allow
        );
    }

    #[test]
    fn manifest_missing_grants() {
        let engine = ManifestAuthEngine::from_toml_str(
            r#"
[module]
name = "no_grants_module"
"#,
        )
        .unwrap();
        assert_eq!(
            engine.authorize("no_grants_module", "Call", "uk_version").unwrap(),
            Decision::Deny
        );
    }

    #[test]
    fn allow_all_permits_everything() {
        let engine = AllowAll;
        assert_eq!(
            engine.authorize("anyone", "Call", "anything").unwrap(),
            Decision::Allow
        );
    }

    #[test]
    fn manifest_bad_toml() {
        assert!(ManifestAuthEngine::from_toml_str("not valid toml = = }").is_err());
    }

    #[test]
    fn manifest_missing_module_section() {
        assert!(ManifestAuthEngine::from_toml_str(
            r#"
[grants]
kernel = ["uk_version"]
"#,
        )
        .is_err());
    }

    #[test]
    fn manifest_grant_method() {
        let mut engine = ManifestAuthEngine::new();
        engine.grant("my_module", "uk_version");
        assert!(engine.is_granted("my_module", "uk_version"));
        assert!(!engine.is_granted("my_module", "uk_evolve"));
        assert!(!engine.is_granted("other", "uk_version"));
    }
}
