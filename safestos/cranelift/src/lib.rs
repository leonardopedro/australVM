use cranelift_jit::{JITModule, JITBuilder};
use cranelift_module::Module;
use std::cell::RefCell;
use std::ffi::{c_void, CString};
use cranelift_codegen::settings::Configurable;

pub mod auth;
#[cfg(feature = "cedar")]
pub mod policy;
pub mod cps;

#[cfg(feature = "cedar")]
use policy::CEDAR_ENGINE;
#[cfg(feature = "cedar")]
use std::ffi::CStr;

thread_local! {
    static JIT: RefCell<Option<JITModule>> = RefCell::new(None);
    static LAST_ERROR: RefCell<Option<CString>> = RefCell::new(None);
}

fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Returns a pointer to the last error string (valid until next call), or null if no error.
#[no_mangle]
pub extern "C" fn cranelift_last_error() -> *const std::ffi::c_char {
    LAST_ERROR.with(|e| {
        e.borrow().as_ref().map(|s| s.as_ptr()).unwrap_or(std::ptr::null())
    })
}

/// Clear the last error.
#[no_mangle]
pub extern "C" fn cranelift_clear_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = None);
}

extern "C" {
    fn au_print_int(i: i64);
    fn au_exit(code: i64);
    fn au_alloc(size: i64) -> *mut u8;
    fn au_free(ptr: *mut u8);
    fn cell_swap(old_id: u64, new_desc: *const c_void) -> bool;
    fn cell_can_replace(old: *const c_void, new: *const c_void) -> bool;
}

#[no_mangle]
pub extern "C" fn __au_swap_module(old_id: u64, new_desc: *const c_void) -> i64 {
    unsafe {
        if cell_swap(old_id, new_desc) { 1 } else { 0 }
    }
}

