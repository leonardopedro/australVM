use cranelift_jit::{JITModule, JITBuilder};
use std::cell::RefCell;
use std::ffi::{c_void, CString};

pub use cps::CpsModule;

fn cps_debug(msg: &str) {
    if std::env::var("CPS_DEBUG").is_ok() {
        eprintln!("CPS: {}", msg);
    }
}

pub mod auth;
#[cfg(feature = "cedar")]
pub mod policy;
#[cfg(feature = "arctic-auth")]
pub mod arctic_auth;
pub mod cps;
pub mod module;
pub mod tidepool_mod;
pub mod capstd_mod;
pub mod federation;
#[cfg(feature = "ecmascript")]
pub mod ecma;
#[cfg(feature = "sandbox")]
pub mod sandbox;

#[cfg(feature = "cedar")]
use policy::CEDAR_ENGINE;
#[cfg(feature = "cedar")]
use std::ffi::CStr;

thread_local! {
    static JIT: RefCell<Option<JITModule>> = RefCell::new(None);
    static LAST_ERROR: RefCell<Option<CString>> = RefCell::new(None);
    static CURRENT_MODULE: RefCell<Option<CpsModule>> = RefCell::new(None);
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
    fn cell_swap(old_id: u64, new_desc: *const c_void) -> bool;
    fn cell_can_replace(old: *const c_void, new: *const c_void) -> bool;
    fn cell_set_jit_fn_ptr(desc: *mut u8, ptr: *const std::ffi::c_void);
}

// The `au_*` runtime primitives the JIT registers for compiled Austral
// code. They are defined HERE (not extern) so the bridge `.so` is
// self-contained — no undefined symbols for host binaries to supply (which
// otherwise breaks linking on binutils configs that enforce shared-library
// symbol resolution). The OCaml side's `rust_bridge.c` provides identical
// definitions for its own CAMLprims; ELF interposition makes the
// executable's copies win at runtime, so the behavior is unchanged.
#[no_mangle]
pub extern "C" fn au_print_int(i: i64) {
    use std::io::Write;
    println!("{i}");
    let _ = std::io::stdout().flush();
}

#[no_mangle]
pub extern "C" fn au_exit(code: i64) {
    eprintln!("Austral: Exit with code {code}");
}

#[no_mangle]
pub extern "C" fn au_alloc(size: i64) -> *mut u8 {
    if size <= 0 {
        return std::ptr::null_mut();
    }
    unsafe { libc::malloc(size as usize) as *mut u8 }
}

#[no_mangle]
pub extern "C" fn au_free(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe { libc::free(ptr as *mut libc::c_void) };
    }
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

            // Register CPS-level intrinsics (handled inline in cps.rs, but
            // need entries in the symbol table so Cranelift JIT finalization
            // can find them via dlsym when a function body references them).
            // These are no-ops — the inline compiler never emits a real call
            // to them; they're only needed to satisfy the symbol resolver.
            builder.symbol("__union_new",   au_exit as *const u8);
            builder.symbol("__record_new",  au_exit as *const u8);
            builder.symbol("__slot_get",    au_exit as *const u8);

            // Register unfer kernel symbols (uk_*) for JIT-compiled modules.
            // Access is gated by the manifest auth engine — uk_ is NOT in the
            // check_call_permission whitelist, so modules need explicit grants.
            #[cfg(feature = "unfer-kernel")]
            register_unfer_symbols(&mut builder);

            // Register Zenodo-Loro storage symbols (uz_*) for modules that
            // persist Loro CRDT documents on Zenodo with incremental deltas.
            // Access is gated by the manifest auth engine — modules need
            // explicit "zenodo" grants in their module.toml.
            #[cfg(feature = "zenodo-store")]
            register_zenodo_symbols(&mut builder);

            Ok(JITModule::new(builder))
        })() {
            Ok(jit) => {
                cell.replace(Some(jit));
                1
            }
            Err(e) => {
                let msg = format!("JIT init failed: {}", e);
                set_last_error(&msg);
                cps_debug(&msg);
                0
            }
        }
    })
}

// ── Data-driven unfer symbol tables ─────────────────────────────────
//
// These tables enumerate every uk_*/uz_* symbol the bridge registers.
// Tests iterate them to auto-sync against unfer's EXPECTED_SYMBOLS.txt.

