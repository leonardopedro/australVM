//! `umbrella` — PLAN_valen §5 P3.
//!
//! A host project that lives OUTSIDE australVM and drives the whole
//! extern-rust pipeline end to end by consuming australVM as (a) a
//! library and (b) a front-end subprocess:
//!
//! 1. **cargo** builds an arbitrary crate (`modules/rscrate`) to a
//!    cdylib — the `--message-format=json` stream yields the artifact
//!    path, so no target-layout assumption is baked in;
//! 2. **dlopen(RTLD_NOW | RTLD_GLOBAL)** registers that artifact in the
//!    process; the bridge's default symbol lookup (dlsym RTLD_DEFAULT)
//!    finds the crate's `rs_*` exports with zero australVM changes;
//! 3. the stock **`austral` binary** (`compile ... --use-cps-jit
//!    --allow-all --target-type=tc --emit-cps=...`, the same invocation
//!    modhost's spawn_jit_server uses) serializes the module to CPS —
//!    australVM never learns about the crate that was just built;
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

use austral_cranelift_bridge::{auth, compile_entry_i64, cranelift_last_error, execute_function};
use umbrella::{cargo, cli, policy, project};

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

// 2. `policy::dlopen` is the same RTLD_NOW|RTLD_GLOBAL mapping described in
//    the module docs; it lives in the library so the stage test can map a
//    fixture library through exactly the code the driver uses.

/// 3. Stock front-end as a subprocess: exactly the invocation modhost's
///    spawn_jit_server uses, minus the JIT-server transport, plus
///    --emit-cps so we get the CPS binary on disk. No umbrella-specific
///    code exists (or may exist) on the australVM side.
fn front_end(austral: &PathBuf, modules: &[PathBuf], cps_out: &PathBuf) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(austral);
    // The dune-built front-end links libaustral_cranelift_bridge.so
    // dynamically; Makefile targets and the nix dev shell carry it via
    // LD_LIBRARY_PATH. A bare caller of the umbrella must not have to
    // reproduce that environment, so prepend the bridge's cargo output
    // dir when it is sitting next to the austral checkout (default
    // layout).
    if let Some(dir) = austral.parent() {
        let bridge_dir = dir.join("safestos/cranelift/target/release");
        if bridge_dir.join("libaustral_cranelift_bridge.so").exists() {
            let mut lp = bridge_dir.display().to_string();
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                lp.push(':');
                lp.push_str(&old.to_string_lossy());
            }
            cmd.env("LD_LIBRARY_PATH", lp);
        }
    }
    cmd.arg("compile");
    for m in modules {
        cmd.arg(m);
    }
    cmd.arg("--use-cps-jit")
        .arg("--allow-all")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cps_out.display()));
    let out = cmd
        .output()
        .map_err(|e| format!("spawn {austral:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "austral front-end failed: {}\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    std::fs::read(cps_out).map_err(|e| format!("read {}: {e}", cps_out.display()))
}

/// 4a. Authorizer state: with a manifest, the bridge's manifest engine
///     (stricter-loading path shared with deploy_hosts); without one, an
///     explicit DenyAll so the denied run is deterministic regardless of
///     feature configuration.
fn install_auth(manifest: Option<&PathBuf>) -> Result<(), String> {
    match manifest {
        Some(p) => {
            let bytes = std::fs::read(p).map_err(|e| format!("read {p:?}: {e}"))?;
            let rc = unsafe { auth::safestos_load_auth_manifest(bytes.as_ptr(), bytes.len()) };
            if rc == 1 {
                Ok(())
            } else {
                Err(format!("manifest loader rejected {p:?}"))
            }
        }
        None => {
            auth::set_deny_all();
            auth::set_deployment_principal("anon".to_string());
            Ok(())
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

    // 2. dlopen → symbols reachable via the bridge's dlsym fallback.
    if !load_allowed {
        die(format!(
            "load is not authorized by the project; refusing to map {} \
             (any build above already ran)",
            cdylib.display()
        ));
    }
    if let Err(e) = policy::dlopen(&cdylib) {
        die(e);
    }
    eprintln!(
        "umbrella: dlopen'd {} (RTLD_NOW|RTLD_GLOBAL)",
        cdylib.display()
    );

    // A denied call is a separate decision: build and load already ran, and
    // the report above says so instead of claiming containment.
    if let Some((_, pf)) = &planned {
        if !pf.call.allowed() {
            eprintln!("umbrella: {}", pf.call.reason());
            die_with_code("call is not authorized; refusing to invoke the entry", 2);
        }
    }

    // 3. front-end subprocess → CPS bytes.
    let cps_out: PathBuf =
        std::env::temp_dir().join(format!("umbrella-{}.cps", std::process::id()));
    let cps = match front_end(&austral, &modules, &cps_out) {
        Ok(b) => {
            let _ = std::fs::remove_file(&cps_out);
            b
        }
        Err(e) => die(e),
    };
    eprintln!("umbrella: front-end emitted {} bytes of CPS", cps.len());

    // 4. in-process gate + compile + execute.
    if let Err(e) = install_auth(cli.manifest.as_ref()) {
        die(e);
    }
    let f = match jit_compile(&cps, &cli.entry) {
        Ok(p) => p,
        Err(e) => die(e),
    };
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
