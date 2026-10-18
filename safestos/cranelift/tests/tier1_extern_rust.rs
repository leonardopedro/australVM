//! PLAN_valen §4 Tier 1 — `extern rust` called end-to-end through the bridge.
//!
//! What each test pins, in the order the plan's P2 verification asks for it:
//!
//! * the `rs_*` namespace is registered in the JIT symbol table from the
//!   reference crate (`RS_FFI_SYMBOLS`, mirroring `UNFER_SYMBOLS`);
//! * a manifest that GRANTS the extern call compiles and *executes* it — the
//!   reference crate's `rs_checksum` runs for real over a live buffer, and
//!   the result is checked against an independent FNV-1a implementation in
//!   this file, not against the crate's own constants;
//! * authorization is inherited, not new: a manifest granting `uk_version`
//!   does NOT grant `rs_checksum` (UK-4001), and no manifest at all denies
//!   the extern call outright (`cps::a10_live_grant_tests` shape, live JIT
//!   gate rather than `modhost authorize`).
//!
//! The auth engine and the JIT module are process-global, and counter-test
//! order matters (each test installs its own engine under one lock, and no
//! test may rely on a previous test's engine), so every test here takes
//! `STATE_LOCK`, sets its own engine + principal on entry, and restores the
//! documented default (DenyAll, no principal) on exit.

use std::process::Command;
use std::ptr;
use std::sync::Mutex;

/// Serializes the process-global auth engine + JIT state across tests in
/// this binary (same discipline as `cps::a10_live_grant_tests`).
static STATE_LOCK: Mutex<()> = Mutex::new(());

// ── manifest helpers ────────────────────────────────────────────────

/// Install a manifest for `module` granting exactly `callables` as the
/// live engine (a10 test shape: explicit engine + deployment principal, no
/// file IO behind the test's back).
fn load_manifest(module: &str, callables: &[&str]) {
    let list = callables
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!("[module]\nname = \"{module}\"\n\n[grants]\nkernel = [{list}]\n");
    let engine = austral_cranelift_bridge::auth::ManifestAuthEngine::from_toml_str(&toml)
        .expect("manifest parses");
    austral_cranelift_bridge::auth::set_auth_engine(Box::new(engine));
    austral_cranelift_bridge::auth::set_deployment_principal(module.to_string());
}

/// The documented default the tests leave behind: no manifest -> DenyAll.
fn restore_default_auth() {
    austral_cranelift_bridge::auth::set_auth_engine(Box::new(
        austral_cranelift_bridge::auth::DenyAll,
    ));
    austral_cranelift_bridge::auth::set_deployment_principal(String::new());
}

// ── CPS binary helpers (mirror tests/cps_tests.rs + the OCaml writer) ──

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

/// CPS **v2** binary: magic, implicit module name "rs_widget" as the
/// authorization principal (the header name is the fallback principal when
/// no deployment principal is set — A10), one function with `params`, body
/// as raw bytes.
fn cps_v2(fname: &str, module: &str, params: &[&str], body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x43505332); // magic v2 (module header present)
    write_str(&mut buf, module);
    write_u32(&mut buf, 1); // func_count
    write_str(&mut buf, fname);
    write_u32(&mut buf, params.len() as u32);
    for p in params {
        write_str(&mut buf, p);
    }
    buf.push(0x01); // ret_type: I64
    write_u32(&mut buf, body.len() as u32);
    buf.extend_from_slice(body);
    buf
}

/// Body: `return <callee>(a, b)` where a/b are the function's own params —
/// exactly what the Austral compiler emits for an `extern rust fn` call
/// whose arguments forward a linear cell's buffer pair.
fn body_tail_call_2params(fname: &str, a: &str, b: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // Return
    body.push(0x04); // App
    write_str(&mut body, fname);
    write_u32(&mut body, 2); // arg_count
    body.push(0x02); // Var a
    write_str(&mut body, a);
    body.push(0x02); // Var b
    write_str(&mut body, b);
    body
}

