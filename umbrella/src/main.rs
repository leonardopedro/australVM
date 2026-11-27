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
use umbrella::{cache, cargo, cli, frontend, interface, policy, project, provider, trust};

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
    // U18: a project document makes its own directory the cache home, so a
    // checkout warms the same artifact across runs without a flag. `--cache-dir`
    // still wins, and a run with no project document keeps the cache off.
    if cli.cache_dir.is_none() {
        if let Some(project) = &cli.project {
            let base = project
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            cli.cache_dir = Some(base.join(".umbrella-cache"));
        }
    }
    cli
}

/// Render the provider wrappers a compiler-emitted descriptor document asks
/// for. The document is the text `austral compile --emit-adapters` writes, so
/// this path never needs the front-end, cargo or the JIT: it parses the same
/// blocks the build path parses and writes the same combined `mod wN { … }`
/// crate source `adapters::build_at` renders.
fn render_wrapper_document(
    document: &std::path::Path,
    dir: &std::path::Path,
) -> Result<(usize, PathBuf), String> {
    let text = std::fs::read_to_string(document)
        .map_err(|e| format!("cannot read descriptor document {}: {e}", document.display()))?;
    let scalars = interface::Adapter::parse_descriptors(&text)?;
    let buffers = interface::BufferAdapter::parse_descriptors(&text)?;
    let handles = interface::HandleAdapter::parse_descriptors(&text)?;
    let adapters: Vec<interface::GeneratedAdapter<'_>> = scalars
        .iter()
        .map(interface::GeneratedAdapter::Scalar)
        .chain(buffers.iter().map(interface::GeneratedAdapter::Buffer))
        .chain(handles.iter().map(interface::GeneratedAdapter::Handle))
        .collect();
    if adapters.is_empty() {
        return Err(format!(
            "{} contains no adapter blocks (`abi scalar`, `abi buffer` or `abi handle`)",
            document.display()
        ));
    }
    let mut source = String::new();
    for (index, adapter) in adapters.iter().enumerate() {
        let wrapper = adapter
            .render()
            .map_err(|e| format!("render {}: {e}", adapter.symbol()))?;
        source.push_str(&format!("mod w{index} {{\n{wrapper}\n}}\n"));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join("wrappers.rs");
    std::fs::write(&file, source).map_err(|e| format!("write {}: {e}", file.display()))?;
    Ok((adapters.len(), file))
}

/// 1. cargo build the crate and hand back its cdylib path.
///
/// U05: the artifact is identified, not guessed. `cargo metadata --no-deps`
/// names the package and cdylib target at the crate directory, and only that
/// exact `(package, target)` may match in the build stream — a dependency's
/// cdylib or a proc-macro (also a `.so`) can no longer win by appearing first.
/// 1. The provider build, through the run's cache when one is enabled. A hit
/// skips cargo entirely; a miss builds and stores a copy, and the *copy* is
/// what the rest of the run maps and links so one path is shared by the
/// provider, the adapter crate and the trust checks.
fn provider_build_cached(
    request: &cargo::BuildRequest,
    cache: Option<(&std::path::Path, &str)>,
) -> Result<(PathBuf, bool), String> {
    let Some((root, key)) = cache else {
        return cargo_build(request).map(|path| (path, false));
    };
    match cache::load_provider(root, key) {
        Ok(Some(path)) => return Ok((path, true)),
        Ok(None) => {}
        Err(reason) => {
            eprintln!("umbrella: provider cache entry refused: {reason}");
            cache::invalidate_provider(root, key)?;
        }
    }
    let built = cargo_build(request)?;
    let cached = cache::store_provider(root, key, &built)?;
    Ok((cached, false))
}

fn cargo_build(request: &cargo::BuildRequest) -> Result<PathBuf, String> {
    let crate_dir = &request.crate_dir;
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
    cache: Option<(&std::path::Path, &str)>,
) -> Result<
    (
        Vec<frontend::Artifact>,
        Vec<interface::BufferAdapter>,
        Vec<interface::HandleAdapter>,
        Vec<interface::Adapter>,
        cache::Outcome,
    ),
    String,
> {
    // U17: one compile over the whole list. `--emit-cps-bundle` makes the
    // compiler write a single artifact holding every dependency's CPS as well as
    // the entry module's, instead of one artifact per dependency prefix (each
    // of which truncated the path). The list no longer has to be typed in
    // dependency order: the imports are read from the sources, the list is
    // refused when no order can exist (a duplicate name, a self-import, a
    // cycle) and otherwise built leaf-first. The last module built is the
    // bundle's namesake, and a client is not imported by its dependencies, so
    // the client still comes last however the arguments were ordered.
    let sources = frontend::read_module_sources(modules)?;
    frontend::check_dependency_order(&sources)?;
    let ordered: Vec<PathBuf> = frontend::dependency_order(&sources)?
        .into_iter()
        .map(|index| modules[index].clone())
        .collect();
    // U18: the artifact is content-addressed by the derived order, every source
    // file, the compiler bytes and the lowering flags, so a warm run reuses the
    // bytes without spawning the compiler. The hit is bytes, not an admission:
    // the descriptor/export pins and the JIT call gate run below either way.
    let (artifact, cache_outcome) = match cache {
        Some((root, key)) => {
            cache::compile_cached_keyed(austral, &ordered, liquid_required, root, key)?
        }
        None => cache::compile_cached(austral, &ordered, liquid_required, None)?,
    };
    let artifacts = vec![artifact];
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
    Ok((artifacts, buffers, handles, adapters, cache_outcome))
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
    cache: Option<(&std::path::Path, &str)>,
) -> Result<PathBuf, String> {
    let built = umbrella::adapters::build_cached(adapters, cache, Some(provider))?;
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

    // U21 bounded start: a descriptor document on its own is enough to render
    // the provider wrappers. No cargo, no front-end, no dlopen.
    if let Some(document) = &cli.descriptor_document {
        let dir = cli
            .emit_wrappers
            .as_ref()
            .expect("`--descriptor-document` is paired with `--emit-wrappers`");
        match render_wrapper_document(document, dir) {
            Ok((count, file)) => {
                event(serde_json::json!({
                    "event": "wrappers",
                    "count": count,
                    "path": file.display().to_string(),
                }));
                eprintln!(
                    "umbrella: rendered {count} provider wrapper(s) from {} to {}",
                    document.display(),
                    file.display()
                );
                return;
            }
            Err(e) => die(e),
        }
    }

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

    // D18: a manifest's `[verify] liquid = "required"` is a compiler input,
    // not just host metadata. It is decided before the cache key so a warm run
    // cannot answer for a different lowering mode. A manifest that cannot be
    // read is still an error *after* the build stage, which is where the run
    // reported it before the cache existed; the key uses the CLI mode until
    // then, and no front-end entry is written because the run stops first.
    let (liquid_required, liquid_error) = match cli.manifest.as_ref() {
        Some(path) => match frontend::manifest_requires_liquid(path) {
            Ok(required) => (cli.liquid_required || required, None),
            Err(e) => (cli.liquid_required, Some(e)),
        },
        None => (cli.liquid_required, None),
    };

    // U05: describe the build explicitly instead of reading it from the
    // environment or hardcoding it. The request and the recorded environment
    // are also part of the cache key, so a changed profile, feature set or
    // build-affecting variable is a different key.
    let krate = planned
        .as_ref()
        .and_then(|(d, _)| d.build.krate.clone())
        .unwrap_or_else(|| default_crate_dir.clone());
    let build_request = cargo::BuildRequest {
        crate_dir: krate.clone(),
        package: cli.cargo_package.clone(),
        target_dir: cli.target_dir.clone(),
        profile: cli.profile,
        features: cli.features.clone(),
        locked: cli.locked,
        offline: cli.offline,
    };
    let build_env = if build_allowed {
        let vars: Vec<(String, String)> = cargo::BUILD_AFFECTING_VARS
            .iter()
            .filter_map(|name| {
                std::env::var(name)
                    .ok()
                    .map(|value| (name.to_string(), value))
            })
            .collect();
        let recorded = match cargo::build_environment(
            vars.iter().map(|(n, v)| (n.as_str(), v.as_str())),
        ) {
            Ok(recorded) => recorded,
            Err(e) => die(e),
        };
        if recorded.is_empty() {
            eprintln!("umbrella: build environment: none of the build-affecting variables are set");
        } else {
            for (name, value) in &recorded {
                eprintln!("umbrella: build environment: {name}={value}");
            }
        }
        for config in cargo::cargo_config_files(&krate) {
            eprintln!("umbrella: cargo config in effect: {}", config.display());
        }
        recorded
    } else {
        Vec::new()
    };

    // U18: one key for this run's derived artifacts. The provider half is only
    // part of it when a build is authorized, because only then is a provider
    // build cached; a prebuilt provider is identified by the adapter digest.
    let cache_root = cli.cache_dir.clone();
    let provider_inputs = if build_allowed {
        Some(cache::ProviderInputs {
            request: build_request.clone(),
            env: build_env.clone(),
            skip: cache_root.iter().cloned().collect(),
        })
    } else {
        None
    };
    let run_key = match &cache_root {
        Some(_) => match cache::key(&cache::KeyInputs {
            austral: &austral,
            modules: &modules,
            liquid_required,
            provider: provider_inputs.as_ref(),
        }) {
            Ok(key) => Some(key),
            Err(e) => die(e),
        },
        None => None,
    };
    let cache_pair = match (&cache_root, &run_key) {
        (Some(root), Some(key)) => Some((root.as_path(), key.as_str())),
        _ => None,
    };

    // 1. cargo subprocess → artifact path (only when build is authorized).
    let (cdylib, provider_cached) = if build_allowed {
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
        match provider_build_cached(&build_request, cache_pair) {
            Ok(pair) => pair,
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
                (p, false)
            }
            None => die("build is not authorized and no prebuilt provider is declared"),
        }
    };
    if build_allowed {
        if provider_cached {
            eprintln!("umbrella: provider cache hit; using {}", cdylib.display());
        } else {
            eprintln!("umbrella: built {}", cdylib.display());
        }
    } else {
        eprintln!("umbrella: build not authorized; using {}", cdylib.display());
    }

    // 2. explicit provider admission → declared exports bound to the exact
    //    admitted image and registered before the JIT initializes. The identity
    //    checks run here, but the mapping itself waits for step 3's typed
    //    contract check: a tampered signature or descriptor must never cause a
    //    provider initializer to run.
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
    // What step 2b will map, decided here so every refusal that does not need
    // the artifact still happens before the front-end runs.
    let admission: Option<(Vec<PathBuf>, Vec<String>)> = match &planned {
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
            Some((providers, declared))
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
            None
        }
    };

    // A denied call is a separate decision: build and load already ran, and
    // the report above says so instead of claiming containment.
    if let Some((_, pf)) = &planned {
        if !pf.call.allowed() {
            eprintln!("umbrella: {}", pf.call.reason());
            die_with_code("call is not authorized; refusing to invoke the entry", 2);
        }
    }

    if let Some(e) = liquid_error {
        die(e);
    }

    // 3. front-end subprocess → a checked compile-only artifact and the
    //    adapter descriptors. The staged outputs are gone by the time this
    //    returns; only the verified bytes and hash remain.
    let (artifacts, buffers, handles, scalars, cache_outcome) =
        match front_end(&austral, &modules, liquid_required, cache_pair) {
            Ok(b) => b,
            Err(e) => die(e),
        };
    match &cache_outcome {
        cache::Outcome::Disabled => {}
        cache::Outcome::Hit(key) => {
            eprintln!("umbrella: front-end cache hit ({key}); the compiler did not run")
        }
        cache::Outcome::Miss(key) => {
            eprintln!("umbrella: front-end cache miss ({key}); stored the artifact")
        }
        cache::Outcome::Invalid(_, reason) => {
            eprintln!("umbrella: front-end cache entry refused: {reason}")
        }
    }
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
                "abi_version": a.abi_version,
                "symbol": a.symbol,
                "descriptor": a.descriptor_text(),
            })
        })
        .chain(buffers.iter().map(|a| {
            serde_json::json!({
                "kind": "buffer",
                "abi_version": a.abi_version,
                "symbol": a.symbol,
                "descriptor": a.descriptor_text(),
            })
        }))
        .chain(handles.iter().map(|a| {
            serde_json::json!({
                "kind": "handle",
                "abi_version": a.abi_version,
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

    // U10: the pins this admitted artifact implies, written where a caller can
    // paste them into the project document. Emitted only after the check above,
    // so the record describes bytes that passed their trust gate.
    if let Some(path) = &cli.emit_trust_record {
        let record = trust::trust_record(&typed_exports, descriptor_document.as_bytes());
        let text = match serde_json::to_string_pretty(&record) {
            Ok(text) => text,
            Err(e) => die(format!("cannot render the trust record: {e}")),
        };
        if let Err(e) = std::fs::write(path, format!("{text}\n")) {
            die(format!(
                "cannot write the trust record {}: {e}",
                path.display()
            ));
        }
        eprintln!(
            "umbrella: wrote trust record for {} export(s) to {}",
            typed_exports.len(),
            path.display()
        );
    }

    // 2b. The typed contract is admitted above, so only now may the provider be
    //     mapped: its initializer must not run for a contract the project has
    //     not approved. `ProviderSet` is retained to the end of `main`: the JIT
    //     bakes the registered addresses into code, so the mapping must outlive
    //     execution.
    let mut _provider: Option<provider::ProviderSet> = None;
    if let Some((providers, declared)) = admission {
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
    event(serde_json::json!({
        "event": "artifact",
        "modules": artifacts.iter().map(|a| a.modules.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "module_names": module_names,
        "exports": exports,
        "adapters": descriptors,
        "adapters_sha256": descriptors_sha256,
        "functions": functions,
        "cache": cache_outcome.label(),
        "cache_key": cache_outcome.key(),
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
        match build_adapters(&adapters, &cdylib, cache_pair) {
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
            // U19: the document's rights and roots are the capability set's
            // authority. A malformed right is refused here, before anything is
            // installed, rather than silently becoming a wider grant.
            let state_rights = match caps.state_rights() {
                Ok(rights) => rights,
                Err(e) => die(format!("cannot construct capabilities: {e}")),
            };
            let roots = match caps.root_grants() {
                Ok(roots) => roots,
                Err(e) => die(format!("cannot construct capabilities: {e}")),
            };
            // U20: the document's network block becomes the dial policy. It is
            // attached before installation, so the instance never exists for a
            // moment with no policy and therefore no network authority.
            let net_policy = match caps.net_policy(project_dir) {
                Ok(policy) => policy,
                Err(e) => die(format!("cannot construct capabilities: {e}")),
            };
            match austral_cranelift_bridge::capstd_mod::caps::InstanceCaps::new_with_grants(
                doc.module.name.clone(),
                &state_dir,
                caps.peers.clone(),
                state_rights,
                roots,
                caps.quota_bytes,
            ) {
                Ok(set) => {
                    let set = match net_policy {
                        Some(policy) => set.with_net_policy(policy),
                        None => set,
                    };
                    austral_cranelift_bridge::capstd_mod::caps::install(instance.instance, set);
                    let network = caps
                        .network
                        .as_ref()
                        .map(|net| {
                            format!(
                                "{} endpoint(s) over {}, {}",
                                net.endpoints.len(),
                                net.allow_ips.join(","),
                                if net.ca.is_some() {
                                    "TLS anchor granted"
                                } else {
                                    "no TLS anchor"
                                }
                            )
                        })
                        .unwrap_or_else(|| "none".to_string());
                    eprintln!(
                        "umbrella: instance {} at {} with state rights [{}], {} root(s), \
                         quota {:?}, peers {:?} and network {}",
                        doc.module.name,
                        state_dir.display(),
                        state_rights,
                        caps.roots.len(),
                        caps.quota_bytes,
                        caps.peers,
                        network
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
