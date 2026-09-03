//! Unit tests for the CPS-to-Cranelift JIT compiler (cps.rs).
//!
//! Builds minimal CPS binary IR buffers and runs them through the full
//! 3-pass compilation + JIT execution pipeline.

use std::ptr;

// ── CPS binary helpers ──────────────────────────────────────────────

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

/// Build a CPS v1 binary with one function named "run" whose body is the
/// raw `body_data`. The JIT entry-point lookup requires a function named
/// "run" (or an explicit name via `compile_to_function_named`'s name arg).
fn cps_v1(body_data: &[u8]) -> Vec<u8> {
    cps_v1_named("run", body_data)
}

/// Like [`cps_v1`] but with a caller-chosen function name — the JIT module
/// is process-global and persists across tests, so compiling two functions
/// with the same name in one test process is a `DuplicateDefinition`.
fn cps_v1_named(fname: &str, body_data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x43505331); // magic v1
    write_u32(&mut buf, 1); // func_count
    write_str(&mut buf, fname);
    write_u32(&mut buf, 0); // param_count
    buf.push(0); // ret_type
    write_u32(&mut buf, body_data.len() as u32);
    buf.extend_from_slice(body_data);
    buf
}

/// Body that returns a constant i64 value using the `0x07 <expr>` pattern.
fn body_return_const(val: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // tail/return context
    body.push(0x01); // iconst
    write_i64(&mut body, val);
    body
}

/// Body that tail-calls a 0-arg import and returns its result.
fn body_call_import(name: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // tail context
    body.push(0x04); // App (function call)
    write_str(&mut body, name);
    write_u32(&mut body, 0); // arg_count
    body
}

/// Body that tail-calls a 2-arg import with two i64 constants and returns
/// its result — the shape the Austral compiler emits for `lhs op rhs` on
/// Int64 (lowers to `Austral.Pervasive::trappingAdd` etc.).
fn body_call_import_2args(name: &str, a: i64, b: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // tail context
    body.push(0x04); // App (function call)
    write_str(&mut body, name);
    write_u32(&mut body, 2); // arg_count
    body.push(0x01); // iconst a
    write_i64(&mut body, a);
    body.push(0x01); // iconst b
    write_i64(&mut body, b);
    body
}

/// Malformed inputs
fn cps_truncated() -> Vec<u8> {
    vec![0x43, 0x50, 0x53]
}

fn cps_bad_opcode() -> Vec<u8> {
    cps_v1(&[0x07, 0xFF]) // 0xFF = unknown expression opcode
}

// ── Test helpers ────────────────────────────────────────────────────

fn compile(cps: &[u8]) -> *const std::ffi::c_void {
    let _lock = COMPILE_LOCK.lock().unwrap();
    // Allow all by default for execution tests.
    austral_cranelift_bridge::auth::set_allow_all();
    let ptr = austral_cranelift_bridge::compile_to_function_named(
        cps.as_ptr(),
        cps.len(),
        ptr::null(),
        0,
    );
    if ptr.is_null() {
        let err = austral_cranelift_bridge::cranelift_last_error();
        let msg = if !err.is_null() {
            unsafe { std::ffi::CStr::from_ptr(err) }
                .to_string_lossy()
                .to_string()
        } else {
            "unknown error".to_string()
        };
        panic!("JIT compile failed: {msg}");
    }
    ptr
}

fn execute(ptr: *const std::ffi::c_void) -> i64 {
    if ptr.is_null() {
        return -999;
    }
    let f: fn() -> i64 = unsafe { std::mem::transmute(ptr) };
    f()
}

// ── Opcode round-trip tests ────────────────────────────────────────

#[test]
fn returns_constant_42() {
    let cps = cps_v1(&body_return_const(42));
    assert_eq!(execute(compile(&cps)), 42);
}

#[test]
fn returns_constant_negative() {
    let cps = cps_v1(&body_return_const(-7));
    assert_eq!(execute(compile(&cps)), -7);
}

