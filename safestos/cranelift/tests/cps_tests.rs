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

/// Body that constructs a union in *tail* position: `0x07` then an App of
/// `__union_new` with `2 + fields.len()` arguments `(size, tag, field…)`.
/// This is the shape the Austral lowering emits for `return Case(…)`.
fn body_tail_union(size: i64, tag: i64, fields: &[i64]) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // tail/return context
    body.push(0x04); // App
    write_str(&mut body, "__union_new");
    write_u32(&mut body, 2 + fields.len() as u32);
    body.push(0x01);
    write_i64(&mut body, size);
    body.push(0x01);
    write_i64(&mut body, tag);
    for field in fields {
        body.push(0x01);
        write_i64(&mut body, *field);
    }
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

/// Code generation must bind a cross-cell import, not fail on it: the cell
/// is linked later, so the symbol is an import the loader resolves. Strict
/// mode (JIT execution) keeps the hard failure. The bound thunk is never
/// executed here — reaching it would abort.
#[test]
fn link_time_mode_binds_a_cross_cell_import() {
    let cps = cps_v1_named("cross_cell_probe", &body_call_import("CrossCell::compute"));
    let compile_once = || {
        let _lock = COMPILE_LOCK.lock().unwrap();
        austral_cranelift_bridge::auth::set_allow_all();
        let p = austral_cranelift_bridge::compile_to_function_named(
            cps.as_ptr(),
            cps.len(),
            ptr::null(),
            0,
        );
        let err = austral_cranelift_bridge::cranelift_last_error();
        let msg = if err.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(err) }
                .to_string_lossy()
                .to_string()
        };
        (p, msg)
    };

    let (strict_ptr, strict_err) = compile_once();
    austral_cranelift_bridge::set_link_time_imports(1);
    let (bound_ptr, bound_err) = compile_once();
    austral_cranelift_bridge::set_link_time_imports(0);

    assert!(
        strict_ptr.is_null(),
        "strict mode must not bind a host-less symbol"
    );
    assert!(
        strict_err.contains("resolve symbol"),
        "strict refusal names the symbol: {strict_err}"
    );
    assert!(
        !bound_ptr.is_null(),
        "link-time mode must compile the import: {bound_err}"
    );
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

// ── The sub/mul instructions (0x06/0x18) ───────────────────────────
//
// Since 2026-11-14 `Compiler_cps.trapping_arith_op` emits these for `-`/`*`
// instead of an `App` to the Pervasive methods, so they are no longer decodable
// IR with no producer — and no longer exercised only through the call path.
// The tests above still cover the call form (older binaries still emit it);
// these cover what the compiler produces now.

/// Body that returns `a <opcode> b`, the shape the CPS emitter writes for a
/// two-operand arithmetic expression.
fn body_not(opcode: u8, a: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // tail/return context
    body.push(opcode);
    body.push(0x01); // iconst a
    write_i64(&mut body, a);
    body
}

fn body_binop(opcode: u8, a: i64, b: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // tail/return context
    body.push(opcode);
    body.push(0x01); // iconst a
    write_i64(&mut body, a);
    body.push(0x01); // iconst b
    write_i64(&mut body, b);
    body
}

#[test]
fn sub_opcode_computes_the_difference() {
    let cps = cps_v1_named("op_sub", &body_binop(0x06, 44, 2));
    assert_eq!(execute(compile(&cps)), 42);
    // Operand order is not commutative: the same operands the other way round
    // must be the negated result, or `a - b` was lowered as `b - a`.
    let cps = cps_v1_named("op_sub_swapped", &body_binop(0x06, 2, 44));
    assert_eq!(execute(compile(&cps)), -42);
}