#[no_mangle]
pub extern "C" fn cranelift_init() -> i64 {
    JIT.with(|cell| {
        if cell.borrow().is_some() { return 1; }

        match (|| -> Result<JITModule, String> {
            let target_builder = cranelift_native::builder()
                .map_err(|e| format!("Native builder failed: {}", e))?;
            let flag_builder = cranelift_codegen::settings::builder();
            let isa = target_builder
                .finish(cranelift_codegen::settings::Flags::new(flag_builder))
                .map_err(|e| format!("ISA finish failed: {}", e))?;
            let mut builder =
                JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());

            // Register runtime primitives
            builder.symbol("au_print_int", au_print_int as *const u8);
            builder.symbol("au_exit",      au_exit      as *const u8);
            builder.symbol("au_alloc",     au_alloc     as *const u8);
            builder.symbol("au_free",      au_free      as *const u8);

            // Register unfer kernel symbols (uk_*) for JIT-compiled modules.
            // Access is gated by the manifest auth engine — uk_ is NOT in the
            // check_call_permission whitelist, so modules need explicit grants.
            #[cfg(feature = "unfer-kernel")]
            {
                builder.symbol("uk_version",           unfer_ffi::uk_version           as *const u8);
                builder.symbol("uk_init",              unfer_ffi::uk_init              as *const u8);
                builder.symbol("uk_model_create",      unfer_ffi::uk_model_create      as *const u8);
                builder.symbol("uk_model_free",        unfer_ffi::uk_model_free        as *const u8);
                builder.symbol("uk_set_prior",         unfer_ffi::uk_set_prior         as *const u8);
                builder.symbol("uk_set_hamiltonian",   unfer_ffi::uk_set_hamiltonian   as *const u8);
                builder.symbol("uk_evolve",             unfer_ffi::uk_evolve             as *const u8);
                builder.symbol("uk_condition",          unfer_ffi::uk_condition          as *const u8);
                builder.symbol("uk_event_probability", unfer_ffi::uk_event_probability as *const u8);
                builder.symbol("uk_observe",           unfer_ffi::uk_observe           as *const u8);
                builder.symbol("uk_get_result",        unfer_ffi::uk_get_result        as *const u8);
                builder.symbol("uk_last_error",         unfer_ffi::uk_last_error         as *const u8);
            }

            Ok(JITModule::new(builder))
        })() {
            Ok(jit) => {
                cell.replace(Some(jit));
                1
            }
            Err(e) => {
                let msg = format!("JIT init failed: {}", e);
                set_last_error(&msg);
                eprintln!("CPS: {}", msg);
                0
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn compile_to_function_named(
    ir_ptr:   *const u8,
    ir_len:   usize,
    name_ptr: *const u8,
    name_len: usize,
) -> *const c_void {
    cranelift_clear_error();

    if JIT.with(|c| c.borrow().is_none()) {
        if cranelift_init() == 0 {
            return std::ptr::null();
        }
    }

    if ir_ptr.is_null() || ir_len == 0 {
        set_last_error("Empty IR passed to compiler");
        return std::ptr::null();
    }

    let ir_slice = unsafe { std::slice::from_raw_parts(ir_ptr, ir_len) };

    let name_str: &str = if name_ptr.is_null() || name_len == 0 {
        ""
    } else {
        let slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
        match std::str::from_utf8(slice) {
            Ok(s) => s.trim_end_matches('\0'),
            Err(_) => {
                set_last_error("Invalid UTF-8 in function name");
                return std::ptr::null();
            }
        }
    };

    JIT.with(|cell| {
        let mut opt = cell.borrow_mut();
        let jit = opt.as_mut().unwrap();

        match cps::compile_cps_to_clif(jit, ir_slice) {
            Ok(module) => {
                match jit.finalize_definitions() {
                    Err(e) => {
                        let msg = format!("Finalize failed: {}", e);
                        set_last_error(&msg);
                        eprintln!("CPS: {}", msg);
                        std::ptr::null()
                    }
                    Ok(_) => {
                        // Entry selection. With an explicit name, resolve it. With
                        // no name (the per-module compile path), execute only the
                        // conventional `run` entry point if present -- never a
                        // random function from the module's table. Library modules
                        // (e.g. UnferKernel) have no `run`: their functions are
                        // still defined for cross-module linking, but executing one
                        // with garbage arguments could dereference a non-pointer and
                        // crash. Returning null here makes the caller skip execution.
                        let (func_id, quiet_skip) = if name_str.is_empty() {
                            match module.name_map.get("run").copied() {
                                Some(fid) => (Some(fid), false),
                                None => (None, true),
                            }
                        } else {
                            (module.name_map.get(name_str).copied(), false)
                        };

                        if let Some(fid) = func_id {
                            let ptr = jit.get_finalized_function(fid) as *const c_void;
                            eprintln!("CPS: SUCCESS compiled at {:?}", ptr);
                            ptr
                        } else if quiet_skip {
                            // No entry point to run in this (library) module.
                            std::ptr::null()
                        } else {
                            let avail: Vec<&String> = module.name_map.keys().collect();
                            let msg = format!(
                                "Function '{}' not found. Available: [{}]",
                                name_str,
                                avail.iter().map(|s| s.as_str())
                                     .collect::<Vec<_>>().join(", ")
                            );
                            set_last_error(&msg);
                            eprintln!("CPS: {}", msg);
                            std::ptr::null()
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("Compilation error: {}", e);
                set_last_error(&msg);
                eprintln!("CPS: compile_to_function_named failed: {}", e);
                std::ptr::null()
            }
        }
    })
}

/// Compile first function (alternative API — name resolved automatically).
#[no_mangle]
pub extern "C" fn compile_to_function(ir_ptr: *const u8, ir_len: usize) -> *const c_void {
    compile_to_function_named(ir_ptr, ir_len, std::ptr::null(), 0)
}

#[no_mangle] pub extern "C" fn cranelift_version()  -> u32  { 0x0083000 }
#[no_mangle] pub extern "C" fn cranelift_is_ready() -> i64  {
    JIT.with(|c| if c.borrow().is_some() { 1 } else { 0 })
}
#[no_mangle] pub extern "C" fn cranelift_shutdown() {
    JIT.with(|c| *c.borrow_mut() = None);
}

#[no_mangle]
pub extern "C" fn execute_function(ptr: *const c_void) -> i64 {
    if ptr.is_null() { return -1; }
    let f: fn() -> i64 = unsafe { std::mem::transmute(ptr) };
    f()
}

#[no_mangle]
pub extern "C" fn execute_function_1(ptr: *const c_void, arg1: i64) -> i64 {
    if ptr.is_null() { return -1; }
    let f: fn(i64) -> i64 = unsafe { std::mem::transmute(ptr) };
    f(arg1)
}

#[no_mangle]
pub extern "C" fn execute_function_2(ptr: *const c_void, arg1: i64, arg2: i64) -> i64 {
    if ptr.is_null() { return -1; }
    let f: fn(i64, i64) -> i64 = unsafe { std::mem::transmute(ptr) };
    f(arg1, arg2)
}

#[cfg(feature = "cedar")]
#[no_mangle]
pub extern "C" fn au_cedar_load_policy(policy_str: *const std::ffi::c_char) -> i64 {
    if policy_str.is_null() {
        set_last_error("Null pointer passed to au_cedar_load_policy");
        return 0;
    }
    let c_str = unsafe { CStr::from_ptr(policy_str) };
    let policy = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("Invalid UTF-8 in policy string");
            return 0;
        }
    };

    CEDAR_ENGINE.with(|engine| {
        match engine.borrow_mut().load_policy(policy) {
            Ok(_) => 1,
            Err(e) => {
                set_last_error(&e);
                0
            }
        }
    })
}

#[cfg(feature = "cedar")]
#[no_mangle]
pub extern "C" fn au_cedar_check_runtime(
    principal_ptr: *const std::ffi::c_char,
    action_ptr: *const std::ffi::c_char,
    resource_ptr: *const std::ffi::c_char,
) -> i64 {
    if principal_ptr.is_null() || action_ptr.is_null() || resource_ptr.is_null() {
        return 0; // Deny by default on null
    }
    let principal = unsafe { CStr::from_ptr(principal_ptr) }.to_string_lossy();
    let action = unsafe { CStr::from_ptr(action_ptr) }.to_string_lossy();
    let resource = unsafe { CStr::from_ptr(resource_ptr) }.to_string_lossy();

    CEDAR_ENGINE.with(|engine| {
        match engine.borrow().is_authorized(&principal, &action, &resource) {
            Ok(true) => 1,  // Allowed
            _ => 0,         // Denied or error
        }
    })
}
// When Cedar is compiled out, the OCaml bridge still links against these
// symbols (it declares `external ... = "ocaml_cedar_load_policy"`). Provide
// no-op stubs so `--no-default-features` builds remain link-compatible; the
// active authorizer in that configuration is the `auth.rs` engine (ManifestAuth
// or AllowAll), not Cedar, so policy loads are intentionally ignored and runtime
// checks defer to `auth::check` (which returns Allow under the AllowAll default).
#[cfg(not(feature = "cedar"))]
#[no_mangle]
pub extern "C" fn au_cedar_load_policy(_policy_str: *const std::ffi::c_char) -> i64 {
    1
}

#[cfg(not(feature = "cedar"))]
#[no_mangle]
pub extern "C" fn au_cedar_check_runtime(
    principal_ptr: *const std::ffi::c_char,
    action_ptr: *const std::ffi::c_char,
    resource_ptr: *const std::ffi::c_char,
) -> i64 {
    if principal_ptr.is_null() || action_ptr.is_null() || resource_ptr.is_null() {
        return 0;
    }
    let principal = unsafe { std::ffi::CStr::from_ptr(principal_ptr) }.to_string_lossy();
    let action = unsafe { std::ffi::CStr::from_ptr(action_ptr) }.to_string_lossy();
    let resource = unsafe { std::ffi::CStr::from_ptr(resource_ptr) }.to_string_lossy();
    match crate::auth::check(&principal, &action, &resource) {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "C" fn au_set_cell_jit_ptr(desc_ptr: *mut u8, jit_ptr: *const std::ffi::c_void) {
    if desc_ptr.is_null() { return; }
    // Offset of _jit_fn_ptr in CellDescriptor (vm.h) is 64 bytes
    unsafe {
        let ptr = desc_ptr.add(64) as *mut *const std::ffi::c_void;
        *ptr = jit_ptr;
    }
}

#[no_mangle]
pub extern "C" fn au_cell_swap(old_id: u64, new_desc: *mut std::ffi::c_void) -> bool {
    unsafe { cell_swap(old_id, new_desc) }
}

/// Rust wrapper for the C `cell_can_replace` compatibility gate
/// (cell_loader.c:63). Returns `true` when the new descriptor is a valid
/// replacement for the old one: same `type_hash` AND new caps ⊆ old caps.
/// Used by the hot-swap positive-path test (P5 #32).
pub fn au_cell_can_replace(old: *const std::ffi::c_void, new: *const std::ffi::c_void) -> bool {
    unsafe { cell_can_replace(old, new) }
}

#[cfg(all(test, feature = "unfer-kernel"))]
mod tests {
    #[test]
    fn uk_version_linkage() {
        assert_eq!(unfer_ffi::uk_version(), 1);
    }

    #[test]
    fn uk_init_linkage() {
        assert_eq!(unfer_ffi::uk_init(std::ptr::null(), 0), 0);
    }
}

#[cfg(test)]
mod hotswap_tests {
    use super::au_cell_swap;

    /// P5 #32: verify the hot-swap gate is callable and guards invalid IDs.
    ///
    /// `au_cell_swap` wraps `cell_swap` (cell_loader.c:82) which bounds-checks
    /// `old_id` against the live `cell_table` (size `cell_count`, starts at 0).
    /// ID 0 is always out of range; U64::MAX is always out of range. The C
    /// function therefore returns `false` without touching the table.
    ///
    /// Full end-to-end hot-swap (load cell V1 → run → swap to V2 → run again)
    /// requires a `CellDescriptor` from a compiled `.so` (cell_loader.c:cell_load)
    /// and is tested at the shell level (`test_integration.sh`); this test
    /// verifies the Rust→C linkage and the guard path without the C scheduler.
    #[test]
    fn hotswap_rejects_invalid_cell_id() {
        assert!(
            !au_cell_swap(0, std::ptr::null_mut()),
            "cell_id 0 is below the 1-indexed table"
        );
        assert!(
            !au_cell_swap(u64::MAX, std::ptr::null_mut()),
            "cell_id u64::MAX exceeds cell_count (0 at test time)"
        );
    }

    /// P5 #32: verify the hot-swap **compatibility gate** (`cell_can_replace`).
    ///
    /// This is the decision function called by `cell_swap` before replacing a
    /// cell in the live table. It checks: (1) both pointers non-NULL, (2)
    /// `type_hash` strings match exactly, (3) `new->required_caps` ⊆
    /// `old->required_caps`. We construct minimal `#[repr(C)]` structs matching
    /// the first two fields of the C `CellDescriptor` (vm.h:65) — the only
    /// fields `cell_can_replace` reads — and exercise all four decision paths.
    use std::ffi::CString;
    use std::os::raw::c_char;

    #[repr(C)]
    struct TestCellDesc {
        type_hash: *const c_char,
        required_caps: u64,
    }

    fn desc(hash: &str, caps: u64) -> TestCellDesc {
        TestCellDesc {
            type_hash: CString::new(hash).unwrap().into_raw(),
            required_caps: caps,
        }
    }

    fn reclaim(d: &TestCellDesc) {
        unsafe { let _ = CString::from_raw(d.type_hash as *mut c_char); }
    }

    #[test]
    fn hotswap_gate_accepts_compatible() {
        let old = desc("abc123", 0b111);
        let new = desc("abc123", 0b101);
        assert!(
            super::au_cell_can_replace(
                &old as *const _ as *const std::ffi::c_void,
                &new as *const _ as *const std::ffi::c_void,
            ),
            "same type_hash + subset caps → compatible"
        );
        reclaim(&old);
        reclaim(&new);
    }

    #[test]
    fn hotswap_gate_rejects_null() {
        assert!(
            !super::au_cell_can_replace(std::ptr::null(), std::ptr::null()),
            "NULL descriptors → incompatible"
        );
    }

    #[test]
    fn hotswap_gate_rejects_type_mismatch() {
        let old = desc("abc123", 0b111);
        let new = desc("xyz789", 0b111);
        assert!(
            !super::au_cell_can_replace(
                &old as *const _ as *const std::ffi::c_void,
                &new as *const _ as *const std::ffi::c_void,
            ),
            "different type_hash → incompatible"
        );
        reclaim(&old);
        reclaim(&new);
    }

    #[test]
    fn hotswap_gate_rejects_caps_escalation() {
        let old = desc("abc123", 0b101);
        let new = desc("abc123", 0b111);
        assert!(
            !super::au_cell_can_replace(
                &old as *const _ as *const std::ffi::c_void,
                &new as *const _ as *const std::ffi::c_void,
            ),
            "new caps ⊃ old caps → incompatible (capability escalation)"
        );
        reclaim(&old);
        reclaim(&new);
    }
}