/// Body: `return <callee>()` (scalar-only case, no buffer crosses).
fn body_tail_call_0params(fname: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // Return
    body.push(0x04); // App
    write_str(&mut body, fname);
    write_u32(&mut body, 0); // arg_count
    body
}

// ── compile / execute helpers (cps_tests.rs shape, auth NOT bypassed) ──

fn compile_result(cps: &[u8]) -> Result<*const std::ffi::c_void, String> {
    let ptr = austral_cranelift_bridge::compile_to_function_named(
        cps.as_ptr(),
        cps.len(),
        ptr::null(),
        0,
    );
    if ptr.is_null() {
        let err = austral_cranelift_bridge::cranelift_last_error();
        let msg = if err.is_null() {
            "unknown error".to_string()
        } else {
            unsafe { std::ffi::CStr::from_ptr(err) }
                .to_string_lossy()
                .to_string()
        };
        return Err(msg);
    }
    Ok(ptr)
}

fn execute_2(ptr: *const std::ffi::c_void, a: i64, b: i64) -> i64 {
    austral_cranelift_bridge::execute_function_2(ptr, a, b)
}

fn execute_0(ptr: *const std::ffi::c_void) -> i64 {
    austral_cranelift_bridge::execute_function(ptr)
}

/// Independent FNV-1a 64 (reference implementation in the test, so the
/// granted-execution assertion checks the reference crate rather than
/// trusting its own constants).
fn fnv1a64(bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h as i64
}

// ── tests ──────────────────────────────────────────────────────────

/// The data-driven table IS the registry: every rs_* symbol the bridge can
/// resolve comes from `RS_FFI_SYMBOLS`, and tests iterate that table
/// (symbol_sync discipline).
#[test]
fn rs_namespace_table_registers_reference_symbols() {
    let symbols = austral_cranelift_bridge::registered_rs_symbols();
    assert!(
        symbols.contains(&"rs_checksum"),
        "rs_checksum missing from RS_FFI_SYMBOLS: {symbols:?}"
    );
    assert!(
        symbols.contains(&"rs_version"),
        "rs_version missing from RS_FFI_SYMBOLS: {symbols:?}"
    );
}

/// Granted: end-to-end execution of the tier-1 call. The manifest names
/// `rs_checksum`, the JIT resolves it to the reference crate, and the callee
/// runs over a live buffer passed straight through the CPS boundary.
#[test]
fn granted_manifest_executes_reference_checksum() {
    let _lock = STATE_LOCK.lock().unwrap();

    // Use the real C-ABI loader — the exact entry the OCaml side
    // (`ocaml_load_auth_manifest`) and `austral --auth-manifest` call.
    let toml = "[module]\nname = \"rs_widget\"\n\n[grants]\nkernel = [\"rs_checksum\"]\n";
    let ok = unsafe {
        austral_cranelift_bridge::auth::safestos_load_auth_manifest(toml.as_ptr(), toml.len())
    };
    assert_eq!(ok, 1, "manifest loader rejected the test manifest");

    let cps = cps_v2(
        "run_rs_checksum",
        "rs_widget",
        &["buf", "len"],
        &body_tail_call_2params("rs_checksum", "buf", "len"),
    );
    let ptr = compile_result(&cps).expect("a granted rs_ call must compile");
    let buf = b"au!";
    let got = execute_2(ptr, buf.as_ptr() as i64, buf.len() as i64);
    assert_eq!(got, fnv1a64(buf), "rs_checksum returned the wrong digest");

    restore_default_auth();
}

/// Scalar-only shape: `rs_version()` crosses with no buffer at all.
#[test]
fn granted_manifest_executes_scalar_rs_version() {
    let _lock = STATE_LOCK.lock().unwrap();
    load_manifest("rs_widget", &["rs_version"]);

    let cps = cps_v2(
        "run_rs_version",
        "rs_widget",
        &[],
        &body_tail_call_0params("rs_version"),
    );
    let ptr = compile_result(&cps).expect("a granted rs_ call must compile");
    assert_eq!(execute_0(ptr), 1, "rs_version must return 1");

    restore_default_auth();
}

