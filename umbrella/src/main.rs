//! `umbrella` — PLAN_valen §5 P3.
//!
//! A host project that lives OUTSIDE australVM and drives the whole
//! extern-rust pipeline end to end by consuming australVM as (a) a
//! library and (b) a front-end subprocess:
//!
//! 1. **cargo** builds an arbitrary crate (`modules/rscrate`) to a
//!    cdylib — the `--message-format=json` stream yields the artifact
//!    path, so no target-layout assumption is baked in;
//! 2. **explicit provider admission** maps that artifact `RTLD_LOCAL` and
//!    resolves each declared export from its own `dlsym` handle, then registers
//!    those exact addresses in the bridge before the JIT initializes. A
//!    preloaded same-named export cannot win the registered JIT import.
//!    Registration is not authorization: the manifest gate still decides every
//!    call by name;
//! 3. the stock **`austral` binary** (`compile ... --use-cps-jit
//!    --emit-cps-only --target-type=tc --emit-cps=...`) lowers the module to
//!    CPS without initializing the bridge or invoking the crate just built;
//!    australVM never learns about that crate at this stage, and no
//!    `--allow-all` grant is needed because nothing is compiled or run;
//! 4. the **bridge, linked as a plain path dependency**, loads the
//!    manifest grant list (or an explicit DenyAll), compiles the CPS
//!    with the gate live, and the entry function execates in-process.
//!
//! Denied run (no `--manifest`): step 4 fails with UK-4001 and exits 1
//! before any execution. Granted run: `RESULT <n>` where n is the demo
//! crate's FNV-1a over the buffer the Austral module handed across.

use std::ffi::{c_void, CStr};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use austral_cranelift_bridge::{
    auth, compile_entry_i64, compile_to_function_named, cranelift_last_error, execute_function,
};
use umbrella::{cargo, cli, frontend, interface, policy, project, provider, trust};

/// U03's JSON event mode: stdout carries one typed event per line and human
/// diagnostics stay on stderr. Set once, before any stage.
static JSON_MODE: OnceLock<bool> = OnceLock::new();

fn json_mode() -> bool {
    *JSON_MODE.get().unwrap_or(&false)
}

fn event(value: serde_json::Value) {
    if json_mode() {
        println!("{value}");
    }
}

fn die(msg: impl std::fmt::Display) -> ! {
    die_with_code(msg, 1)
}

fn die_with_code(msg: impl std::fmt::Display, code: i32) -> ! {
    let text = msg.to_string();
    event(serde_json::json!({
        "event": "error",
        "message": text,
    }));
    eprintln!("umbrella: {text}");
    std::process::exit(code);
}

