use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

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
    fn authorize(
        &self,
        _principal: &str,
        _action: &str,
        _resource: &str,
    ) -> Result<Decision, String> {
        Ok(Decision::Allow)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ManifestAuthEngine {
    grants: HashMap<String, HashSet<String>>,
    effects: HashMap<String, HashSet<String>>,
    /// PLAN_liquid_types.md L8: the `[verify]` posture, carried alongside the
    /// grants rather than in a second parse of the same file.
    liquid: LiquidMode,
    qualifiers: Vec<String>,
}

impl ManifestAuthEngine {
    pub fn new() -> Self {
        Self {
            grants: HashMap::new(),
            effects: HashMap::new(),
            liquid: LiquidMode::Off,
            qualifiers: Vec::new(),
        }
    }

    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let manifest: ManifestToml =
            toml::from_str(s).map_err(|e| format!("TOML parse error: {}", e))?;
        let module_name = manifest.module.name.clone();
        let mut engine = Self::new();
        let mut callables: HashSet<String> = manifest.grants.kernel.into_iter().collect();
        // Fold `[grants] zenodo = [...]` (`uz_*` calls) into the module's grant
        // set: they are ordinary Call actions on the kernel-interface boundary,
        // exactly like `uk_*` symbols. Without this the zenodo module's positive
        // test can never pass (uz_init/uz_manifest_json would be ungranted).
        callables.extend(manifest.grants.zenodo);
        engine.grants.insert(module_name.clone(), callables);
        let effects: HashSet<String> = manifest.grants.effects.into_iter().collect();
        engine.effects.insert(module_name, effects);
        // `[verify]` is additive and defaults to `off`; a malformed value is a
        // hard error rather than a silent downgrade, because silently
        // downgrading `required` to `off` would disable exactly the check the
        // module author asked for.
        engine.liquid = match manifest.verify.liquid.as_deref() {
            None => LiquidMode::Off,
            Some(s) => LiquidMode::parse(s)?,
        };
        engine.qualifiers = manifest.verify.qualifiers;
        Ok(engine)
    }

    /// The `[verify] liquid` posture of this manifest.
    pub fn liquid_mode(&self) -> LiquidMode {
        self.liquid
    }

    /// The `[verify] qualifiers` list: the refinement predicates the module's
    /// prover was given. Recorded in the attestation so a host can tell which
    /// theory a verdict was proved against.
    pub fn liquid_qualifiers(&self) -> &[String] {
        &self.qualifiers
    }

    pub fn merge(&mut self, other: Self) {
        for (module, callables) in other.grants {
            self.grants
                .entry(module.clone())
                .or_default()
                .extend(callables);
            self.effects.entry(module).or_default();
        }
        for (module, effects) in other.effects {
            self.effects.entry(module).or_default().extend(effects);
        }
        // Merging takes the *stricter* liquid posture. Merging is used when
        // several manifests compose into one deployment, and the safe
        // direction for a verification requirement is upward: a deployment
        // that includes one `required` module must not become `optional`
        // because another manifest said so.
        self.liquid = match (self.liquid, other.liquid) {
            (LiquidMode::Required, _) | (_, LiquidMode::Required) => LiquidMode::Required,
            (LiquidMode::Optional, _) | (_, LiquidMode::Optional) => LiquidMode::Optional,
            _ => LiquidMode::Off,
        };
        for q in other.qualifiers {
            if !self.qualifiers.contains(&q) {
                self.qualifiers.push(q);
            }
        }
    }

    pub fn grant(&mut self, module: &str, callable: &str) {
        self.grants
            .entry(module.to_string())
            .or_default()
            .insert(callable.to_string());
    }

    /// Grant an effect (S4 `effects` namespace) to a module.
    pub fn grant_effect(&mut self, module: &str, effect: &str) {
        self.effects
            .entry(module.to_string())
            .or_default()
            .insert(effect.to_string());
    }

    pub fn is_granted(&self, module: &str, callable: &str) -> bool {
        self.grants
            .get(module)
            .is_some_and(|set| set.contains(callable))
    }

    pub fn is_effect_granted(&self, module: &str, effect: &str) -> bool {
        self.effects
            .get(module)
            .is_some_and(|set| set.contains(effect))
    }
}