#[test]
fn returns_constant_zero() {
    let cps = cps_v1(&body_return_const(0));
    assert_eq!(execute(compile(&cps)), 0);
}

#[test]
fn returns_constant_max_i64() {
    let cps = cps_v1(&body_return_const(i64::MAX));
    assert_eq!(execute(compile(&cps)), i64::MAX);
}

#[test]
fn calls_uk_version() {
    let cps = cps_v1(&body_call_import("uk_version"));
    assert_eq!(execute(compile(&cps)), 1);
}

#[test]
fn calls_uk_init() {
    let cps = cps_v1(&body_call_import("uk_init"));
    assert_eq!(execute(compile(&cps)), 0);
}

// ── Austral.Pervasive::trapping* arithmetic intrinsics ─────────────
// The Austral compiler lowers `+ - * /` on Int64 to calls to these
// typeclass methods; the JIT must resolve them (regression: before they
// were registered, any module using arithmetic panicked at finalize with
// "can't resolve symbol Austral.Pervasive::trappingAdd").

#[test]
fn trapping_add_resolves_and_executes() {
    let cps = cps_v1(&body_call_import_2args(
        "Austral.Pervasive::trappingAdd",
        40,
        2,
    ));
    assert_eq!(execute(compile(&cps)), 42);
}

#[test]
fn trapping_subtract_resolves_and_executes() {
    let cps = cps_v1(&body_call_import_2args(
        "Austral.Pervasive::trappingSubtract",
        44,
        2,
    ));
    assert_eq!(execute(compile(&cps)), 42);
}

#[test]
fn trapping_multiply_resolves_and_executes() {
    let cps = cps_v1(&body_call_import_2args(
        "Austral.Pervasive::trappingMultiply",
        6,
        7,
    ));
    assert_eq!(execute(compile(&cps)), 42);
}

#[test]
fn trapping_divide_resolves_and_executes() {
    let cps = cps_v1(&body_call_import_2args(
        "Austral.Pervasive::trappingDivide",
        84,
        2,
    ));
    assert_eq!(execute(compile(&cps)), 42);
}

/// The packed return shape of `durable_status_module`'s `run()`:
/// `(statusN * 65536) + snapN` with both operands small — the exact
/// arithmetic the module does through the JIT. The JIT module is
/// process-global, so each compile uses a distinct function name to avoid
/// DuplicateDefinition.
#[test]
fn trapping_packed_return_matches_durable_status_module() {
    let _lock = COMPILE_LOCK.lock().unwrap();
    austral_cranelift_bridge::auth::set_allow_all();
    let mul = cps_v1_named(
        "pack_mul",
        &body_call_import_2args("Austral.Pervasive::trappingMultiply", 400, 65536),
    );
    // Direct FFI call — `compile` would re-lock the mutex (deadlock).
    let hi = execute(austral_cranelift_bridge::compile_to_function_named(
        mul.as_ptr(),
        mul.len(),
        ptr::null(),
        0,
    ));
    // (statusN << 16) — high 16 bits of the packed return.
    assert_eq!(hi, 400 * 65536);

    let add = cps_v1_named(
        "pack_add",
        &body_call_import_2args("Austral.Pervasive::trappingAdd", hi, 3),
    );
    let ptr = austral_cranelift_bridge::compile_to_function_named(
        add.as_ptr(),
        add.len(),
        ptr::null(),
        0,
    );
    // ... + snapN — low 16 bits.
    assert_eq!(execute(ptr), 400 * 65536 + 3);
}

/// Global auth is mutable global state; all compile paths must be serialized.
use std::sync::Mutex;
static COMPILE_LOCK: Mutex<()> = Mutex::new(());

/// Like `compile()` but allows null return (for malformed/truncated input tests).
fn compile_null_ok(cps: &[u8]) -> *const std::ffi::c_void {
    let _lock = COMPILE_LOCK.lock().unwrap();
    austral_cranelift_bridge::auth::set_allow_all();
    austral_cranelift_bridge::compile_to_function_named(cps.as_ptr(), cps.len(), ptr::null(), 0)
}

