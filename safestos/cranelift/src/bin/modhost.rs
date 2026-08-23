//! `modhost` — module host for the unfer kernel module system.
//!
//! Subcommands:
//!   authorize   <manifest.toml> <module> <uk_symbol>   — check authorization
//!   host        <module-dir> --call <ep> [--args ...] [--args-json <json>] [--repeat N] [--swap <dir>]
//!   host-legacy <austral-path> <module-src>... -- <entrypoint>...
//!
//! The `host` subcommand loads a pre-compiled module directory (module.cps +
//! module.toml), compiles it once via the Cranelift JIT, then calls entrypoints
//! without recompiling. Supports hot-swap via --swap.
//!
//! Exit codes: 0 = success, 1 = denied/error, 2 = usage error.

use austral_cranelift_bridge::auth::{self, ManifestAuthEngine};
use austral_cranelift_bridge::module::ModuleHost;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};

/// Host runtime primitives for JIT-compiled Austral modules.
///
/// The Rust library (`austral_cranelift_bridge`) now provides `au_*`
/// itself (self-contained `.so`, see `src/lib.rs`), so modhost inherits
/// them from the rlib it links; the OCaml FFI provides identical copies
/// in `lib/rust_bridge.c` (ELF interposition: the executable's copies win).
fn cmd_authorize(args: &[String]) -> ExitCode {
    let (manifest_path, principal, symbol) = (&args[2], &args[3], &args[4]);

    let toml = match std::fs::read_to_string(manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("modhost: cannot read manifest {manifest_path}: {e}");
            return ExitCode::from(2);
        }
    };
    let engine = match ManifestAuthEngine::from_toml_str(&toml) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("modhost: invalid manifest: {e}");
            return ExitCode::from(2);
        }
    };
    auth::set_auth_engine(Box::new(engine));

    match auth::check(principal, "Call", symbol) {
        Ok(()) => {
            println!("ALLOW: module '{principal}' may Call '{symbol}'");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("UK-4001 CallDenied: {reason}");
            ExitCode::from(1)
        }
    }
}