impl AuthorizationEngine for ManifestAuthEngine {
    fn authorize(&self, principal: &str, action: &str, resource: &str) -> Result<Decision, String> {
        match action {
            // Kernel symbol call: `[grants] kernel = [...]`.
            "Call" => Ok(if self.is_granted(principal, resource) {
                Decision::Allow
            } else {
                Decision::Deny
            }),
            // Side-effecting op (S4): `[grants] effects = [...]`. The effect name is the
            // resource; only modules holding it may submit the corresponding action.
            "Effect" => Ok(if self.is_effect_granted(principal, resource) {
                Decision::Allow
            } else {
                Decision::Deny
            }),
            _ => Ok(Decision::Deny),
        }
    }
}

#[derive(serde::Deserialize)]
struct ManifestToml {
    module: ModuleToml,
    #[serde(default)]
    grants: GrantsToml,
    /// PLAN_liquid_types.md L8: additive `[verify]` section. Defaults to
    /// `LiquidMode::Off`, so a manifest written before this section behaves
    /// exactly as it did — the frozen vocabulary is untouched.
    #[serde(default)]
    verify: VerifyToml,
}

/// `[verify]` — how much liquid-type checking a module claims to have passed.
///
/// `Off` is the default and means "this module says nothing about liquid
/// types". `Optional` means a `liquid.ok` sidecar is honoured if present.
/// `Required` means the host refuses the module unless the sidecar is present
/// *and* its recorded hashes match what is on disk.
#[derive(serde::Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidMode {
    #[default]
    Off,
    Optional,
    Required,
}

impl LiquidMode {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "off" => Ok(LiquidMode::Off),
            "optional" => Ok(LiquidMode::Optional),
            "required" => Ok(LiquidMode::Required),
            other => Err(format!(
                "[verify] liquid must be \"off\", \"optional\" or \"required\", got {:?}",
                other
            )),
        }
    }
}

#[derive(serde::Deserialize, Default)]
struct VerifyToml {
    /// Raw string so a bad value produces our own message rather than a serde
    /// enum error that does not say which field was wrong.
    #[serde(default)]
    liquid: Option<String>,
    #[serde(default)]
    qualifiers: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ModuleToml {
    name: String,
}

#[derive(serde::Deserialize, Default)]
struct GrantsToml {
    #[serde(default)]
    kernel: Vec<String>,
    #[serde(default)]
    effects: Vec<String>,
    #[serde(default)]
    zenodo: Vec<String>,
}

static AUTH_ENGINE: OnceLock<RwLock<Option<Box<dyn AuthorizationEngine>>>> = OnceLock::new();

static MANIFEST_ENGINE: OnceLock<RwLock<ManifestAuthEngine>> = OnceLock::new();

/// The deployment principal: the `[module] name` of the most recently loaded
/// manifest. The JIT attributes every gated `uk_*`/`uz_*` kernel call to this
/// principal (not to the per-Austral-module name in a CPS header), so grants
/// keyed by the manifest name authorize calls made from any module in the
/// compiled program, including kernel-interface libraries.
static DEPLOYMENT_PRINCIPAL: OnceLock<RwLock<Option<String>>> = OnceLock::new();

fn deployment_lock() -> &'static RwLock<Option<String>> {
    DEPLOYMENT_PRINCIPAL.get_or_init(|| RwLock::new(None))
}

pub fn set_deployment_principal(name: String) {
    *deployment_lock().write().unwrap_or_else(|e| e.into_inner()) = Some(name);
}

pub fn deployment_principal() -> Option<String> {
    deployment_lock()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn auth_lock() -> &'static RwLock<Option<Box<dyn AuthorizationEngine>>> {
    AUTH_ENGINE.get_or_init(|| RwLock::new(None))
}

fn manifest_lock() -> &'static RwLock<ManifestAuthEngine> {
    MANIFEST_ENGINE.get_or_init(|| RwLock::new(ManifestAuthEngine::new()))
}

pub fn set_auth_engine(engine: Box<dyn AuthorizationEngine>) {
    *auth_lock().write().unwrap_or_else(|e| e.into_inner()) = Some(engine);
}

/// DenyAll engine — default when no manifest or Cedar is loaded.
#[derive(Debug, Clone, Default)]
pub struct DenyAll;

impl AuthorizationEngine for DenyAll {
    fn authorize(
        &self,
        principal: &str,
        _action: &str,
        resource: &str,
    ) -> Result<Decision, String> {
        eprintln!(
            "safestos: no authorization engine installed for '{}' calling '{}' — DENYING by default. \
             Load a manifest with safestos_load_auth_manifest or pass --allow-all for AllowAll.",
            principal, resource
        );
        Ok(Decision::Deny)
    }
}