#[test]
fn float_helpers_bitcast_and_compute() {
    // Float64 travels as its IEEE bit pattern in the CPS one-word ABI
    // (`CpsGen`'s literal encoding), so these helpers must bitcast, compute in
    // f64 and bitcast back. A rounding literal or an integer add at the call
    // site is exactly what these assertions rule out.
    let bits = |f: f64| f.to_bits() as i64;
    let f = |b: i64| f64::from_bits(b as u64);
    assert_eq!(
        f(austral_cranelift_bridge::au_fadd(bits(1.5), bits(2.25))),
        3.75
    );
    assert_eq!(
        f(austral_cranelift_bridge::au_fsub(bits(1.5), bits(2.25))),
        -0.75
    );
    assert_eq!(
        f(austral_cranelift_bridge::au_fmul(bits(1.5), bits(2.25))),
        1.5 * 2.25
    );
    assert_eq!(
        f(austral_cranelift_bridge::au_fdiv(bits(1.5), bits(2.25))),
        1.5 / 2.25
    );
    // Negative constants are bit patterns too, not sign-magnitude integers.
    assert_eq!(
        f(austral_cranelift_bridge::au_fadd(bits(-0.5), bits(0.25))),
        -0.25
    );
}

#[test]
fn not_opcode_complements_zero_and_one() {
    // 0x19 is the compiler's `not` on a 0/1 Bool. A bitwise complement would
    // return -2 for `not 1`, so this pins the 0/1 contract rather than the
    // instruction the JIT happens to emit.
    assert_eq!(
        execute(compile(&cps_v1_named("op_not_true", &body_not(0x19, 1)))),
        0
    );
    assert_eq!(
        execute(compile(&cps_v1_named("op_not_false", &body_not(0x19, 0)))),
        1
    );
}

#[test]
fn mul_opcode_computes_the_product() {
    let cps = cps_v1_named("op_mul", &body_binop(0x18, 6, 7));
    assert_eq!(execute(compile(&cps)), 42);
    let cps = cps_v1_named("op_mul_negative", &body_binop(0x18, -6, 7));
    assert_eq!(execute(compile(&cps)), -42);
}

/// The trap half of the contract: `-`/`*` abort on overflow
/// (`BuiltInModules.ml`), so the instruction must not wrap. The abort is
/// `std::process::abort` — uncatchable in-process — so the assertion is made in
/// a child process running this same binary, which is also the only way to
/// observe the message a user sees.
///
/// Without the `brif` in `emit_trapping_arith`, both cases would return a
/// wrapped i64 and exit 0; this test is what makes that a failure rather than
/// a silently wrong number.
fn run_trap_child(opcode: &str, kind: &str) -> std::process::Output {
    std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .args([
            "--exact",
            "arith_trap_child_exit",
            "--nocapture",
            "--ignored",
            "--test-threads=1",
        ])
        .env("AVM_ARITH_TRAP", format!("{opcode}:{kind}"))
        .output()
        .expect("re-exec the test binary")
}

fn assert_trapped(out: &std::process::Output, expected: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "overflow must abort, not return a wrapped value (stderr: {stderr})"
    );
    assert!(
        stderr.contains(expected),
        "expected {expected:?} on stderr, got: {stderr}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            out.status.signal(),
            Some(6),
            "the trap path must abort (SIGABRT), not exit with a code: {stderr}"
        );
    }
}

#[test]
fn sub_opcode_traps_on_overflow() {
    let out = run_trap_child("sub", "over");
    assert_trapped(&out, "Overflow in trappingSubtract (Int64)");
}

#[test]
fn mul_opcode_traps_on_overflow() {
    let out = run_trap_child("mul", "over");
    assert_trapped(&out, "Overflow in trappingMultiply (Int64)");
}

/// Not a test of its own: the child half of the two tests above. Ignored so it
/// only runs when re-executed with `AVM_ARITH_TRAP` set; the values are the
/// overflow operands, so this process is expected to abort.
#[test]
#[ignore]
fn arith_trap_child_exit() {
    let spec = std::env::var("AVM_ARITH_TRAP").expect("only meaningful as a child process");
    let (opcode, kind) = spec.split_once(':').expect("op:kind");
    let (op, name, a, b) = match (opcode, kind) {
        ("sub", "over") => (0x06, "child_sub_over", i64::MIN, 1),
        ("mul", "over") => (0x18, "child_mul_over", i64::MAX, 2),
        other => panic!("unknown child spec {other:?}"),
    };
    let cps = cps_v1_named(name, &body_binop(op, a, b));
    let _ = execute(compile(&cps));
    panic!("the child reached the wrap-free result instead of aborting");
}

