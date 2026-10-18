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

use std::ffi::{c_void, CStr, CString};
use std::path::PathBuf;
use std::process::Command;

use austral_cranelift_bridge::{
    auth, compile_to_function_named, cranelift_last_error, execute_function,
};

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("umbrella: {msg}");
    std::process::exit(1);
}

/// Read a C string out of the bridge's last-error slot, if any.
fn take_bridge_error() -> String {
    let err = cranelift_last_error();
    if err.is_null() {
        "unknown bridge error".to_string()
    } else {
        unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned()
    }
}

struct Cli {
    modules: Vec<PathBuf>,
    manifest: Option<PathBuf>,
    entry: String,
    austral: PathBuf,
    crate_dir: PathBuf,
    expect: Option<i64>,
}

/// Minimal hand-rolled flag parser: `--k v` / `--k=v`, positionals are
/// module sources. Deliberately not a dependency — the umbrella's whole
/// point is to stay legible as a prototype.
fn parse_cli() -> Cli {
    /// `--flag` values may arrive as `--flag=value` or `--flag value`.
    fn value(args: &[String], i: &mut usize, flag: &str) -> Option<String> {
        let a = &args[*i];
        if let Some(rest) = a.strip_prefix(&format!("{flag}=")) {
            return Some(rest.to_string());
        }
        let v = args.get(*i + 1).cloned();
        if v.is_some() {
            *i += 1;
        }
        v
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        die("usage: umbrella [--manifest <toml>] [--entry <name>] [--austral <bin>] [--cratedir <dir>] [--expect <i64>] <module.aum>...");
    }
    let mut modules = Vec::new();
    let mut manifest = None;
    let mut entry = "compute".to_string();
    let mut austral = None;
    let mut crate_dir = None;
    let mut expect = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--manifest" | _ if a.starts_with("--manifest") => {
                match value(&args, &mut i, "--manifest") {
                    Some(v) => manifest = Some(PathBuf::from(v)),
                    None => die("--manifest needs a path"),
                }
            }
            "--entry" | _ if a.starts_with("--entry") => match value(&args, &mut i, "--entry") {
                Some(v) => entry = v,
                None => die("--entry needs a name"),
            },
            "--austral" | _ if a.starts_with("--austral") => {
                match value(&args, &mut i, "--austral") {
                    Some(v) => austral = Some(PathBuf::from(v)),
                    None => die("--austral needs a path"),
                }
            }
            "--cratedir" | _ if a.starts_with("--cratedir") => {
                match value(&args, &mut i, "--cratedir") {
                    Some(v) => crate_dir = Some(PathBuf::from(v)),
                    None => die("--cratedir needs a path"),
                }
            }
            "--expect" | _ if a.starts_with("--expect") => match value(&args, &mut i, "--expect") {
                Some(v) => match v.parse::<i64>() {
                    Ok(n) => expect = Some(n),
                    Err(_) => die(format!("--expect wants an i64, got {v}")),
                },
                None => die("--expect needs a number"),
            },
            _ if !a.starts_with("--") => modules.push(PathBuf::from(a)),
            _ => die(format!("unknown flag: {a}")),
        }
        i += 1;
    }
    if modules.is_empty() {
        die("at least one .aum module source is required");
    }

    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Cli {
        modules,
        manifest,
        entry,
        austral: austral.unwrap_or_else(|| {
            std::env::var_os("AUSTRAL_BIN")
                .map(PathBuf::from)
                // The stock front-end is the dune-built binary at the
                // australVM root (the same one test-programs/runner.py
                // invokes as "./austral"). Since umbrella is a submodule
                // inside australVM, that root is one level up.
                .unwrap_or_else(|| here.join("../austral"))
        }),
        crate_dir: crate_dir.unwrap_or_else(|| here.join("modules/rscrate")),
        expect,
    }
}

/// 1. cargo build the crate and hand back its cdylib path.
fn cargo_build(crate_dir: &PathBuf) -> Result<PathBuf, String> {
    let out = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--message-format=json")
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
    // The artifact path comes from cargo's own JSON stream: the first
    // compiler-artifact payload whose filename is a shared library.
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["reason"] != "compiler-artifact" {
            continue;
        }
        for f in v["filenames"].as_array().into_iter().flatten() {
            let Some(p) = f.as_str() else { continue };
            if p.ends_with(".so") || p.ends_with(".dylib") || p.ends_with(".dll") {
                return Ok(PathBuf::from(p));
            }
        }
    }
    Err("cargo reported no cdylib artifact".to_string())
}

/// 2. RTLD_NOW (fail loudly on any unresolved symbol in the crate) +
///    RTLD_GLOBAL (put its symbols where dlsym(RTLD_DEFAULT) — and thus
///    the bridge's import resolver — can see them). The handle is
///    intentionally never closed: JIT'd code may call into it for the
///    lifetime of the process.
fn dlopen(path: &PathBuf) -> Result<std::ptr::NonNull<c_void>, String> {
    let cpath = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|e| format!("path is not valid CString: {e}"))?;
    let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    std::ptr::NonNull::new(handle)
        .ok_or_else(|| unsafe {
            let e = libc::dlerror();
            if e.is_null() {
                "dlopen failed".to_string()
            } else {
                CStr::from_ptr(e).to_string_lossy().into_owned()
            }
        })
}
// Note: the NonNull handle is deliberately dropped here without
// dlclose — closing it while JIT'd code may still call into the
// crate would be unsound, so the mapping lives for the process.

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
    let out = cmd.output().map_err(|e| format!("spawn {austral:?}: {e}"))?;
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
fn jit_compile(cps: &[u8], entry: &str) -> Result<*const c_void, String> {
    let ptr = compile_to_function_named(cps.as_ptr(), cps.len(), entry.as_ptr(), entry.len());
    if ptr.is_null() {
        Err(take_bridge_error())
    } else {
        Ok(ptr)
    }
}

fn main() {
    let cli = parse_cli();

    // 1. cargo subprocess → artifact path.
    let cdylib = match cargo_build(&cli.crate_dir) {
        Ok(p) => p,
        Err(e) => die(e),
    };
    eprintln!("umbrella: built {}", cdylib.display());

    // 2. dlopen → symbols reachable via the bridge's dlsym fallback.
    if let Err(e) = dlopen(&cdylib) {
        die(e);
    }
    eprintln!("umbrella: dlopen'd {} (RTLD_NOW|RTLD_GLOBAL)", cdylib.display());

    // 3. front-end subprocess → CPS bytes.
    let cps_out: PathBuf = std::env::temp_dir().join(format!("umbrella-{}.cps", std::process::id()));
    let cps = match front_end(&cli.austral, &cli.modules, &cps_out) {
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
    println!("RESULT {result}");
    if let Some(want) = cli.expect {
        if result != want {
            die(format!("expected {want}, got {result}"));
        }
        eprintln!("umbrella: matches --expect {want}");
    }
}