/// Set the engine that `check()` uses. If never called, `check()` uses
/// [`DenyAll`] by default (secure). Call [`set_allow_all`] or pass
/// `--allow-all` to override.
pub fn set_deny_all() {
    *auth_lock().write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(DenyAll));
}

/// Override to AllowAll (for `--allow-all` flag or test helpers).
pub fn set_allow_all() {
    *auth_lock().write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(AllowAll));
}

pub fn check(principal: &str, action: &str, resource: &str) -> Result<(), String> {
    if let Ok(read) = auth_lock().read() {
        if let Some(engine) = read.as_ref() {
            return match engine.authorize(principal, action, resource) {
                Ok(Decision::Allow) => Ok(()),
                Ok(Decision::Deny) => Err(format!(
                    "UK-4001: Authorization denied — '{}' is not granted '{}'",
                    principal, resource
                )),
                Err(e) => Err(e),
            };
        }
    }

    // No engine installed at all → use DenyAll by default (secure).
    // Install a singleton DenyAll so subsequent calls skip this branch.
    {
        let mut write = auth_lock().write().unwrap_or_else(|e| e.into_inner());
        if write.is_none() {
            *write = Some(Box::new(DenyAll));
        }
    }
    check(principal, action, resource)
}

/// Load a TOML auth manifest. Returns 1 on success, 0 on failure.
///
/// # Safety
///
/// `ptr` must point to `len` readable bytes for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn safestos_load_auth_manifest(ptr: *const u8, len: usize) -> i64 {
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
    set_deployment_principal(new_engine_principal(toml_str));
    1
}

/// Extract the manifest's `[module] name` for use as the deployment principal.
/// On any parse failure, falls back to an empty principal so the JIT's
/// per-module fallback still applies.
fn new_engine_principal(toml_str: &str) -> String {
    #[derive(serde::Deserialize)]
    struct NameOnly {
        module: ModuleToml,
    }
    match toml::from_str::<NameOnly>(toml_str) {
        Ok(n) => n.module.name,
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// D08: immutable per-instance caller context
// ---------------------------------------------------------------------------
//
// Before this block the JIT answered "who is calling?" with a process-global:
// whichever manifest loaded most recently set `DEPLOYMENT_PRINCIPAL` and
// replaced the whole authorization engine. With two modules loaded, module A's
// calls were attributed to module B, so A borrowed B's grants — and A's own
// grants disappeared the moment B loaded. Loading a module that then failed to
// compile left the new engine installed anyway.
//
// An [`InstanceContext`] is bound once, at load, and immutable for the life of
// the loaded module. It keeps the three identities the plan requires *distinct*:
//
// * `module`     — the deployment module the manifest grants by;
// * `delegator`  — the user (or the module) on whose behalf it acts;
// * `node`       — the remote node it is executing for, if any.
//
// `instance` + `epoch` are the identity the host validates: a context is live
// only while its id is registered with the same epoch, so revocation and
// hot-swap invalidate an already-compiled module's authority without touching
// the code that was generated.

/// The identity one loaded module instance acts as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceContext {
    /// Allocated once per `load`; never reused, never guessed.
    pub instance: u64,
    /// The manifest's `[module] name` — the principal grants are keyed by.
    pub module: String,
    /// The user on whose behalf the module acts, when a host supplied one.
    pub delegator: Option<String>,
    /// The remote node this instance serves, when it is a distributed endpoint.
    pub node: Option<String>,
    /// Bumped when the instance is reloaded or its policy is replaced.
    pub epoch: u64,
    /// D08 strict profile: only declared ABI registry entries are callable.
    pub strict: bool,
}

impl InstanceContext {
    /// A context with no delegator and no remote node (a local module).
    pub fn local(instance: u64, module: impl Into<String>, strict: bool) -> Self {
        Self {
            instance,
            module: module.into(),
            delegator: None,
            node: None,
            epoch: 0,
            strict,
        }
    }

    /// The same context acting for a user and/or a remote node.
    pub fn delegated(mut self, delegator: Option<String>, node: Option<String>) -> Self {
        self.delegator = delegator;
        self.node = node;
        self
    }

    /// A one-line description for diagnostics; never a rendered grant list.
    pub fn label(&self) -> String {
        match (&self.delegator, &self.node) {
            (None, None) => format!("{}#{}@{}", self.module, self.instance, self.epoch),
            (Some(user), None) => format!(
                "{}#{}@{} for {user}",
                self.module, self.instance, self.epoch
            ),
            (None, Some(node)) => {
                format!("{}#{}@{} at {node}", self.module, self.instance, self.epoch)
            }
            (Some(user), Some(node)) => format!(
                "{}#{}@{} for {user} at {node}",
                self.module, self.instance, self.epoch
            ),
        }
    }
}

static INSTANCES: OnceLock<RwLock<HashMap<u64, InstanceContext>>> = OnceLock::new();
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

fn instance_registry() -> &'static RwLock<HashMap<u64, InstanceContext>> {
    INSTANCES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Bind a fresh immutable instance identity. The context returned is the one the
/// host attaches to the compiled module; nothing else can change its module.
pub fn bind_instance(module: impl Into<String>, strict: bool) -> InstanceContext {
    bind_instance_for_node(module, strict, None)
}

/// Bind a fresh identity for a module that serves a remote node (`[module]
/// node = "…"`). The `node` recorded here is the one a receiver compares with
/// the peer its transport authenticated: it is configuration, never something
/// the calling peer supplies about itself.
pub fn bind_instance_for_node(
    module: impl Into<String>,
    strict: bool,
    node: Option<String>,
) -> InstanceContext {
    let id = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
    let context = InstanceContext::local(id, module, strict).delegated(None, node);
    instance_registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, context.clone());
    context
}