/// The plan's key claim (PLAN_valen §4): "A manifest that grants `uk_push`
/// does not thereby grant a tier-1 call." uk-version granted, rs_checksum
/// denied at the LIVE JIT gate with UK-4001, attributed to the module.
#[test]
fn uk_grant_does_not_grant_rs_namespace() {
    let _lock = STATE_LOCK.lock().unwrap();
    load_manifest("rs_widget", &["uk_version"]);

    let cps = cps_v2(
        "run_rs_denied_by_ns",
        "rs_widget",
        &["buf", "len"],
        &body_tail_call_2params("rs_checksum", "buf", "len"),
    );
    let err = compile_result(&cps).expect_err("a uk-only manifest must deny rs_checksum");
    assert!(
        err.contains("UK-4001"),
        "expected UK-4001 for rs_checksum, got: {err}"
    );
    assert!(
        err.contains("'rs_widget'") && !err.contains("run_rs_denied_by_ns"),
        "the denial principal must be the module, got: {err}"
    );

    restore_default_auth();
}

/// No manifest at all -> DenyAll: an ungranted extern rust call never
/// reaches the reference crate.
#[test]
fn no_manifest_denies_rs_call() {
    let _lock = STATE_LOCK.lock().unwrap();
    restore_default_auth();

    let cps = cps_v2(
        "run_rs_denied_no_manifest",
        "rs_widget",
        &["buf", "len"],
        &body_tail_call_2params("rs_checksum", "buf", "len"),
    );
    let err = compile_result(&cps).expect_err("DenyAll must deny an rs_ call");
    assert!(err.contains("UK-4001"), "expected UK-4001, got: {err}");

    restore_default_auth();
}

/// The out-of-process enforcement point (PLAN_valen §4 Tier 0) answers the
/// same question for `rs_*` names from a manifest file: `modhost authorize`
/// allows a symbol the manifest grants and denies `rs_checksum` to a
/// uk-only manifest with UK-4001 — namespace isolation proven on both
/// sides of the fence.
#[test]
fn modhost_authorize_answers_for_rs_namespace() {
    let dir = std::env::temp_dir().join(format!("tier1_rs_auth_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let granted = dir.join("granted.toml");
    std::fs::write(
        &granted,
        "[module]\nname = \"rs_widget\"\n\n[grants]\nkernel = [\"uk_version\", \"rs_checksum\"]\n",
    )
    .expect("write granted manifest");
    let uk_only = dir.join("uk_only.toml");
    std::fs::write(
        &uk_only,
        "[module]\nname = \"rs_widget\"\n\n[grants]\nkernel = [\"uk_version\"]\n",
    )
    .expect("write uk-only manifest");

    let authorize = |manifest: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_modhost"))
            .args([
                "authorize",
                manifest.to_str().unwrap(),
                "rs_widget",
                "rs_checksum",
            ])
            .output()
            .expect("modhost binary runs")
    };

    let allowed = authorize(&granted);
    let allowed_out = String::from_utf8_lossy(&allowed.stdout);
    assert_eq!(
        allowed.status.code(),
        Some(0),
        "granted manifest must be allowed; stderr: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert!(
        allowed_out.contains("ALLOW") && allowed_out.contains("rs_checksum"),
        "expected ALLOW for rs_checksum, got: {allowed_out}"
    );

    let denied = authorize(&uk_only);
    let denied_err = String::from_utf8_lossy(&denied.stderr);
    assert_eq!(
        denied.status.code(),
        Some(1),
        "a uk-only manifest must deny rs_checksum"
    );
    assert!(
        denied_err.contains("UK-4001") && denied_err.contains("rs_checksum"),
        "expected UK-4001 naming rs_checksum, got: {denied_err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
