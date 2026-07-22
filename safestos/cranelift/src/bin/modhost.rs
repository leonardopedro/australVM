//! `modhost` — module host for the unfer kernel module system.
//!
//! Subcommands:
//!   authorize <manifest.toml> <module> <uk_symbol>  — check authorization
//!   host      <austral-path> <module-src>...         — host a compiled module
//!
//! The `host` subcommand spawns the Austral compiler in JIT-server mode,
//! compiles the given module(s) once, then accepts `call <entrypoint>`
//! commands on stdin so the caller can invoke entrypoints without recompiling.
//!
//! Usage:
//!   modhost authorize <manifest.toml> <module> <uk_symbol>
//!   modhost host <austral-path> <module-src>... -- [call <name>]* <exit>
//!
//! Exit codes: 0 = success, 1 = denied/error, 2 = usage error.

use austral_cranelift_bridge::auth::{self, ManifestAuthEngine};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, ExitCode, Stdio};

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
            // Mirrors unfer_protocol code 4001 CallDenied.
            eprintln!("UK-4001 CallDenied: {reason}");
            ExitCode::from(1)
        }
    }
}

/// Spawn the Austral compiler in JIT-server mode and communicate with it.
fn spawn_jit_server(austral_path: &str, module_args: &[String], call_args: &[String]) -> ExitCode {
    // Build the austral command: `austral compile <module>... --jit-server --allow-all --target-type=tc`
    let mut cmd = Command::new(austral_path);
    cmd.arg("compile");
    for m in module_args {
        cmd.arg(m);
    }
    cmd.arg("--jit-server");
    cmd.arg("--allow-all");
    cmd.arg("--target-type=tc");

    // Pipe stdin/stdout
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

    // Wait for "READY" line from austral
    let mut ready_line = String::new();
    match reader.read_line(&mut ready_line) {
        Ok(n) if n > 0 && ready_line.starts_with("READY") => {
            // All good
        }
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

    // Send call commands and read results
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

    // Send exit
    let _ = writeln!(stdin, "exit");
    let _ = child.wait();

    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("authorize") => {
            if args.len() != 5 {
                eprintln!("usage: modhost authorize <manifest.toml> <module> <uk_symbol>");
                return ExitCode::from(2);
            }
            cmd_authorize(&args)
        }
        Some("host") => {
            if args.len() < 4 {
                eprintln!("usage: modhost host <austral-path> <module-src>... -- <entrypoint>...");
                return ExitCode::from(2);
            }
            // Parse: modhost host <austral-path> <module-src>... -- <entrypoint>...
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
            eprintln!("  modhost authorize <manifest.toml> <module> <uk_symbol>");
            eprintln!("  modhost host <austral-path> <module-src>... -- <entrypoint>...");
            ExitCode::from(2)
        }
    }
}