/// Whether this context is still the registered one for its instance id. A
/// revoked instance, or one whose epoch moved (reload/hot-swap), is not live:
/// the caller must stop trusting authority it obtained before the change.
pub fn is_live(context: &InstanceContext) -> bool {
    instance_registry()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&context.instance)
        .is_some_and(|registered| registered.epoch == context.epoch)
}

/// Revoke an instance's authority. Returns whether it was live. Already-compiled
/// code keeps its bytes; what stops is the authority those bytes carried.
pub fn revoke_instance(instance: u64) -> bool {
    instance_registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&instance)
        .is_some()
}

/// Move an instance to a new epoch (reload/hot-swap), returning the replacement
/// context. The old one stops being live immediately.
pub fn rebind_instance(context: &InstanceContext, module: impl Into<String>) -> InstanceContext {
    let replacement = InstanceContext {
        instance: context.instance,
        module: module.into(),
        delegator: context.delegator.clone(),
        node: context.node.clone(),
        epoch: context.epoch + 1,
        strict: context.strict,
    };
    instance_registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(replacement.instance, replacement.clone());
    replacement
}

thread_local! {
    /// The context of the module instance whose code is being compiled or run on
    /// this thread. A stack, so a nested host call restores its caller's context.
    static CALLER_STACK: RefCell<Vec<Arc<InstanceContext>>> = const { RefCell::new(Vec::new()) };
}

/// The guard that pops the caller context when it goes out of scope.
pub struct CallerScope {
    _private: (),
}