// ── Malformed input tests ──────────────────────────────────────────

#[test]
fn truncated_input_returns_null() {
    assert!(compile_null_ok(&cps_truncated()).is_null());
}

#[test]
fn bad_opcode_returns_null() {
    assert!(compile_null_ok(&cps_bad_opcode()).is_null());
}

#[test]
fn empty_input_returns_null() {
    assert!(compile_null_ok(&[]).is_null());
}

#[test]
fn null_input_returns_null() {
    assert!(compile_null_ok(&[]).is_null());
}

#[test]
fn bad_magic_returns_null() {
    let bad = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x01];
    assert!(compile_null_ok(&bad).is_null());
}

#[test]
fn truncated_body_returns_null() {
    // Valid header, valid body length, but body is truncated
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x43505331);
    write_u32(&mut buf, 1);
    write_str(&mut buf, "run");
    write_u32(&mut buf, 0);
    buf.push(0);
    write_u32(&mut buf, 100); // claims 100 bytes but none follow
    assert!(compile_null_ok(&buf).is_null());
}

// ── Permission-gate tests ──────────────────────────────────────────

/// Acquire the compile lock, set up auth, then compile.
fn with_auth<F>(setup: F, cps: &[u8]) -> *const std::ffi::c_void
where
    F: FnOnce(),
{
    let _lock = COMPILE_LOCK.lock().unwrap();
    setup();
    austral_cranelift_bridge::compile_to_function_named(cps.as_ptr(), cps.len(), ptr::null(), 0)
}

#[test]
fn self_call_allowed_under_deny_all() {
    let cps = cps_v1(&body_call_import("run"));
    let ptr = with_auth(austral_cranelift_bridge::auth::set_deny_all, &cps);
    assert!(!ptr.is_null(), "self-call should be allowed under DenyAll");
}

#[test]
fn au_call_allowed_under_deny_all() {
    let cps = cps_v1(&body_call_import("au_alloc"));
    let ptr = with_auth(austral_cranelift_bridge::auth::set_deny_all, &cps);
    assert!(!ptr.is_null(), "au_* call should be allowed under DenyAll");
}

#[test]
fn au_print_int_allowed_under_deny_all() {
    let cps = cps_v1(&body_call_import("au_print_int"));
    let ptr = with_auth(austral_cranelift_bridge::auth::set_deny_all, &cps);
    assert!(!ptr.is_null(), "au_* call should be allowed under DenyAll");
}

#[test]
fn uk_call_denied_under_deny_all() {
    let cps = cps_v1(&body_call_import("uk_version"));
    let ptr = with_auth(austral_cranelift_bridge::auth::set_deny_all, &cps);
    assert!(ptr.is_null(), "uk_* call should be denied under DenyAll");
}

#[test]
fn uk_call_allowed_under_grant() {
    use austral_cranelift_bridge::auth::{set_auth_engine, ManifestAuthEngine};
    let manifest = ManifestAuthEngine::from_toml_str(
        r#"
[module]
name = "run"

[grants]
kernel = ["uk_version"]
"#,
    )
    .unwrap();
    let cps = cps_v1(&body_call_import("uk_version"));
    let ptr = with_auth(|| set_auth_engine(Box::new(manifest)), &cps);
    assert!(
        !ptr.is_null(),
        "uk_* call should be allowed when grant is present"
    );
    assert_eq!(execute(ptr), 1, "uk_version() should return 1");
}

#[test]
fn uk_call_denied_when_grant_missing() {
    use austral_cranelift_bridge::auth::{set_auth_engine, ManifestAuthEngine};
    let manifest = ManifestAuthEngine::from_toml_str(
        r#"
[module]
name = "run"

[grants]
kernel = ["uk_version"]
"#,
    )
    .unwrap();
    let cps = cps_v1(&body_call_import("uk_evolve")); // not in grants
    let ptr = with_auth(|| set_auth_engine(Box::new(manifest)), &cps);
    assert!(
        ptr.is_null(),
        "uk_evolve should be denied when not in grants"
    );
}