/// Read a C string out of the bridge's last-error slot, if any.
fn take_bridge_error() -> String {
    let err = cranelift_last_error();
    if err.is_null() {
        "unknown bridge error".to_string()
    } else {
        unsafe { CStr::from_ptr(err) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Fill CLI defaults that depend on the checkout layout. Parsing itself is
/// `umbrella::cli`'s job (U03); this only supplies what the grammar leaves
/// optional.
fn with_defaults(cli: umbrella::cli::Cli) -> umbrella::cli::Cli {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cli = cli;
    if cli.austral.is_none() {
        cli.austral = Some(
            std::env::var_os("AUSTRAL_BIN")
                .map(PathBuf::from)
                // The stock front-end is the dune-built binary at the
                // australVM root (the same one test-programs/runner.py
                // invokes as "./austral"). Since umbrella is a submodule
                // inside australVM, that root is one level up.
                .unwrap_or_else(|| here.join("../austral")),
        );
    }
    if cli.crate_dir.is_none() {
        cli.crate_dir = Some(here.join("modules/rscrate"));
    }
    cli
}

/// 1. cargo build the crate and hand back its cdylib path.
///
/// U05: the artifact is identified, not guessed. `cargo metadata --no-deps`
/// names the package and cdylib target at the crate directory, and only that
/// exact `(package, target)` may match in the build stream — a dependency's
/// cdylib or a proc-macro (also a `.so`) can no longer win by appearing first.
fn cargo_build(cli: &cli::Cli, crate_dir: &PathBuf) -> Result<PathBuf, String> {
    // U05 bullet 2: everything that varies about the build is stated on the
    // command line, and everything that can vary it invisibly is recorded —
    // or refused, in the case of a compiler wrapper.
    let request = cargo::BuildRequest {
        crate_dir: crate_dir.clone(),
        package: cli.cargo_package.clone(),
        target_dir: cli.target_dir.clone(),
        profile: cli.profile,
        features: cli.features.clone(),
        locked: cli.locked,
        offline: cli.offline,
    };
    let vars: Vec<(String, String)> = cargo::BUILD_AFFECTING_VARS
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_string(), value))
        })
        .collect();
    let recorded = cargo::build_environment(vars.iter().map(|(n, v)| (n.as_str(), v.as_str())))?;
    if recorded.is_empty() {
        eprintln!("umbrella: build environment: none of the build-affecting variables are set");
    } else {
        for (name, value) in &recorded {
            eprintln!("umbrella: build environment: {name}={value}");
        }
    }
    for config in cargo::cargo_config_files(crate_dir) {
        eprintln!("umbrella: cargo config in effect: {}", config.display());
    }

    let mut metadata_cmd = Command::new("cargo");
    metadata_cmd
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version=1");
    if request.locked {
        metadata_cmd.arg("--locked");
    }
    if request.offline {
        metadata_cmd.arg("--offline");
    }
    let metadata = metadata_cmd
        .current_dir(crate_dir)
        .output()
        .map_err(|e| format!("spawn cargo metadata: {e}"))?;
    if !metadata.status.success() {
        return Err(format!(
            "cargo metadata failed: {}\n{}",
            metadata.status,
            String::from_utf8_lossy(&metadata.stderr)
        ));
    }
    let wanted = cargo::wanted_from_metadata_for(
        &String::from_utf8_lossy(&metadata.stdout),
        crate_dir,
        request.package.as_deref(),
    )?;
    eprintln!(
        "umbrella: cargo will build package `{}`, target `{}`, profile `{}`",
        wanted.package,
        wanted.target,
        request.profile.as_str()
    );

    let out = Command::new("cargo")
        .args(cargo::build_args(&request))
        .current_dir(crate_dir)
        .output()
        .map_err(|e| format!("spawn cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo build failed: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    cargo::select_shared_artifact(&String::from_utf8_lossy(&out.stdout), &wanted)
}

// 2. `provider::ProviderSet::load` maps the admitted library RTLD_LOCAL and
//    registers its declared exports. The adapter crates link that exact image,
//    so neither provider nor adapter symbols rely on global scope.

/// 3. Stock front-end as a subprocess: the CPS lowering, and nothing else.
///    U07's compile-only mode never initializes the bridge or invokes target
///    code, so the artifact is the whole product. The frontend module owns the
///    private staging, completion checks and hashes; this function only splits
///    the source list into dependency prefixes.
fn front_end(
    austral: &PathBuf,
    modules: &[PathBuf],
    liquid_required: bool,
) -> Result<
    (
        Vec<frontend::Artifact>,
        Vec<interface::BufferAdapter>,
        Vec<interface::HandleAdapter>,
        Vec<interface::Adapter>,
    ),
    String,
> {
    // The front-end emits one CPS binary per run, covering the prefix of the
    // source list it was given. Compile each prefix so every dependency body is
    // in the JIT module as well as the entry module. This assumes the sources
    // are listed in dependency order, which the compiler requires anyway.
    let prefixes: Vec<&[PathBuf]> = if modules.len() <= 1 {
        vec![modules]
    } else {
        (1..=modules.len()).map(|i| &modules[..i]).collect()
    };
    let mut artifacts = Vec::new();
    for prefix in prefixes {
        artifacts.push(frontend::compile(austral, prefix, liquid_required)?);
    }
    let text = artifacts
        .last()
        .map(|a| a.adapters.clone())
        .unwrap_or_default();
    // One document carries both block kinds: the scalar reader skips
    // `abi buffer` blocks and the buffer reader skips `abi scalar` ones, so a
    // module that calls both gets both wrappers.
    let adapters = interface::Adapter::parse_descriptors(&text)
        .map_err(|e| format!("adapter document: {e}"))?;
    let buffers = interface::BufferAdapter::parse_descriptors(&text)
        .map_err(|e| format!("adapter document: {e}"))?;
    // U13: a handle block names an owned-value type; the reader numbers its
    // registries from 1, which is the owner tag the handle words carry.
    let handles = interface::HandleAdapter::parse_descriptors(&text)
        .map_err(|e| format!("adapter document: {e}"))?;
    Ok((artifacts, buffers, handles, adapters))
}

/// 3.5. Generate the wrappers the front-end asked for and load them where the
///      JIT can resolve them. Each adapter becomes one module (the generator
///      writes a file per adapter and each starts with the same `use`), all in
///      one crate. The crate is linked against the admitted provider by absolute
///      path and mapped `RTLD_LOCAL`; its exports are registered by exact
///      address, so neither provider nor adapter identity depends on global
///      scope. Every call is a new build at the next provider generation (see
///      `adapters`), which lets a provider reload build a second crate here.
fn build_adapters(
    adapters: &[interface::GeneratedAdapter<'_>],
    provider: &std::path::Path,
) -> Result<PathBuf, String> {
    let built = umbrella::adapters::build_next_against(adapters, Some(provider))?;
    umbrella::adapters::load_for_jit(&built)?;
    Ok(built.path)
}

/// 4a. Authorizer state: with a manifest, the bridge's manifest engine
///     (stricter-loading path shared with deploy_hosts); without one, an
///     explicit DenyAll so the denied run is deterministic regardless of
///     feature configuration.
fn install_auth(manifest: Option<&PathBuf>) -> Result<Option<auth::ManifestAuthEngine>, String> {
    match manifest {
        Some(p) => {
            let bytes = std::fs::read(p).map_err(|e| format!("read {p:?}: {e}"))?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| format!("manifest {p:?} is not UTF-8: {e}"))?;
            let policy = auth::ManifestAuthEngine::from_toml_str(text)
                .map_err(|e| format!("manifest auth error: {e}"))?;
            let rc = unsafe { auth::safestos_load_auth_manifest(bytes.as_ptr(), bytes.len()) };
            if rc == 1 {
                Ok(Some(policy))
            } else {
                Err(format!("manifest loader rejected {p:?}"))
            }
        }
        None => {
            auth::set_deny_all();
            auth::set_deployment_principal("anon".to_string());
            Ok(None)
        }
    }
}

/// 4b. Compile the CPS with the gate live; a denied extern returns null
///     and the UK-4001 message explains which grant was missing.
///
/// U04: the strict entry path. `execute_function` invokes the pointer as
/// `fn() -> i64`, so the declared contract is "zero parameters, Int64
/// return"; the bridge refuses a module whose entry declares anything else
/// instead of handing back a pointer the caller would miscall.
fn jit_compile_module(cps: &[u8]) -> Result<(), String> {
    let ptr = compile_to_function_named(cps.as_ptr(), cps.len(), std::ptr::null(), 0);
    if ptr.is_null() {
        Err(take_bridge_error())
    } else {
        Ok(())
    }
}

/// The provider exports the project declares. A pinned artifact names its ABI
/// explicitly; either way, a granted `rs_*` symbol that the library really
/// exports is added to the binding, so a preloaded same-named symbol cannot
/// win a granted JIT import. Grants naming the bridge (`uk_*`/`uz_*`) or a
/// generated adapter (`*__adapter`) are not provider exports and are left to
/// their own resolution.
fn provider_exports(
    doc: &project::Project,
    library: &std::path::Path,
) -> Result<Vec<String>, String> {
    let available = trust::exports(library)?;
    let mut declared = match &doc.trusted {
        Some(trusted) => trusted.exports.clone(),
        None => Vec::new(),
    };
    for grant in &doc.grants {
        if !grant.symbol.starts_with("rs_") || grant.symbol.ends_with("__adapter") {
            continue;
        }
        if available.iter().any(|s| s == &grant.symbol) && !declared.contains(&grant.symbol) {
            declared.push(grant.symbol.clone());
        }
    }
    Ok(declared)
}

fn jit_compile(cps: &[u8], entry: &str) -> Result<*const c_void, String> {
    let ptr = compile_entry_i64(cps.as_ptr(), cps.len(), entry.as_ptr(), entry.len(), 0);
    if ptr.is_null() {
        Err(take_bridge_error())
    } else {
        Ok(ptr)
    }
}

fn main() {
    // Usage errors exit 2 and happen before any stage: no cargo, no dlopen,
    // no front-end. `--help`/`--version` exit 0 without touching the checkout.
    let parsed = match cli::parse(std::env::args_os().skip(1)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("umbrella: {e}");
            eprintln!("{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    let _ = JSON_MODE.set(parsed.cli.json);
    match parsed.action {
        cli::Action::Help => {
            println!("{}", cli::USAGE);
            return;
        }
        cli::Action::Version => {
            println!("umbrella {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        cli::Action::Run => {}
    }
    let cli = with_defaults(parsed.cli);
    let austral = cli.austral.clone().expect("default front-end path");
    let default_crate_dir = cli.crate_dir.clone().expect("default crate dir");

    // 0. U02: a project document is validated and preflighted before any
    //    stage runs. A refusal here means cargo, dlopen and the front-end are
    //    never reached — the order the plan requires.
    let planned = match &cli.project {
        Some(path) => {
            let doc = match project::load(path) {
                Ok(d) => d,
                Err(e) => die(e),
            };
            let pf = match policy::preflight(&doc) {
                Ok(p) => p,
                Err(e) => die(e),
            };
            eprintln!("{}", pf.report());
            Some((doc, pf))
        }
        None => None,
    };
    let modules: Vec<PathBuf> = if cli.modules.is_empty() {
        planned
            .as_ref()
            .map(|(d, _)| d.module.sources.clone())
            .unwrap_or_default()
    } else {
        cli.modules.clone()
    };

    let build_allowed = planned
        .as_ref()
        .map(|(_, pf)| pf.build.allowed())
        .unwrap_or(true);
    let load_allowed = planned
        .as_ref()
        .map(|(_, pf)| pf.load.allowed())
        .unwrap_or(true);

    // 1. cargo subprocess → artifact path (only when build is authorized).
    let cdylib = if build_allowed {
        let krate = planned
            .as_ref()
            .and_then(|(d, _)| d.build.krate.clone())
            .unwrap_or_else(|| default_crate_dir.clone());
        // D09: the build is only attempted for the pinned dependency graph and
        // feature set. A mismatch is a refusal *here*, with no cargo process
        // started — building an arbitrary crate is executing its build scripts.
        if let Some((doc, _)) = &planned {
            let mut doc = doc.clone();
            doc.requested_features = cli.features.clone();
            if let Err(e) = trust::verify_before_build(&doc, &krate) {
                die(e);
            }
        }
        match cargo_build(&cli, &krate) {
            Ok(p) => p,
            Err(e) => die(e),
        }
    } else {
        match planned
            .as_ref()
            .and_then(|(d, _)| d.load.providers.first().cloned())
        {
            Some(p) => {
                eprintln!(
                    "umbrella: build not authorized; using prebuilt provider {}",
                    p.display()
                );
                p
            }
            None => die("build is not authorized and no prebuilt provider is declared"),
        }
    };
    if build_allowed {
        eprintln!("umbrella: built {}", cdylib.display());
    } else {
        eprintln!("umbrella: build not authorized; using {}", cdylib.display());
    }

    // 2. explicit provider admission → declared exports bound to the exact
    //    admitted image and registered before the JIT initializes.
    if !load_allowed {
        die(format!(
            "load is not authorized by the project; refusing to map {} \
             (any build above already ran)",
            cdylib.display()
        ));
    }
    // D09: the bytes that are about to be mapped must be the pinned artifact,
    // and it must define the pinned ABI exports. This runs after the build (the
    // digest is of what was produced) and before mapping (the initializer runs
    // on load).
    if let Some((doc, _)) = &planned {
        if let Err(e) = trust::verify_before_load(doc, &cdylib) {
            die(e);
        }
    }
    // `ProviderSet` is retained to the end of `main`: the JIT bakes the
    // registered addresses into code, so the mapping must outlive execution.
    let mut _provider: Option<provider::ProviderSet> = None;
    match &planned {
        Some((doc, _)) => {
            let providers = if build_allowed {
                vec![cdylib.clone()]
            } else {
                doc.load.providers.clone()
            };
            if providers.len() != 1 {
                die(format!(
                    "the project declares {} provider libraries; this profile pins one \
                     artifact identity per run, so a multi-provider binding needs the \
                     per-provider ABI records U10 adds",
                    providers.len()
                ));
            }
            let declared = match provider_exports(doc, &providers[0]) {
                Ok(exports) => exports,
                Err(e) => die(e),
            };
            if declared.is_empty() {
                die(
                    "the project declares no provider exports; refusing to map a library \
                     whose granted identity cannot be named"
                        .to_string(),
                );
            }
            match provider::ProviderSet::load(&providers, &declared) {
                Ok(set) => {
                    let names: Vec<&str> = set.exports().iter().map(|(n, _)| n.as_str()).collect();
                    eprintln!(
                        "umbrella: admitted provider {} with {} declared export(s): {}",
                        providers[0].display(),
                        names.len(),
                        names.join(", ")
                    );
                    _provider = Some(set);
                }
                Err(e) => die(e),
            }
        }
        // No project document means no declared provider identity. Keep the
        // prototype's global mapping for that explicit DenyAll path; there is
        // nothing to bind by name.
        None => {
            if let Err(e) = policy::dlopen(&cdylib) {
                die(e);
            }
            eprintln!(
                "umbrella: mapped {} (RTLD_NOW|RTLD_GLOBAL; no declared exports)",
                cdylib.display()
            );
        }
    }

    // A denied call is a separate decision: build and load already ran, and
    // the report above says so instead of claiming containment.
    if let Some((_, pf)) = &planned {
        if !pf.call.allowed() {
            eprintln!("umbrella: {}", pf.call.reason());
            die_with_code("call is not authorized; refusing to invoke the entry", 2);
        }
    }

    // D18: a manifest's `[verify] liquid = "required"` is a compiler input,
    // not just host metadata. The CLI flag can also request it (and is unioned
    // with the manifest, never used to downgrade it).
    let liquid_required = cli.liquid_required
        || match cli.manifest.as_ref() {
            Some(path) => match frontend::manifest_requires_liquid(path) {
                Ok(required) => required,
                Err(e) => die(e),
            },
            None => false,
        };

    // 3. front-end subprocess → a checked compile-only artifact and the
    //    adapter descriptors. The staged outputs are gone by the time this
    //    returns; only the verified bytes and hash remain.
    let (artifacts, buffers, handles, scalars) =
        match front_end(&austral, &modules, liquid_required) {
            Ok(b) => b,
            Err(e) => die(e),
        };
    let cps_bytes: usize = artifacts.iter().map(|a| a.cps.len()).sum();
    let functions: usize = artifacts.iter().map(|a| a.functions).sum();
    let cps_sha256 = frontend::digest(&artifacts);
    let module_names: Vec<&str> = artifacts
        .iter()
        .flat_map(|a| a.module_names.iter().map(String::as_str))
        .collect();
    let mut exports_by_name = std::collections::BTreeMap::new();
    for export in artifacts.iter().flat_map(|a| a.exports.iter()) {
        exports_by_name.insert(export.name.clone(), export);
    }
    let typed_exports: Vec<frontend::Export> = exports_by_name
        .values()
        .map(|export| (*export).clone())
        .collect();
    let exports: Vec<serde_json::Value> = typed_exports
        .iter()
        .map(|export| {
            serde_json::json!({
                "name": export.name,
                "params": export.params,
                "returns": export.returns,
            })
        })
        .collect();
    // The descriptor document is the contract the compiler emitted, so the
    // event carries both the digest a project can pin and each block's
    // canonical text. U10 checks the pin before any wrapper is generated.
    let descriptor_document = artifacts
        .last()
        .map(|a| a.adapters.clone())
        .unwrap_or_default();
    let descriptors: Vec<serde_json::Value> = scalars
        .iter()
        .map(|a| {
            serde_json::json!({
                "kind": "scalar",
                "symbol": a.symbol,
                "descriptor": a.descriptor_text(),
            })
        })
        .chain(buffers.iter().map(|a| {
            serde_json::json!({
                "kind": "buffer",
                "symbol": a.symbol,
                "descriptor": a.descriptor_text(),
            })
        }))
        .chain(handles.iter().map(|a| {
            serde_json::json!({
                "kind": "handle",
                "symbol": a.symbol,
                "descriptor": a.descriptor_text(),
            })
        }))
        .collect();
    let descriptors_sha256 = trust::sha256_bytes(descriptor_document.as_bytes());
    if let Some((doc, _)) = &planned {
        if let Err(e) = trust::verify_artifact(doc, &typed_exports, descriptor_document.as_bytes())
        {
            die(e);
        }
    }
    event(serde_json::json!({
        "event": "artifact",
        "modules": artifacts.iter().map(|a| a.modules.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "module_names": module_names,
        "exports": exports,
        "adapters": descriptors,
        "adapters_sha256": descriptors_sha256,
        "functions": functions,
        "cps_bytes": cps_bytes,
        "cps_sha256": cps_sha256,
        "liquid": match artifacts.first().map(|a| a.liquid) {
            Some(frontend::Liquid::Required) => "required",
            _ => "optional",
        },
    }));
    eprintln!(
        "umbrella: front-end emitted {} module(s), {functions} function(s), \
         {cps_bytes} bytes of CPS (sha256 {cps_sha256})",
        artifacts.len()
    );
    let cps_binaries: Vec<Vec<u8>> = artifacts.into_iter().map(|a| a.cps).collect();

    // 3.5. The wrappers the descriptors ask for. Empty is a real answer: every
    //      call in the module was word-native and needs no mediation. Buffer
    //      adapters are rendered into the same crate as the scalar ones — the
    //      JIT resolves `<name>__adapter` whichever block produced it.
    let adapters: Vec<interface::GeneratedAdapter<'_>> = scalars
        .iter()
        .map(interface::GeneratedAdapter::Scalar)
        .chain(buffers.iter().map(interface::GeneratedAdapter::Buffer))
        .chain(handles.iter().map(interface::GeneratedAdapter::Handle))
        .collect();
    if adapters.is_empty() {
        eprintln!("umbrella: the module needs no adapters");
    } else {
        let names: Vec<&str> = adapters.iter().map(|a| a.symbol()).collect();
        match build_adapters(&adapters, &cdylib) {
            Ok(path) => eprintln!(
                "umbrella: built and loaded {} adapter wrapper(s) ({}) from {}",
                adapters.len(),
                names.join(", "),
                path.display()
            ),
            Err(e) => die(e),
        }
    }

    // 4. in-process gate + compile + execute. U09: every run is bound to one
    //    immutable instance whose own policy snapshot is the authority; the
    //    global manifest install above remains for legacy callers and capstd.
    let policy = match install_auth(cli.manifest.as_ref()) {
        Ok(Some(policy)) => policy,
        Ok(None) => auth::ManifestAuthEngine::new(),
        Err(e) => die(e),
    };
    let module_name = planned
        .as_ref()
        .map(|(doc, _)| doc.module.name.clone())
        .or_else(|| policy.primary_principal().map(str::to_string))
        .unwrap_or_else(|| "anon".to_string());
    let node = planned
        .as_ref()
        .and_then(|(doc, _)| doc.capabilities.as_ref())
        .and_then(|caps| caps.peers.first().cloned());
    let user = planned
        .as_ref()
        .and_then(|(doc, _)| doc.identity.as_ref())
        .map(|identity| identity.user.clone());
    let instance = auth::bind_instance_with_identity(module_name, false, user, node, policy);
    let _scope = auth::enter_caller(&instance);
    // Every module but the entry one is compiled for its definitions only; the
    // entry binary is compiled last through the strict entry path. They share
    // the process-global JIT module, so cross-module calls resolve by name.
    let (entry_cps, dependencies) = match cps_binaries.split_last() {
        Some((entry, rest)) => (entry, rest),
        None => die("the front-end emitted no CPS".to_string()),
    };
    for cps in dependencies {
        if let Err(e) = jit_compile_module(cps) {
            die(e);
        }
    }
    if !dependencies.is_empty() {
        eprintln!(
            "umbrella: compiled {} dependency module(s) into the JIT",
            dependencies.len()
        );
    }
    let f = match jit_compile(entry_cps, &cli.entry) {
        Ok(p) => p,
        Err(e) => die(e),
    };
    eprintln!("umbrella: entry `{}` compiled, executing", cli.entry);
    // D09: capabilities are constructed from the document, not from the shell's
    // ambient access. A relative `state_dir` is resolved against the project
    // document's own directory, so a checkout with a different layout cannot
    // silently root the instance somewhere else.
    if let Some((doc, _)) = &planned {
        if let Some(caps) = &doc.capabilities {
            let project_dir = cli
                .project
                .as_ref()
                .and_then(|p| p.parent())
                .unwrap_or_else(|| std::path::Path::new("."));
            let state_dir = if caps.state_dir.is_absolute() {
                caps.state_dir.clone()
            } else {
                project_dir.join(&caps.state_dir)
            };
            match austral_cranelift_bridge::capstd_mod::caps::InstanceCaps::new(
                doc.module.name.clone(),
                &state_dir,
                caps.peers.clone(),
            ) {
                Ok(set) => {
                    austral_cranelift_bridge::capstd_mod::caps::install(instance.instance, set);
                    eprintln!(
                        "umbrella: instance {} at {} with peers {:?}",
                        doc.module.name,
                        state_dir.display(),
                        caps.peers
                    );
                    // The compiled entry runs inside this identity, so every gated
                    // call and every capability verb is attributed to *this*
                    // instance rather than to whatever loaded last. The caller
                    // scope was entered before compilation; it is still active.
                    let result = execute_function(f);
                    if json_mode() {
                        event(serde_json::json!({"event": "done", "result": result}));
                    } else {
                        println!("RESULT {result}");
                    }
                    if let Some(want) = cli.expect {
                        if result != want {
                            die(format!("expected {want}, got {result}"));
                        }
                        eprintln!("umbrella: matches --expect {want}");
                    }
                    return;
                }
                Err(e) => die(format!("cannot construct capabilities: {e}")),
            }
        }
    }
    let result = execute_function(f);
    if json_mode() {
        event(serde_json::json!({"event": "done", "result": result}));
    } else {
        println!("RESULT {result}");
    }
    if let Some(want) = cli.expect {
        if result != want {
            die(format!("expected {want}, got {result}"));
        }
        eprintln!("umbrella: matches --expect {want}");
    }
}