/// A kernel symbol entry: the name Cranelift JIT modules use (e.g. `uk_evolve`)
/// and an opaque pointer to the corresponding `pub extern "C" fn` in `unfer_ffi`.
/// Pointers are stored as `*const u8` because the function signatures differ;
/// `JITBuilder::symbol()` only needs the address for linking.
pub(crate) struct KernelSymbol {
    pub(crate) name: &'static str,
    pub(crate) addr: *const u8,
}

// Function pointers are cast to `*const u8` for `JITBuilder::symbol()`.
// On all Cranelift targets (x86-64, aarch64) function and data pointers
// have the same size, so the cast is valid.

/// Auto-generated: call `cargo test` to verify this matches
/// `unfer_ffi/EXPECTED_SYMBOLS.txt`. Adding a new `uk_*` function to unfer
/// requires adding it here and in the expected-symbols file.
#[cfg(feature = "unfer-kernel")]
pub(crate) const UNFER_SYMBOLS: &[KernelSymbol] = &[
    KernelSymbol { name: "uk_version",           addr: unfer_ffi::uk_version           as *const u8 },
    KernelSymbol { name: "uk_init",              addr: unfer_ffi::uk_init              as *const u8 },
    KernelSymbol { name: "uk_model_create",      addr: unfer_ffi::uk_model_create      as *const u8 },
    KernelSymbol { name: "uk_model_free",        addr: unfer_ffi::uk_model_free        as *const u8 },
    KernelSymbol { name: "uk_set_prior",         addr: unfer_ffi::uk_set_prior         as *const u8 },
    KernelSymbol { name: "uk_set_hamiltonian",   addr: unfer_ffi::uk_set_hamiltonian   as *const u8 },
    KernelSymbol { name: "uk_evolve",            addr: unfer_ffi::uk_evolve             as *const u8 },
    KernelSymbol { name: "uk_condition",         addr: unfer_ffi::uk_condition          as *const u8 },
    KernelSymbol { name: "uk_event_probability", addr: unfer_ffi::uk_event_probability as *const u8 },
    KernelSymbol { name: "uk_observe",           addr: unfer_ffi::uk_observe           as *const u8 },
    KernelSymbol { name: "uk_get_result",        addr: unfer_ffi::uk_get_result        as *const u8 },
    KernelSymbol { name: "uk_last_error",        addr: unfer_ffi::uk_last_error         as *const u8 },
    // Logos CNL->UNF compilation (unique-normal-form via interaction-net reduction).
    KernelSymbol { name: "uk_logos_compile",     addr: unfer_ffi::uk_logos_compile      as *const u8 },
    // Austral->deltanet UNF translation (the austral/australVM-language side
    // of the unique-normal-form pipeline, used by the Deltanet_plugin pass).
    KernelSymbol { name: "uk_austral_unf",       addr: unfer_ffi::uk_austral_unf        as *const u8 },
    KernelSymbol { name: "uk_snapshot",          addr: unfer_ffi::uk_snapshot          as *const u8 },
    KernelSymbol { name: "uk_restore",           addr: unfer_ffi::uk_restore           as *const u8 },
    KernelSymbol { name: "uk_subscribe",         addr: unfer_ffi::uk_subscribe         as *const u8 },
    KernelSymbol { name: "uk_poll",              addr: unfer_ffi::uk_poll              as *const u8 },
    // H9: deployment security posture (S22 admin seam).
    KernelSymbol { name: "uk_posture_get",       addr: unfer_ffi::uk_posture_get       as *const u8 },
    KernelSymbol { name: "uk_posture_set",       addr: unfer_ffi::uk_posture_set       as *const u8 },
    // S29: Lean4 proof verification (nanoda_lib external type checker).
    KernelSymbol { name: "uk_proof_verify",      addr: unfer_ffi::uk_proof_verify      as *const u8 },
    // S30: Cadabra2 symbolic coupling (external CAS subprocess).
    KernelSymbol { name: "uk_symbolic_simplify", addr: unfer_ffi::uk_symbolic_simplify as *const u8 },
    // S36: WhyML codegen for the compiler-extension cycle (external Why3 toolchain).
    KernelSymbol { name: "uk_whyml_emit",       addr: unfer_ffi::uk_whyml_emit       as *const u8 },
    KernelSymbol { name: "uk_bayesian_update",   addr: unfer_ffi::uk_bayesian_update   as *const u8 },
    KernelSymbol { name: "uk_belief_propagation",addr: unfer_ffi::uk_belief_propagation as *const u8 },
    KernelSymbol { name: "uk_buf_free",          addr: unfer_ffi::uk_buf_free           as *const u8 },
    KernelSymbol { name: "uk_ode_analyze",       addr: unfer_ffi::uk_ode_analyze        as *const u8 },
    KernelSymbol { name: "uk_ode_measure_original", addr: unfer_ffi::uk_ode_measure_original as *const u8 },
    // S4: deferred approval + local simulation (effects grant namespace).
    KernelSymbol { name: "uk_action_submit", addr: unfer_ffi::uk_action_submit     as *const u8 },
    KernelSymbol { name: "uk_action_apply",  addr: unfer_ffi::uk_action_apply      as *const u8 },
    KernelSymbol { name: "uk_action_reject", addr: unfer_ffi::uk_action_reject     as *const u8 },
    KernelSymbol { name: "uk_action_revert", addr: unfer_ffi::uk_action_revert     as *const u8 },
    KernelSymbol { name: "uk_action_get",    addr: unfer_ffi::uk_action_get        as *const u8 },
    KernelSymbol { name: "uk_action_list",   addr: unfer_ffi::uk_action_list       as *const u8 },
    // S5: .cell blueprint archives (instance isolation + blueprints).
    KernelSymbol { name: "uk_blueprint_export",      addr: unfer_ffi::uk_blueprint_export      as *const u8 },
    KernelSymbol { name: "uk_blueprint_instantiate", addr: unfer_ffi::uk_blueprint_instantiate as *const u8 },
    // S6: agent accountability + audit (GatekeeperCaller tags, audit trail, AgentSpawner).
    KernelSymbol { name: "uk_audit_list",     addr: unfer_ffi::uk_audit_list     as *const u8 },
    KernelSymbol { name: "uk_audit_clear",    addr: unfer_ffi::uk_audit_clear    as *const u8 },
    KernelSymbol { name: "uk_agent_spawn",    addr: unfer_ffi::uk_agent_spawn    as *const u8 },
    KernelSymbol { name: "uk_agent_list",     addr: unfer_ffi::uk_agent_list     as *const u8 },
    KernelSymbol { name: "uk_agent_kill",     addr: unfer_ffi::uk_agent_kill     as *const u8 },
    KernelSymbol { name: "uk_agent_grants",   addr: unfer_ffi::uk_agent_grants   as *const u8 },
    // S5: .cell blueprint archives — remaining blueprint surface.
    KernelSymbol { name: "uk_blueprint_cell",           addr: unfer_ffi::uk_blueprint_cell           as *const u8 },
    KernelSymbol { name: "uk_blueprint_export_gadget",  addr: unfer_ffi::uk_blueprint_export_gadget  as *const u8 },
    KernelSymbol { name: "uk_blueprint_get_by_id",      addr: unfer_ffi::uk_blueprint_get_by_id      as *const u8 },
    KernelSymbol { name: "uk_blueprint_import",         addr: unfer_ffi::uk_blueprint_import         as *const u8 },
    KernelSymbol { name: "uk_blueprint_list",           addr: unfer_ffi::uk_blueprint_list           as *const u8 },
    // S7: observability + issue reporting.
    KernelSymbol { name: "uk_observability", addr: unfer_ffi::uk_observability as *const u8 },
    KernelSymbol { name: "uk_report_issue",  addr: unfer_ffi::uk_report_issue  as *const u8 },
    // S22: owner-scoped owner log (operator console).
    KernelSymbol { name: "uk_owner_clear", addr: unfer_ffi::uk_owner_clear as *const u8 },
    KernelSymbol { name: "uk_owner_list",  addr: unfer_ffi::uk_owner_list  as *const u8 },
    KernelSymbol { name: "uk_owner_log",   addr: unfer_ffi::uk_owner_log   as *const u8 },
    // S21: vetted identity registry (console-only).
    KernelSymbol { name: "uk_registry_vetted", addr: unfer_ffi::uk_registry_vetted as *const u8 },
    // S25: read-only metering status + S27 credential vault.
    KernelSymbol { name: "uk_meter_status", addr: unfer_ffi::uk_meter_status as *const u8 },
    KernelSymbol { name: "uk_secret_put",    addr: unfer_ffi::uk_secret_put    as *const u8 },
    KernelSymbol { name: "uk_secret_get",    addr: unfer_ffi::uk_secret_get    as *const u8 },
    KernelSymbol { name: "uk_secret_revoke", addr: unfer_ffi::uk_secret_revoke as *const u8 },
    // S18: resource introductions + caps + forfeit.
    KernelSymbol { name: "uk_request_resource", addr: unfer_ffi::uk_request_resource as *const u8 },
    KernelSymbol { name: "uk_resource_introduce", addr: unfer_ffi::uk_resource_introduce as *const u8 },
    KernelSymbol { name: "uk_resource_pending",   addr: unfer_ffi::uk_resource_pending   as *const u8 },
    KernelSymbol { name: "uk_resource_use",       addr: unfer_ffi::uk_resource_use       as *const u8 },
    KernelSymbol { name: "uk_resource_forfeit",   addr: unfer_ffi::uk_resource_forfeit   as *const u8 },
    // Plan R: certificate ledger ops (ReFi exchange).
    KernelSymbol { name: "uk_cert_set_authority", addr: unfer_ffi::uk_cert_set_authority as *const u8 },
    KernelSymbol { name: "uk_cert_mint",          addr: unfer_ffi::uk_cert_mint          as *const u8 },
    KernelSymbol { name: "uk_cert_mint_request",  addr: unfer_ffi::uk_cert_mint_request  as *const u8 },
    KernelSymbol { name: "uk_cert_transfer",      addr: unfer_ffi::uk_cert_transfer      as *const u8 },
    KernelSymbol { name: "uk_cert_burn",          addr: unfer_ffi::uk_cert_burn          as *const u8 },
    KernelSymbol { name: "uk_cert_status",        addr: unfer_ffi::uk_cert_status        as *const u8 },
    KernelSymbol { name: "uk_cert_root",          addr: unfer_ffi::uk_cert_root          as *const u8 },
    // Plan R: unified auction (Prebid-model, carbon credits + publicity inventory).
    KernelSymbol { name: "uk_auction_open",   addr: unfer_ffi::uk_auction_open   as *const u8 },
    KernelSymbol { name: "uk_auction_bid",    addr: unfer_ffi::uk_auction_bid    as *const u8 },
    KernelSymbol { name: "uk_auction_close",  addr: unfer_ffi::uk_auction_close  as *const u8 },
    KernelSymbol { name: "uk_auction_report", addr: unfer_ffi::uk_auction_report as *const u8 },
    // Plan R: gate approval surface (secondary market settlement).
    KernelSymbol { name: "uk_gate_list_pending", addr: unfer_ffi::uk_gate_list_pending as *const u8 },
    KernelSymbol { name: "uk_gate_approve",      addr: unfer_ffi::uk_gate_approve      as *const u8 },
    KernelSymbol { name: "uk_gate_reject",       addr: unfer_ffi::uk_gate_reject       as *const u8 },
    // H3: event-sourced session fork + compaction.
    KernelSymbol { name: "uk_session_fork",    addr: unfer_ffi::uk_session_fork    as *const u8 },
    KernelSymbol { name: "uk_session_compact", addr: unfer_ffi::uk_session_compact as *const u8 },
    // H13: skills registry (discovery/sharing over the module path).
    KernelSymbol { name: "uk_skill_get",         addr: unfer_ffi::uk_skill_get         as *const u8 },
    KernelSymbol { name: "uk_skill_list",        addr: unfer_ffi::uk_skill_list        as *const u8 },
    KernelSymbol { name: "uk_skill_pack_import", addr: unfer_ffi::uk_skill_pack_import as *const u8 },
    KernelSymbol { name: "uk_skill_register",    addr: unfer_ffi::uk_skill_register    as *const u8 },
];

