//! netcert — ask the kernel's DeltaNets whether two Austral fragments are the
//! same net, and print the certificate.
//!
//! The command-line face of `austral_defeq_translate` (the `uk_austral_defeq`
//! kernel symbol): both fragments are lowered to interaction nets, reduced to
//! their unique normal forms, and compared. When the normal forms are
//! isomorphic the report carries the node-permutation witness π — the
//! certificate a conversion step needs. The JSON is printed verbatim so the
//! caller reads the kernel's own report.
//!
//! `scripts/lean-net-cert` consumes this to turn a conversion obligation of a
//! generated typing derivation into a `valid_iso` certificate
//! (docs/lean4mm0.md §13.2).
//!
//! Exit codes: 0 equivalent · 1 not equivalent · 2 the check could not run
//! (bridge unavailable, or a fragment the reducer refuses).

use std::ffi::{CStr, CString};
use std::process::ExitCode;

fn last_error() -> String {
    let p = austral_cranelift_bridge::cranelift_last_error();
    if p.is_null() {
        "(no error recorded)".to_string()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().to_string()
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!(
            "usage: netcert <source-a> <source-b>\n\
             \x20 Both sides are Austral source fragments the deltanet reducer accepts."
        );
        return ExitCode::from(2);
    }
    let (a, b) = match (CString::new(args[0].clone()), CString::new(args[1].clone())) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            eprintln!("netcert: a source fragment contains a NUL byte");
            return ExitCode::from(2);
        }
    };

    let out = austral_cranelift_bridge::austral_defeq_translate(a.as_ptr(), b.as_ptr());
    if out.is_null() {
        eprintln!("netcert: {}", last_error());
        return ExitCode::from(2);
    }
    let json = unsafe { CStr::from_ptr(out) }.to_string_lossy().to_string();
    unsafe { austral_cranelift_bridge::cranelift_free_string(out) };

    println!("{json}");
    // The report carries the verdict; the exit code mirrors it for shell callers.
    let equivalent = serde_json::from_str::<serde_json::Value>(&json)
        .ok()
        .and_then(|v| v.get("equivalent").and_then(|e| e.as_bool()))
        .unwrap_or(false);
    ExitCode::from(if equivalent { 0 } else { 1 })
}