impl Drop for CallerScope {
    fn drop(&mut self) {
        CALLER_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Enter `context` as the current caller for the rest of this scope.
pub fn enter_caller(context: &InstanceContext) -> CallerScope {
    CALLER_STACK.with(|stack| stack.borrow_mut().push(Arc::new(context.clone())));
    CallerScope { _private: () }
}

/// The current caller, if a host entered one.
pub fn current_caller() -> Option<Arc<InstanceContext>> {
    CALLER_STACK.with(|stack| stack.borrow().last().cloned())
}

/// Why this context may not act right now, if it may not. Separate from a grant
/// check so the two refusals stay distinguishable in a diagnostic.
pub fn context_refusal(context: &InstanceContext) -> Option<String> {
    if context.module.trim().is_empty() {
        return Some("UK-4001: the caller context names no module".to_string());
    }
    if !is_live(context) {
        return Some(format!(
            "UK-4001: the caller context {} is no longer live (revoked or replaced); \
             already-compiled code may not keep using authority it obtained earlier",
            context.label()
        ));
    }
    None
}

// ── strict profile: the declared ABI registry ───────────────────────
//
// A prefix is not a grant. `uk_`/`uz_`/`rs_` name *namespaces*, and the old gate
// admitted any symbol in them that a manifest happened to list. The strict
// profile additionally requires the symbol to be one the bridge actually
// declared (the same tables the JIT builder registers from), so a typo, a symbol
// from another build, or a name a manifest invented cannot become a call.

static STRICT_PROFILE: AtomicBool = AtomicBool::new(false);

static DECLARED_ABI: OnceLock<RwLock<Option<HashSet<String>>>> = OnceLock::new();

/// Every ABI symbol name the bridge declares, derived from the tables the JIT
/// registers (`UNFER_SYMBOLS`, `ZENODO_SYMBOLS`, `RS_FFI_SYMBOLS`) plus the
/// `au_*`/`__*` runtime builtins. One source of truth: a symbol the JIT can
/// resolve is a symbol the strict profile can name.
fn declared_abi() -> &'static RwLock<Option<HashSet<String>>> {
    DECLARED_ABI.get_or_init(|| {
        let mut names: HashSet<String> = HashSet::new();
        #[cfg(feature = "unfer-kernel")]
        for symbol in crate::UNFER_SYMBOLS {
            names.insert(symbol.name.to_string());
        }
        #[cfg(feature = "zenodo-store")]
        for symbol in crate::ZENODO_SYMBOLS {
            names.insert(symbol.name.to_string());
        }
        #[cfg(feature = "tier1-rs")]
        for symbol in crate::RS_FFI_SYMBOLS {
            names.insert(symbol.name.to_string());
        }
        for name in [
            "au_print_int",
            "au_exit",
            "au_alloc",
            "au_calloc",
            "au_free",
            "__union_new",
            "__record_new",
            "__slot_get",
        ] {
            names.insert(name.to_string());
        }
        RwLock::new(Some(names))
    })
}

/// Turn the strict profile on or off for callers without an instance context
/// (legacy binaries and `modhost authorize`). A host-loaded module uses the
/// profile recorded in its own [`InstanceContext`] instead.
pub fn set_strict_profile(on: bool) {
    STRICT_PROFILE.store(on, Ordering::Relaxed);
}

pub fn strict_profile() -> bool {
    STRICT_PROFILE.load(Ordering::Relaxed)
}

/// Declare an ABI symbol beyond the built-in tables (a host-provided adapter).
pub fn declare_abi_symbol(name: &str) {
    let mut guard = declared_abi().write().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashSet::new)
        .insert(name.to_string());
}

/// Whether `name` is a declared ABI symbol.
pub fn is_declared_abi_symbol(name: &str) -> bool {
    declared_abi()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|names| names.contains(name))
}

/// Check one gated call for an instance context: liveness first (stale authority
/// is refused before any grant is consulted), then the strict ABI check, then the
/// grant for this context's own module.
pub fn check_instance_call(context: &InstanceContext, callee: &str) -> Result<(), String> {
    if let Some(refusal) = context_refusal(context) {
        return Err(refusal);
    }
    if context.strict && !is_declared_abi_symbol(callee) {
        return Err(format!(
            "UK-4001: strict profile refuses '{}' — it is not a declared ABI symbol \
             (a prefix is not a grant)",
            callee
        ));
    }
    check(&context.module, "Call", callee)
}

/// The receiver side of the distributed/effect boundary.
///
/// A context arrives from another process, so the receiver must not trust it:
/// `from_peer` is the peer identity the transport *authenticated*, passed
/// separately from the token, and it must equal the context's `node`. The
/// context is then checked for liveness at the receiver (its own registry) and
/// for a grant — never by asking the sender.
pub fn check_remote_use(
    from_peer: &str,
    context: &InstanceContext,
    action: &str,
    resource: &str,
) -> Result<(), String> {
    match context.node.as_deref() {
        Some(node) if node == from_peer => {}
        Some(node) => {
            return Err(format!(
                "UK-4001: context {} claims node '{node}' but the authenticated peer is \
                 '{from_peer}': a context cannot certify its own node",
                context.label()
            ))
        }
        None => {
            return Err(format!(
                "UK-4001: context {} names no remote node and cannot be used across the \
                 distributed boundary",
                context.label()
            ))
        }
    }
    if let Some(refusal) = context_refusal(context) {
        return Err(refusal);
    }
    if context.strict && !is_declared_abi_symbol(resource) {
        return Err(format!(
            "UK-4001: strict profile refuses '{resource}' — it is not a declared ABI symbol"
        ));
    }
    // The grant is looked up by the *context's* module at the receiver's own
    // policy, so a sender's stale copy of its grants buys nothing here.
    check(&context.module, action, resource)
}

