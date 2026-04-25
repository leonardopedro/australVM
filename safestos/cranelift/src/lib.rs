/// Cranelift Backend for SafestOS - v0.131
/// 
/// Provides JIT compilation with guaranteed tail calls
/// 
/// # Architecture
/// 
/// Uses Cranelift's `return_call` instruction for O(1) recursion.
/// Thread-local JITModule avoids Send/Sync issues.
/// 
/// # API
/// 
/// - `compile_to_function(ptr, len)` -> function_pointer
/// - `cranelift_init()` -> initialize
/// - `cranelift_is_ready()` -> check status
pub mod cps;

// Re-export for crate usage
pub use cranelift_jit::JITModule;

use cranelift_jit::JITBuilder;
use std::cell::RefCell;
use std::ffi::c_void;

thread_local! {
    static JIT: RefCell<Option<JITModule>> = RefCell::new(None);
}

extern "C" {
    fn scheduler_dispatch() -> !;
}

#[no_mangle]
pub extern "C" fn cranelift_init() -> i32 {
    JIT.with(|cell| {
        if cell.borrow().is_some() {
            return 0;
        }

        let resolver = |_libcall: cranelift_codegen::ir::LibCall| String::new();
        let mut builder = JITBuilder::new(Box::new(resolver)).unwrap();
        builder.symbol("scheduler_dispatch", scheduler_dispatch as *const u8);
        let jit = cranelift_jit::JITModule::new(builder);
        cell.replace(Some(jit));
        0
    })
}

#[no_mangle]
pub extern "C" fn compile_to_function(
    _ir_ptr: *const u8,
    _ir_len: usize,
) -> *const c_void {
    // Initialize JIT if needed
    if JIT.with(|cell| cell.borrow().is_none()) {
        if cranelift_init() != 0 {
            return std::ptr::null();
        }
    }

    JIT.with(|cell| {
        let mut opt = cell.borrow_mut();
        let jit = opt.as_mut().unwrap();

        // Demo mode: return simple function returning 42
        if _ir_ptr.is_null() || _ir_len == 0 {
            let result = cps::build_simple(jit);
            match result {
                Ok(cf) => {
                    if jit.finalize_definitions().is_err() {
                        return std::ptr::null();
                    }
                    return jit.get_finalized_function(cf.id) as *const c_void;
                }
                Err(_) => return std::ptr::null(),
            }
        }

        // Parse CPS IR and compile
        // _ir_ptr points to CPS format: [magic][func_count][functions...]
        let ir_slice = unsafe {
            std::slice::from_raw_parts(_ir_ptr, _ir_len)
        };
        
        match cps::compile_cps_to_clif(jit, ir_slice) {
            Ok(cf) => {
                if jit.finalize_definitions().is_err() {
                    return std::ptr::null();
                }
                jit.get_finalized_function(cf.id) as *const c_void
            }
            Err(_) => std::ptr::null(),
        }
    })
}

#[no_mangle]
pub extern "C" fn cranelift_version() -> u32 {
    0x0083000
}

#[no_mangle]
pub extern "C" fn cranelift_is_ready() -> i32 {
    JIT.with(|cell| if cell.borrow().is_some() { 1 } else { 0 })
}

#[no_mangle]
pub extern "C" fn cranelift_shutdown() {
    JIT.with(|cell| *cell.borrow_mut() = None);
}