/// X4a: two modules exporting the same name in one thread.
#[test]
fn two_run_exports_in_one_thread_both_compile_and_keep_their_own_body() {
    // This is the `JitTest` failure, reproduced deliberately.
    //
    // The engine is a per-thread singleton that lives as long as the thread, and
    // it has one symbol namespace. Two compiles in the same thread that both
    // export `run` used to collide, and the only workaround was to invent a
    // different function name per compile -- which hid the defect instead of
    // fixing it and made every test that needed two modules carry a rename.
    let _lock = COMPILE_LOCK.lock().unwrap();
    austral_cranelift_bridge::auth::set_allow_all();

    let first = cps_v1(&body_return_const(11));
    let a = austral_cranelift_bridge::compile_to_function_named(
        first.as_ptr(),
        first.len(),
        ptr::null(),
        0,
    );
    assert!(!a.is_null(), "first run export must compile");
    assert_eq!(execute(a), 11);

    // Same name, different body. Before X4a this was a DuplicateDefinition.
    let second = cps_v1(&body_return_const(22));
    let b = austral_cranelift_bridge::compile_to_function_named(
        second.as_ptr(),
        second.len(),
        ptr::null(),
        0,
    );
    assert!(!b.is_null(), "second run export must compile");
    assert_eq!(execute(b), 22);

    // The point of namespacing rather than resetting: the first definition is
    // still reachable, and each module executes its own body rather than
    // whichever one happened to be declared last.
    assert_ne!(a, b);
}

/// X4a: a reset gives the name back.
#[test]
fn reset_jit_returns_a_claimed_export_name() {
    let _lock = COMPILE_LOCK.lock().unwrap();
    austral_cranelift_bridge::auth::set_allow_all();
    let before = austral_cranelift_bridge::claimed_export_symbol_count();
    let m = cps_v1(&body_return_const(7));
    let p =
        austral_cranelift_bridge::compile_to_function_named(m.as_ptr(), m.len(), ptr::null(), 0);
    assert!(!p.is_null());
    assert!(
        austral_cranelift_bridge::claimed_export_symbol_count() > before,
        "the compile should have claimed a symbol"
    );
    austral_cranelift_bridge::reset_jit();
    assert_eq!(
        austral_cranelift_bridge::claimed_export_symbol_count(),
        0,
        "a reset must release every claimed name"
    );
}

/// The packed return shape of `durable_status_module`'s `run()`:
/// `(statusN * 65536) + snapN` with both operands small — the exact
/// arithmetic the module does through the JIT.
///
/// Both halves are named `run`, which is what the real module uses. Before X4a
/// this test had to call them `pack_mul` / `pack_add` and carry a comment
/// explaining that the rename was mandatory; the collision it worked around is
/// now namespaced, so the test exercises the shape it is actually about.
#[test]
fn trapping_packed_return_matches_durable_status_module() {
    let _lock = COMPILE_LOCK.lock().unwrap();
    austral_cranelift_bridge::auth::set_allow_all();
    let mul = cps_v1(&body_call_import_2args(
        "Austral.Pervasive::trappingMultiply",
        400,
        65536,
    ));
    // Direct FFI call — `compile` would re-lock the mutex (deadlock).
    let hi = execute(austral_cranelift_bridge::compile_to_function_named(
        mul.as_ptr(),
        mul.len(),
        ptr::null(),
        0,
    ));
    // (statusN << 16) — high 16 bits of the packed return.
    assert_eq!(hi, 400 * 65536);

    let add = cps_v1(&body_call_import_2args(
        "Austral.Pervasive::trappingAdd",
        hi,
        3,
    ));
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

// ── U04: strict entry admission (compile_entry_i64) ─────────────────
// The strict path admits an entry only when the transport can call it:
// 64-bit host, explicit non-empty name, exported function, declared arity
// equal to the caller's, and an Int64 return. Every refusal must return
// null so no pointer reaches the caller — a non-null pointer here *is* the
// invocation permission.

/// Like [`cps_v1_named`] but declares `params` parameters and a return-type
/// byte, so an entry's declared signature can be made to disagree with the
/// caller's contract.
fn cps_v1_sig(fname: &str, params: u32, ret: u8, body_data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x43505331); // magic v1
    write_u32(&mut buf, 1); // func_count
    write_str(&mut buf, fname);
    write_u32(&mut buf, params);
    for i in 0..params {
        write_str(&mut buf, &format!("p{i}"));
    }
    buf.push(ret);
    write_u32(&mut buf, body_data.len() as u32);
    buf.extend_from_slice(body_data);
    buf
}

