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

pub use cranelift_jit::JITModule;

use cranelift_jit::JITBuilder;
use std::cell::RefCell;
use std::ffi::c_void;

thread_local! {
    static JIT: RefCell<Option<JITModule>> = RefCell::new(None);
}

#[no_mangle]
pub extern "C" fn cranelift_init() -> i32 {
    JIT.with(|cell| {
        if cell.borrow().is_some() {
            return 0;
        }

        let resolver = |_libcall: cranelift_codegen::ir::LibCall| String::new();
        let mut builder = JITBuilder::new(Box::new(resolver)).unwrap();
        let jit = cranelift_jit::JITModule::new(builder);
        cell.replace(Some(jit));
        0
    })
}

/// Compile CPS IR binary to native code.
///
/// Returns a function pointer to the LAST function defined in the IR
/// (which is typically `main`).
///
/// The IR format is:
/// [magic: u32 = 0x43505331][functions: u32][functions...]
#[no_mangle]
pub extern "C" fn compile_to_function(
    ir_ptr: *const u8,
    ir_len: usize,
) -> *const c_void {
    // Initialize JIT if needed
    if JIT.with(|cell| cell.borrow().is_none()) {
        if cranelift_init() != 0 {
            return std::ptr::null();
        }
    }

    if ir_ptr.is_null() || ir_len == 0 {
        return std::ptr::null();
    }

    JIT.with(|cell| {
        let mut opt = cell.borrow_mut();
        let jit = opt.as_mut().unwrap();

        let ir_slice = unsafe { std::slice::from_raw_parts(ir_ptr, ir_len) };

        match cps::compile_cps_to_clif(jit, ir_slice) {
            Ok(module) => {
                if jit.finalize_definitions().is_err() {
                    eprintln!("CPS: jit.finalize_definitions() failed");
                    return std::ptr::null();
                }

                // Return the last function (usually main)
                if let Some(last) = module.functions.last() {
                    jit.get_finalized_function(last.id) as *const c_void
                } else {
                    std::ptr::null()
                }
            }
            Err(e) => {
                eprintln!("CPS compile error: {}", e);
                std::ptr::null()
            }
        }
    })
}

/// Compile CPS IR and return a specific function by name.
#[no_mangle]
pub extern "C" fn compile_to_function_named(
    ir_ptr: *const u8,
    ir_len: usize,
    name_ptr: *const u8,
    name_len: usize,
) -> *const c_void {
    if JIT.with(|cell| cell.borrow().is_none()) {
        if cranelift_init() != 0 {
            return std::ptr::null();
        }
    }

    if ir_ptr.is_null() || ir_len == 0 || name_ptr.is_null() {
        return std::ptr::null();
    }

    let ir_slice = unsafe { std::slice::from_raw_parts(ir_ptr, ir_len) };
    let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
    let func_name = match std::str::from_utf8(name_slice) {
        Ok(s) => s.to_string(),
        Err(_) => return std::ptr::null(),
    };

    JIT.with(|cell| {
        let mut opt = cell.borrow_mut();
        let jit = opt.as_mut().unwrap();

        match cps::compile_cps_to_clif(jit, ir_slice) {
            Ok(module) => {
                if jit.finalize_definitions().is_err() {
                    eprintln!("CPS: jit.finalize_definitions() failed");
                    return std::ptr::null();
                }

                if let Some(&func_id) = module.name_map.get(&func_name) {
                    jit.get_finalized_function(func_id) as *const c_void
                } else {
                    eprintln!("CPS: function '{}' not found", func_name);
                    std::ptr::null()
                }
            }
            Err(e) => {
                eprintln!("CPS compile error: {}", e);
                std::ptr::null()
            }
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
