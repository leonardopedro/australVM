//! `modhost` — minimal module host for the unfer kernel module system.
//!
//! It loads a module's `module.toml` manifest into the safestos
//! [`ManifestAuthEngine`] and answers the authorization question the cranelift
//! JIT asks for every `uk_*` kernel call a module makes: *is this module
//! granted this kernel symbol?* This is the enforcement point behind UK-4001
//! `CallDenied`.
//!
//! The actual JIT execution of a compiled Austral cell is driven by the Austral
//! compiler's CPS-JIT path (`austral compile --use-cps-jit`, see
//! `demo_module/run_demo.sh`); both share the same `auth::check` gate registered
//! in `cps.rs::check_call_permission`, so the decision `modhost` reports is the
//! decision the JIT enforces.
//!
//! Usage:
//!   modhost authorize <manifest.toml> <module> <uk_symbol>
//!
//! Exit codes: 0 = allowed, 1 = denied (UK-4001), 2 = usage/IO error.

use austral_cranelift_bridge::auth::{self, ManifestAuthEngine};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 || args[1] != "authorize" {
        eprintln!("usage: modhost authorize <manifest.toml> <module> <uk_symbol>");
        return ExitCode::from(2);
    }
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