fn cmd_host(args: &[String]) -> ExitCode {
    let module_dir = Path::new(&args[2]);

    let mut entrypoint = String::from("run");
    let mut call_args: Vec<i64> = Vec::new();
    let mut args_json: Option<String> = None;
    let mut repeat: u64 = 1;
    let mut swap_dir: Option<String> = None;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--call" => {
                i += 1;
                if i < args.len() {
                    entrypoint = args[i].clone();
                }
            }
            "--args-json" => {
                i += 1;
                if i < args.len() {
                    args_json = Some(args[i].clone());
                }
            }
            "--args" => {
                i += 1;
                while i < args.len() && !args[i].starts_with("--") {
                    if let Ok(v) = args[i].parse::<i64>() {
                        call_args.push(v);
                    }
                    i += 1;
                }
                continue;
            }
            "--repeat" => {
                i += 1;
                if i < args.len() {
                    repeat = args[i].parse().unwrap_or(1);
                }
            }
            "--swap" => {
                i += 1;
                if i < args.len() {
                    swap_dir = Some(args[i].clone());
                }
            }
            other => {
                eprintln!("modhost: unknown option '{other}'");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let mut host = ModuleHost::new();

    match host.load(module_dir) {
        Ok(handle) => {
            println!(
                "modhost: loaded '{}' v{} ({} functions, entry='{}')",
                handle.manifest.name,
                handle.manifest.version,
                handle.function_count(),
                handle.manifest.entry,
            );
        }
        Err(e) => {
            eprintln!("modhost: load failed: {e}");
            return ExitCode::from(1);
        }
    }

    if let Some(ref sd) = swap_dir {
        let module_name = {
            let h = host.loaded_modules();
            h[0].to_string()
        };
        match host.swap(&module_name, Path::new(sd)) {
            Ok(()) => println!("modhost: hot-swapped '{module_name}' from {sd}"),
            Err(e) => {
                eprintln!("modhost: swap failed: {e}");
                return ExitCode::from(1);
            }
        }
    }

    let module_name = {
        let h = host.loaded_modules();
        h[0].to_string()
    };

    for call_idx in 0..repeat {
        if repeat > 1 {
            println!("--- call {}/{} ---", call_idx + 1, repeat);
        }
        let result: Result<String, String> = if let Some(json) = &args_json {
            #[cfg(feature = "ecmascript")]
            {
                host.call_json(&module_name, &entrypoint, json)
            }
            #[cfg(not(feature = "ecmascript"))]
            {
                let _ = json;
                Err("--args-json requires the 'ecmascript' feature".to_string())
            }
        } else {
            host.call(&module_name, &entrypoint, &call_args).map(|v| v.to_string())
        };
        match result {
            Ok(result) => println!("{entrypoint}: {result}"),
            Err(e) => {
                eprintln!("modhost: call failed: {e}");
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

fn spawn_jit_server(
    austral_path: &str,
    module_args: &[String],
    call_args: &[String],
) -> ExitCode {
    let mut cmd = Command::new(austral_path);
    cmd.arg("compile");
    for m in module_args {
        cmd.arg(m);
    }
    cmd.arg("--jit-server");
    cmd.arg("--allow-all");
    cmd.arg("--target-type=tc");

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::inherit());

    let mut child: Child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("modhost: failed to spawn austral: {e}");
            return ExitCode::from(1);
        }
    };

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    let mut ready_line = String::new();
    match reader.read_line(&mut ready_line) {
        Ok(n) if n > 0 && ready_line.starts_with("READY") => {}
        Ok(_) => {
            eprintln!("modhost: unexpected output from austral: {ready_line}");
            let _ = child.kill();
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("modhost: failed to read from austral: {e}");
            let _ = child.kill();
            return ExitCode::from(1);
        }
    }

    for target in call_args {
        writeln!(stdin, "call {target}").unwrap();
        let mut result_line = String::new();
        match reader.read_line(&mut result_line) {
            Ok(n) if n > 0 => {
                let result = result_line.trim();
                if result.starts_with("RESULT ") {
                    let val = &result["RESULT ".len()..];
                    println!("{target}: {val}");
                } else if result.starts_with("ERROR ") {
                    eprintln!("modhost: {result}");
                    let _ = child.kill();
                    return ExitCode::from(1);
                } else {
                    eprintln!("modhost: unexpected output: {result}");
                }
            }
            Ok(_) => break,
            Err(e) => {
                eprintln!("modhost: read error: {e}");
                break;
            }
        }
    }

    let _ = writeln!(stdin, "exit");
    let _ = child.wait();

    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("authorize") => {
            if args.len() != 5 {
                eprintln!(
                    "usage: modhost authorize <manifest.toml> <module> <uk_symbol>"
                );
                return ExitCode::from(2);
            }
            cmd_authorize(&args)
        }
        Some("host") => {
            if args.len() < 3 {
                eprintln!(
                    "usage: modhost host <module-dir> --call <entrypoint> [--args ...] [--repeat N] [--swap <dir>]"
                );
                return ExitCode::from(2);
            }
            cmd_host(&args)
        }
        Some("host-legacy") => {
            if args.len() < 4 {
                eprintln!(
                    "usage: modhost host-legacy <austral-path> <module-src>... -- <entrypoint>..."
                );
                return ExitCode::from(2);
            }
            let austral_path = &args[2];
            let mut i = 3;
            let mut module_args: Vec<String> = Vec::new();
            let mut call_args: Vec<String> = Vec::new();
            let mut seen_double_dash = false;

            while i < args.len() {
                if args[i] == "--" {
                    seen_double_dash = true;
                    i += 1;
                    continue;
                }
                if seen_double_dash {
                    call_args.push(args[i].clone());
                } else {
                    module_args.push(args[i].clone());
                }
                i += 1;
            }

            if module_args.is_empty() {
                eprintln!("modhost: no module sources provided");
                return ExitCode::from(2);
            }

            spawn_jit_server(austral_path, &module_args, &call_args)
        }
        _ => {
            eprintln!("usage:");
            eprintln!(
                "  modhost authorize <manifest.toml> <module> <uk_symbol>"
            );
            eprintln!(
                "  modhost host <module-dir> --call <entrypoint> [--args ...] [--repeat N] [--swap <dir>]"
            );
            eprintln!(
                "  modhost host-legacy <austral-path> <module-src>... -- <entrypoint>..."
            );
            ExitCode::from(2)
        }
    }
}