#[cfg(feature = "unfer-kernel")]
fn register_unfer_symbols(builder: &mut JITBuilder) {
    for sym in UNFER_SYMBOLS {
        builder.symbol(sym.name, sym.addr);
    }
}

/// Return the set of uk_* symbol names registered in the bridge.
#[cfg(feature = "unfer-kernel")]
pub fn registered_unfer_symbols() -> Vec<&'static str> {
    UNFER_SYMBOLS.iter().map(|s| s.name).collect()
}

#[cfg(feature = "zenodo-store")]
pub(crate) const ZENODO_SYMBOLS: &[KernelSymbol] = &[
    KernelSymbol { name: "uz_init",          addr: unfer_ffi::zenodo::uz_init          as *const u8 },
    KernelSymbol { name: "uz_push",          addr: unfer_ffi::zenodo::uz_push          as *const u8 },
    KernelSymbol { name: "uz_pull",          addr: unfer_ffi::zenodo::uz_pull          as *const u8 },
    KernelSymbol { name: "uz_manifest_json", addr: unfer_ffi::zenodo::uz_manifest_json as *const u8 },
    KernelSymbol { name: "uz_last_error",    addr: unfer_ffi::zenodo::uz_last_error    as *const u8 },
];

#[cfg(feature = "zenodo-store")]
fn register_zenodo_symbols(builder: &mut JITBuilder) {
    for sym in ZENODO_SYMBOLS {
        builder.symbol(sym.name, sym.addr);
    }
}