/// Snapshot the manifest engine so a failed load can be rolled back exactly
/// (D08: a load that fails must leave the previous authority untouched).
pub fn manifest_engine_snapshot() -> ManifestAuthEngine {
    manifest_lock()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Merge a module's grants into the live manifest engine instead of replacing it,
/// so loading module B does not forget module A's grants. Returns the previous
/// engine for [`restore_manifest_engine`].
pub fn install_module_grants(engine: ManifestAuthEngine) -> ManifestAuthEngine {
    let previous = manifest_engine_snapshot();
    let mut merged = manifest_lock().write().unwrap_or_else(|e| e.into_inner());
    merged.merge(engine);
    let live = merged.clone();
    drop(merged);
    set_auth_engine(Box::new(live));
    previous
}

/// Put back a snapshot from [`install_module_grants`] (used when the load that
/// installed the grants then failed).
pub fn restore_manifest_engine(previous: ManifestAuthEngine) {
    *manifest_lock().write().unwrap_or_else(|e| e.into_inner()) = previous.clone();
    set_auth_engine(Box::new(previous));
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
            engine
                .authorize("demo_module", "Call", "uk_version")
                .unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine
                .authorize("demo_module", "Call", "uk_evolve")
                .unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine
                .authorize("demo_module", "Call", "uk_model_free")
                .unwrap(),
            Decision::Deny
        );
        assert_eq!(
            engine
                .authorize("evil_module", "Call", "uk_version")
                .unwrap(),
            Decision::Deny
        );
        assert_eq!(
            engine
                .authorize("demo_module", "Write", "uk_version")
                .unwrap(),
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
            engine
                .authorize("demo_module", "Call", "uk_version")
                .unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine
                .authorize("other_module", "Call", "uk_version")
                .unwrap(),
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
            engine
                .authorize("no_grants_module", "Call", "uk_version")
                .unwrap(),
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

    #[test]
    fn manifest_effect_grant_namespace() {
        // S4: `[grants] effects = [...]` is a separate capability namespace from
        // `[grants] kernel = [...]`, gated by the "Effect" action.
        let engine = ManifestAuthEngine::from_toml_str(
            r#"
[module]
name = "client_module"

[grants]
effects = ["send_notification", "run_experiment"]
"#,
        )
        .unwrap();
        assert_eq!(
            engine
                .authorize("client_module", "Effect", "send_notification")
                .unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine
                .authorize("client_module", "Effect", "run_experiment")
                .unwrap(),
            Decision::Allow
        );
        assert_eq!(
            engine
                .authorize("client_module", "Effect", "unlisted_effect")
                .unwrap(),
            Decision::Deny
        );
        // Effects are not kernel grants and vice versa.
        assert_eq!(
            engine
                .authorize("client_module", "Call", "send_notification")
                .unwrap(),
            Decision::Deny
        );
        assert_eq!(
            engine
                .authorize("client_module", "Effect", "uk_version")
                .unwrap(),
            Decision::Deny
        );
        // A module without the effect grant is denied even for a listed effect name.
        let denied = ManifestAuthEngine::from_toml_str(
            r#"
[module]
name = "other_module"
"#,
        )
        .unwrap();
        assert_eq!(
            denied
                .authorize("other_module", "Effect", "send_notification")
                .unwrap(),
            Decision::Deny
        );
    }

    #[test]
    fn manifest_effect_merge() {
        let mut engine = ManifestAuthEngine::from_toml_str(
            r#"
[module]
name = "a"

[grants]
effects = ["x"]
"#,
        )
        .unwrap();
        engine.merge(
            ManifestAuthEngine::from_toml_str(
                r#"
[module]
name = "b"

[grants]
effects = ["y"]
"#,
            )
            .unwrap(),
        );
        assert!(engine.is_effect_granted("a", "x"));
        assert!(engine.is_effect_granted("b", "y"));
        assert!(!engine.is_effect_granted("a", "y"));
    }

    #[test]
    fn manifest_effect_grant_method() {
        let mut engine = ManifestAuthEngine::new();
        engine.grant_effect("my_module", "send_notification");
        assert!(engine.is_effect_granted("my_module", "send_notification"));
        assert!(!engine.is_effect_granted("my_module", "run_experiment"));
        assert!(!engine.is_effect_granted("other", "send_notification"));
    }
}
