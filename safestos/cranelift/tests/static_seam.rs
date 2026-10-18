//! Task 8.6 (docs/spectrum_plan.md §8.4, option 1) — the dlopen → static
//! seam swap at the bridge's import resolver.
//!
//! The umbrella feeds `rs_*` symbols to the JIT by `dlopen(RTLD_NOW |
//! RTLD_GLOBAL)` + cranelift-jit's `dlsym(RTLD_DEFAULT)` fallback. Hosts
//! without `dlopen` — a Hermit unikernel guest, a statically linked
//! image — cannot do that, so the bridge grows a **static import seam**:
//! the image registers its symbols at startup, and every JIT builder
//! consults the registry while resolving imports.
//!
//! What these tests pin (this binary is its own process: registry and
//! thread-local JIT are fresh here, same isolation discipline as
//! `tier1_extern_rust.rs`):
//!
//! * a symbol that exists in NEITHER the CPS binary nor the bridge's
//!   baked `rs_*` table resolves through the registry and EXECUTES;
//! * the manifest gate is unchanged — registering a symbol does NOT
//!   grant permission to call it (UK-4001 under DenyAll);
//! * with a manifest that grants exactly the registered name, the
//!   registered implementation runs end to end;
//! * the C-ABI registrar (what a static image calls) reaches the same
//!   registry;
//! * registration after this thread's JIT is initialized is refused
//!   (it would be silently ineffective otherwise).

use std::ptr;
use std::sync::Mutex;

/// Serializes the process-global auth engine across tests in this binary
/// (tier1_extern_rust.rs discipline).
static STATE_LOCK: Mutex<()> = Mutex::new(());

// ── manifest helpers (tier1_extern_rust.rs shape) ───────────────────

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

fn restore_default_auth() {
    austral_cranelift_bridge::auth::set_auth_engine(Box::new(
        austral_cranelift_bridge::auth::DenyAll,
    ));
    austral_cranelift_bridge::auth::set_deployment_principal(String::new());
}

// ── CPS binary helpers (tier1_extern_rust.rs shape) ─────────────────

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

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

/// Body: `return <callee>(a, b)` over the function's own params.
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

/// Body: `return 42` — a trivial module to initialize this thread's JIT.
fn body_return_const(val: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07); // Return
    body.push(0x01); // iconst
    body.extend_from_slice(&val.to_le_bytes());
    body
}

// ── compile / execute helpers (auth NOT bypassed) ───────────────────

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

/// Independent FNV-1a 64 (so the executed result is checked against the
/// algorithm, not against the implementation under test).
fn fnv1a64(bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h as i64
}

// ── the "statically linked" foreign implementation ──────────────────
//
// In a real static image this function lives in a cross-built rscrate
// archive linked into the guest. Here it is the same shape: a symbol
// present in neither the CPS binary nor RS_FFI_SYMBOLS — it can only be
// reached through the seam.

extern "C" fn seam_checksum(buf: i64, len: i64) -> i64 {
    let slice = unsafe { std::slice::from_raw_parts(buf as *const u8, len as usize) };
    fnv1a64(slice)
}

// ── tests ───────────────────────────────────────────────────────────

/// The seam's contract end to end: registered-but-ungated is denied
/// (UK-4001 under DenyAll — registration is resolution, not permission),
/// and granted-by-name resolves through the registry and executes the
/// statically linked implementation over a live buffer.
#[test]
fn static_seam_resolves_executes_and_the_gate_still_applies() {
    let _lock = STATE_LOCK.lock().unwrap();
    restore_default_auth();

    austral_cranelift_bridge::register_static_symbol(
        "rs_seam_checksum",
        seam_checksum as *const u8,
    )
    .expect("registration succeeds before any compile on this thread");
    assert!(
        austral_cranelift_bridge::registered_static_symbols()
            .iter()
            .any(|(n, a)| n == "rs_seam_checksum" && *a == seam_checksum as *const () as usize),
        "registry must contain the registered seam symbol"
    );

    let cps = cps_v2(
        "run_seam_grant",
        "rs_widget",
        &["buf", "len"],
        &body_tail_call_2params("rs_seam_checksum", "buf", "len"),
    );

    // Gate first: DenyAll must refuse even though the symbol resolves.
    let err = compile_result(&cps).expect_err("DenyAll must deny the seam call");
    assert!(
        err.contains("UK-4001"),
        "expected UK-4001 for the ungranted rs_ seam call, got: {err}"
    );

    // Grant exactly the registered name: compile + execute through the seam.
    load_manifest("rs_widget", &["rs_seam_checksum"]);
    let ptr = compile_result(&cps).expect("a granted seam call must compile");
    let buf = b"au!";
    let got = execute_2(ptr, buf.as_ptr() as i64, buf.len() as i64);
    assert_eq!(
        got,
        fnv1a64(buf),
        "the statically linked implementation ran"
    );

    restore_default_auth();
}

/// The C-ABI registrar (what a Hermit-side image calls in place of dlopen)
/// reaches the same registry.
#[test]
fn c_abi_registrar_reaches_the_registry() {
    let _lock = STATE_LOCK.lock().unwrap();

    extern "C" fn seam_version() -> i64 {
        7
    }
    let name = std::ffi::CString::new("rs_seam_version").unwrap();
    let rc = austral_cranelift_bridge::austral_register_static_symbol(
        name.as_ptr(),
        seam_version as *const u8,
    );
    assert_eq!(rc, 1, "the C-ABI registrar accepts a valid symbol");
    assert!(austral_cranelift_bridge::registered_static_symbols()
        .iter()
        .any(|(n, _)| n == "rs_seam_version"));
    // Invalid arguments are refused with the documented codes. The address
    // is never consulted — `name` is null-checked first — so it is a
    // `dangling` placeholder rather than a fabricated address, which is
    // also what clippy (1.99, `-D warnings`) asks for.
    assert_eq!(
        austral_cranelift_bridge::austral_register_static_symbol(
            ptr::null(),
            std::ptr::dangling::<u8>(),
        ),
        0,
        "a null name is an invalid argument"
    );
}

/// Registration after this thread's JIT is initialized is refused rather
/// than silently ineffective (the JIT consumes its table at init).
#[test]
fn late_registration_is_refused() {
    let _lock = STATE_LOCK.lock().unwrap();

    // Any successful compile initializes this thread's JIT.
    let cps = cps_v2("run_late_probe", "rs_widget", &[], &body_return_const(42));
    let ptr = compile_result(&cps).expect("trivial module compiles");
    assert_eq!(execute_0(ptr), 42);

    extern "C" fn too_late() -> i64 {
        0
    }
    let err =
        austral_cranelift_bridge::register_static_symbol("rs_too_late", too_late as *const u8)
            .expect_err("late registration must be refused");
    assert!(
        err.contains("already initialized"),
        "the refusal must name the reason, got: {err}"
    );
}