fn checked(cps: &[u8], name: &str, expected_params: u32) -> *const std::ffi::c_void {
    let _lock = COMPILE_LOCK.lock().unwrap();
    austral_cranelift_bridge::auth::set_allow_all();
    austral_cranelift_bridge::compile_entry_i64(
        cps.as_ptr(),
        cps.len(),
        name.as_ptr(),
        name.len(),
        expected_params,
    )
}

fn last_error() -> String {
    let err = austral_cranelift_bridge::cranelift_last_error();
    if err.is_null() {
        String::new()
    } else {
        unsafe { std::ffi::CStr::from_ptr(err) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Boundary values of the advertised transport, each compiled and executed
/// through the strict path. Independent of the refusal cases: these must
/// actually run and return the exact bit pattern.
#[test]
fn strict_entry_returns_every_i64_boundary_value() {
    for (i, value) in [
        0i64,
        1,
        -1,
        i64::MIN,
        i64::MIN + 1,
        i64::MAX,
        i64::MAX - 1,
        i64::MIN / 2,
        -(1 << 32),
        (1 << 32) - 1,
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("boundary_{i}");
        let cps = cps_v1_sig(&name, 0, 0x01, &body_return_const(value));
        let ptr = checked(&cps, &name, 0);
        assert!(!ptr.is_null(), "value {value}: {}", last_error());
        assert_eq!(execute(ptr), value, "value {value} must round-trip");
    }
}

/// The legitimate case: declared arity matches the caller and the return is
/// Int64, so the pointer is handed back and executes.
#[test]
fn strict_entry_admits_a_matching_signature() {
    let cps = cps_v1_sig("strict_ok", 0, 0x01, &body_return_const(7));
    let ptr = checked(&cps, "strict_ok", 0);
    assert!(!ptr.is_null(), "{}", last_error());
    assert_eq!(execute(ptr), 7);
}

#[test]
fn strict_entry_refuses_an_empty_name() {
    let cps = cps_v1_sig("strict_empty", 0, 0x01, &body_return_const(1));
    let ptr = checked(&cps, "", 0);
    assert!(ptr.is_null(), "an unnamed entry must not be invocable");
    assert!(
        last_error().contains("explicit exported name"),
        "{}",
        last_error()
    );
}

/// The control for every mutable fixture: the module itself is executable,
/// so a refusal below is the signature gate and not a broken buffer.
#[test]
fn strict_entry_refuses_a_missing_name() {
    let cps = cps_v1_sig("strict_present", 0, 0x01, &body_return_const(3));
    let ptr = checked(&cps, "strict_absent", 0);
    assert!(ptr.is_null());
    assert!(
        last_error().contains("strict_absent") && last_error().contains("strict_present"),
        "the diagnostic must name the missing entry and the available one: {}",
        last_error()
    );
}

#[test]
fn strict_entry_refuses_a_wrong_arity() {
    let cps = cps_v1_sig("strict_arity", 1, 0x01, &body_return_const(9));
    let ptr = checked(&cps, "strict_arity", 0);
    assert!(
        ptr.is_null(),
        "a 1-parameter entry must not be called as ()"
    );
    assert!(
        last_error().contains("declares 1 parameter") && last_error().contains("passes 0"),
        "{}",
        last_error()
    );

    // Control: the same buffer through the legacy path compiles and runs, so
    // the refusal above was the declared signature, not the fixture.
    let _lock = COMPILE_LOCK.lock().unwrap();
    austral_cranelift_bridge::auth::set_allow_all();
    let legacy = austral_cranelift_bridge::compile_to_function_named(
        cps.as_ptr(),
        cps.len(),
        "strict_arity".as_ptr(),
        "strict_arity".len(),
    );
    assert!(!legacy.is_null(), "{}", last_error());
}

#[test]
fn strict_entry_refuses_a_non_i64_return() {
    for (byte, named) in [
        (0x00u8, "Unit"),
        (0x02, "Bool"),
        (0x03, "Int32"),
        (0x04, "String"),
        (0x05, "Float64"),
    ] {
        let name = format!("strict_ret_{byte:02x}");
        let cps = cps_v1_sig(&name, 0, byte, &body_return_const(1));
        let ptr = checked(&cps, &name, 0);
        assert!(ptr.is_null(), "return {named} must not be admitted");
        assert!(
            last_error().contains(named),
            "the diagnostic must name {named}: {}",
            last_error()
        );
    }
}

/// A caller expecting one argument cannot be handed a 0-parameter entry.
#[test]
fn strict_entry_refuses_when_the_caller_expects_arguments() {
    let cps = cps_v1_sig("strict_caller_args", 0, 0x01, &body_return_const(1));
    assert!(checked(&cps, "strict_caller_args", 2).is_null());
    assert!(last_error().contains("passes 2"), "{}", last_error());
}

/// A union constructed in tail position is lowered inline, not as an import.
///
/// The Austral lowering emits `return Case(…)` as `0x07` followed by an App of
/// `__union_new`. The tail-call path used to fall through to `declare_import`,
/// so a module returning a one-field union and then a two-field union declared
/// `__union_new` twice with different arities and failed to compile. This test
/// compiles those two shapes in one thread (the engine is process-global, so
/// the declarations persist) and reads the fields back to prove the allocation
/// layout as well.
#[test]
fn tail_position_union_constructions_of_different_arity_compile() {
    let empty = cps_v1_named("tail_union_zero_fields", &body_tail_union(16, 0, &[]));
    let no_fields = compile(&empty);

    let two = cps_v1_named("tail_union_two_fields", &body_tail_union(24, 1, &[7, 9]));
    let with_fields = compile(&two);

    // `__union_new` returns a pointer: tag at offset 0, fields from offset 8.
    let read = |ptr: *const std::ffi::c_void, slot: isize| -> i64 {
        let base = execute(ptr) as *const i64;
        unsafe { *base.offset(slot) }
    };
    assert_eq!(read(no_fields, 0), 0, "tag of the zero-field case");
    assert_eq!(read(with_fields, 0), 1, "tag of the two-field case");
    assert_eq!(read(with_fields, 1), 7, "first field");
    assert_eq!(read(with_fields, 2), 9, "second field");
}

/// A tail self call jumps to the function's restart block instead of
/// recursing. The linear job wrappers await completion with exactly this
/// shape (`poll -> Pending token -> poll again`), and a machine whose device
/// work takes longer than the recursion budget used to overflow the stack.
fn body_tail_self_countdown(fname: &str) -> Vec<u8> {
    let mut then = Vec::new();
    then.push(0x07); // return
    then.push(0x01); // iconst 0
    write_i64(&mut then, 0);

    let mut otherwise = Vec::new();
    otherwise.push(0x07); // return
    otherwise.push(0x04); // App of the function itself
    write_str(&mut otherwise, fname);
    write_u32(&mut otherwise, 1);
    otherwise.push(0x06); // Sub p0 - 1
    otherwise.push(0x02);
    write_str(&mut otherwise, "p0");
    otherwise.push(0x01);
    write_i64(&mut otherwise, 1);

    let mut body = Vec::new();
    body.push(0x08); // if
    body.push(0x14); // CmpEq
    body.push(0x02);
    write_str(&mut body, "p0");
    body.push(0x01);
    write_i64(&mut body, 0);
    write_u32(&mut body, then.len() as u32);
    body.extend(then);
    write_u32(&mut body, otherwise.len() as u32);
    body.extend(otherwise);
    body
}

#[test]
fn a_tail_self_call_does_not_grow_the_stack() {
    let name = "tail_self_countdown";
    let cps = cps_v1_sig(name, 1, 0x01, &body_tail_self_countdown(name));
    let ptr = compile(&cps);
    let countdown: fn(i64) -> i64 = unsafe { std::mem::transmute(ptr) };
    assert_eq!(countdown(5), 0);
    // 1_000_000 frames would overflow an 8 MiB stack; the loop returns.
    assert_eq!(countdown(1_000_000), 0);
}