/// Return the set of uz_* symbol names registered in the bridge.
#[cfg(feature = "zenodo-store")]
pub fn registered_zenodo_symbols() -> Vec<&'static str> {
    ZENODO_SYMBOLS.iter().map(|s| s.name).collect()
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
                        cps_debug(&msg);
                        std::ptr::null()
                    }
                    Ok(_) => {
                        // Save module handle for re-lookup via lookup_function().
                        let name_map = module.name_map.clone();
                        CURRENT_MODULE.with(|m| *m.borrow_mut() = Some(module));

                        // Entry selection. With an explicit name, resolve it. With
                        // no name (the per-module compile path), execute only the
                        // conventional `run` entry point if present -- never a
                        // random function from the module's table. Library modules
                        // (e.g. UnferKernel) have no `run`: their functions are
                        // still defined for cross-module linking, but executing one
                        // with garbage arguments could dereference a non-pointer and
                        // crash. Returning null here makes the caller skip execution.
                        let (func_id, quiet_skip) = if name_str.is_empty() {
                            // Try "run" first (conventional), then "main"
                            // (entry-point from the Austral compiler), then
                            // fall back to the first function in the module.
                            let found = name_map.get("run")
                                .or_else(|| name_map.get("main"))
                                .or_else(|| name_map.values().next())
                                .copied();
                            match found {
                                Some(fid) => (Some(fid), false),
                                None => (None, true),
                            }
                        } else {
                            (name_map.get(name_str).copied(), false)
                        };

                        if let Some(fid) = func_id {
                            let ptr = jit.get_finalized_function(fid) as *const c_void;
                            cps_debug(&format!("SUCCESS compiled at {:?}", ptr));
                            ptr
                        } else if quiet_skip {
                            // No entry point to run in this (library) module.
                            std::ptr::null()
                        } else {
                            let avail: Vec<&String> = name_map.keys().collect();
                            let msg = format!(
                                "Function '{}' not found. Available: [{}]",
                                name_str,
                                avail.iter().map(|s| s.as_str())
                                     .collect::<Vec<_>>().join(", ")
                            );
                            set_last_error(&msg);
                            cps_debug(&msg);
                            std::ptr::null()
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("Compilation error: {}", e);
                set_last_error(&msg);
                cps_debug(&format!("compile_to_function_named failed: {}", e));
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
    CURRENT_MODULE.with(|m| *m.borrow_mut() = None);
}

/// Hot-swap: replace the entire JIT module with one compiled from the given
/// IR binary. All previous function pointers become invalid. Returns the
/// new `CpsModule` entry function pointer (or null if compilation fails),
/// and sets `CURRENT_MODULE` to the new module.
#[no_mangle]
pub extern "C" fn cranelift_swap_binary(
    ir_ptr: *const u8,
    ir_len: usize,
) -> *const c_void {
    cranelift_clear_error();
    if ir_ptr.is_null() || ir_len == 0 {
        set_last_error("Empty IR passed to swap");
        return std::ptr::null();
    }
    let ir_slice = unsafe { std::slice::from_raw_parts(ir_ptr, ir_len) };

    // Build a fresh JIT — same as cranelift_init() but forced.
    let mut new_jit = match (|| -> Result<JITModule, String> {
        let target_builder = cranelift_native::builder()
            .map_err(|e| format!("Native builder failed: {}", e))?;
        let flag_builder = cranelift_codegen::settings::builder();
        let isa = target_builder
            .finish(cranelift_codegen::settings::Flags::new(flag_builder))
            .map_err(|e| format!("ISA finish failed: {}", e))?;
        let mut builder =
            JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        builder.symbol("au_print_int", au_print_int as *const u8);
        builder.symbol("au_exit",      au_exit      as *const u8);
        builder.symbol("au_alloc",     au_alloc     as *const u8);
        builder.symbol("au_free",      au_free      as *const u8);
        builder.symbol("__union_new",   au_exit as *const u8);
        builder.symbol("__record_new",  au_exit as *const u8);
        builder.symbol("__slot_get",    au_exit as *const u8);
        #[cfg(feature = "unfer-kernel")]
        register_unfer_symbols(&mut builder);
        #[cfg(feature = "zenodo-store")]
        register_zenodo_symbols(&mut builder);
        Ok(JITModule::new(builder))
    })() {
        Ok(jit) => jit,
        Err(e) => {
            let msg = format!("Swap JIT init failed: {}", e);
            set_last_error(&msg);
            cps_debug(&msg);
            return std::ptr::null();
        }
    };

    // Compile the new binary into the fresh JIT
    let (module, entry_ptr) = match cps::compile_cps_to_clif(&mut new_jit, ir_slice) {
        Ok(module) => {
            match new_jit.finalize_definitions() {
                Err(e) => {
                    let msg = format!("Swap finalize failed: {}", e);
                    set_last_error(&msg);
                    cps_debug(&msg);
                    return std::ptr::null();
                }
                Ok(_) => {
                    let name_map = module.name_map.clone();
                    // Determine which function to return as entry
                    let entry_name = name_map.get("run")
                        .or_else(|| name_map.get("main"))
                        .or_else(|| name_map.values().next())
                        .copied();
                    let ptr = entry_name
                        .and_then(|fid| {
                            let p = new_jit.get_finalized_function(fid) as *const c_void;
                            Some(p)
                        })
                        .unwrap_or(std::ptr::null());
                    (module, ptr)
                }
            }
        }
        Err(e) => {
            let msg = format!("Swap compilation error: {}", e);
            set_last_error(&msg);
            cps_debug(&msg);
            return std::ptr::null();
        }
    };

    // Replace global state
    let name_map_len = module.name_map.len();
    JIT.with(|c| *c.borrow_mut() = Some(new_jit));
    CURRENT_MODULE.with(|m| *m.borrow_mut() = Some(module));

    cps_debug(&format!("Swap complete: {} functions, entry at {:?}",
        name_map_len, entry_ptr));
    entry_ptr
}

/// Look up a previously compiled function by name. Returns a function pointer
/// that can be called via `execute_function`, or null if not found.
/// This does NOT recompile — the function must have been compiled by an earlier
/// call to `compile_to_function_named` (or `compile_to_function`).
#[no_mangle]
pub extern "C" fn lookup_function(
    name_ptr: *const u8,
    name_len: usize,
) -> *const c_void {
    if name_ptr.is_null() || name_len == 0 {
        return std::ptr::null();
    }
    let slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
    let name = match std::str::from_utf8(slice) {
        Ok(s) => s.trim_end_matches('\0'),
        Err(_) => return std::ptr::null(),
    };

    JIT.with(|jit_cell| {
        CURRENT_MODULE.with(|mod_cell| {
            let jit = jit_cell.borrow();
            let jit = jit.as_ref()?;
            let module = mod_cell.borrow();
            let module = module.as_ref()?;
            let fid = module.name_map.get(name)?;
            let ptr = jit.get_finalized_function(*fid) as *const c_void;
            Some(ptr)
        }).unwrap_or(std::ptr::null())
    })
}

/// Return the names of all compiled functions as a JSON array string
/// (e.g. `["run","main"]`). The caller must free the returned string via
/// `cranelift_free_string`.
#[no_mangle]
pub extern "C" fn list_compiled_function_names() -> *mut std::ffi::c_char {
    CURRENT_MODULE.with(|mod_cell| {
        let module = mod_cell.borrow();
        let names: Vec<&String> = match module.as_ref() {
            Some(m) => m.name_map.keys().collect(),
            None => vec![],
        };
        let json = format!("[{}]",
            names.iter()
                .map(|n| format!("\"{}\"", n))
                .collect::<Vec<_>>()
                .join(",")
        );
        CString::new(json).unwrap_or_default().into_raw()
    })
}

/// Free a string previously returned by `list_compiled_function_names`.
#[no_mangle]
pub extern "C" fn cranelift_free_string(s: *mut std::ffi::c_char) {
    if !s.is_null() {
        unsafe { let _ = CString::from_raw(s); }
    }
}

/// Austral→deltanet UNF translation bridge for the OCaml compiler plugin.
///
/// Runs the Austral source fragment through the kernel's `uk_austral_unf`
/// symbol on a lazily-created model and returns the `AustralReport` JSON as
/// a malloc'd NUL-terminated string (freed by [`cranelift_free_string`]), or
/// NULL on failure (see `cranelift_last_error`). This is the "call the
/// kernel from the compiler" direction of the S36 cycle: the OCaml
/// `Deltanet_plugin` pass recomputes top-level constant expressions through
/// the kernel's unique-normal-form (interaction-net) reducer and rejects the
/// module when the values disagree.
#[cfg(feature = "unfer-kernel")]
#[no_mangle]
pub extern "C" fn austral_unf_translate(src: *const std::ffi::c_char) -> *mut std::ffi::c_char {
    use std::ffi::CStr;
    use std::sync::OnceLock;

    static MODEL: OnceLock<i64> = OnceLock::new();

    if src.is_null() {
        set_last_error("austral_unf_translate: null source");
        return std::ptr::null_mut();
    }
    let src_str = match unsafe { CStr::from_ptr(src) }.to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            set_last_error(&format!("austral_unf_translate: invalid UTF-8: {e}"));
            return std::ptr::null_mut();
        }
    };
    let model = *MODEL.get_or_init(|| {
        // A minimal QFM-free model spec so the handle-based kernel API has a
        // session to attach the report to (mirrors the FFI test spec).
        let spec = br#"{"hamiltonian":{"kind":"builtin","name":"harmonic_chain","params":{"n_modes":2,"omega":1.0}},"prior":{"kind":"vacuum"},"solver":{"krylov_dim":4,"prune_eps":1e-12,"max_components":50000,"restarts":1,"device":{"kind":"cpu"}}}"#;
        unfer_ffi::uk_model_create(spec.as_ptr() as *const u8, spec.len() as i64)
    });
    if model <= 0 {
        set_last_error("austral_unf_translate: kernel model creation failed");
        return std::ptr::null_mut();
    }
    let code = unfer_ffi::uk_austral_unf(model, src_str.as_ptr(), src_str.len() as i64);
    if code != 0 {
        set_last_error(&format!("austral_unf_translate: uk_austral_unf failed with {code}"));
        return std::ptr::null_mut();
    }
    // Probe-then-copy result retrieval.
    let needed = unfer_ffi::uk_get_result(model, std::ptr::null_mut(), 0);
    if needed <= 0 {
        set_last_error("austral_unf_translate: uk_get_result probe failed");
        return std::ptr::null_mut();
    }
    let mut buf = vec![0u8; needed as usize + 1];
    let written = unfer_ffi::uk_get_result(model, buf.as_mut_ptr(), buf.len() as i64);
    if written != needed {
        set_last_error("austral_unf_translate: uk_get_result copy mismatch");
        return std::ptr::null_mut();
    }
    buf.truncate(needed as usize);
    match CString::new(buf) {
        Ok(cs) => cs.into_raw(),
        Err(_) => {
            set_last_error("austral_unf_translate: result contains NUL");
            std::ptr::null_mut()
        }
    }
}

/// Install AllowAll authorizer (disables all authorization checks).
#[no_mangle]
pub extern "C" fn set_allow_all() {
    crate::auth::set_allow_all();
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
    unsafe {
        cell_set_jit_fn_ptr(desc_ptr, jit_ptr);
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

// The `au_*` runtime primitives are now defined in this crate (see the
// `#[no_mangle]` definitions above) so the bridge `.so` is self-contained.
// The `test-stubs` feature is retained as a no-op for compatibility with
// existing `--features test-stubs` invocations.

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
    /// is tested at the shell level via `make hotswap-test` in `safestos/`
    /// (builds `cells/counter_v1.so` + `cells/counter_v2.so`, runs
    /// `test/hotswap_e2e` which loads V1, steps, swaps to V2 with state
    /// migration, steps again, and asserts the counter was preserved).
    /// This Rust test verifies the Rust→C linkage and the guard path
    /// without the C scheduler.
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
