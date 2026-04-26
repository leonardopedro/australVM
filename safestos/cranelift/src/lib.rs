/// Minimal Cranelift-CPS bridge compatible with HEAD version of cps.rs
use cranelift::prelude::*;
use cranelift_jit::{JITModule, JITBuilder};
use cranelift_module::{FuncId, Linkage, Module};
use std::cell::RefCell;
use std::ffi::c_void;
use std::collections::HashMap;

pub mod cps;

thread_local! {
    static JIT: RefCell<Option<JITModule>> = RefCell::new(None);
}

#[no_mangle]
pub extern "C" fn cranelift_init() -> i32 {
    JIT.with(|cell| {
        if cell.borrow().is_some() {
            return 0;
        }
        
        // No external symbols
        let resolver = |_libcall: cranelift_codegen::ir::LibCall| String::new();
        match JITBuilder::new(Box::new(resolver)) {
            Ok(builder) => {
                let jit = JITModule::new(builder);
                cell.replace(Some(jit));
                0
            }
            Err(_) => 1,
        }
    })
}

/// Compile first function from CPS binary
#[no_mangle]
pub extern "C" fn compile_to_function(
    ir_ptr: *const u8,
    ir_len: usize,
) -> *const c_void {
    if JIT.with(|cell| cell.borrow().is_none()) {
        if cranelift_init() != 0 {
            return std::ptr::null();
        }
    }

    if ir_ptr.is_null() || ir_len == 0 {
        return std::ptr::null();
    }

    let ir_slice = unsafe { std::slice::from_raw_parts(ir_ptr, ir_len) };

    JIT.with(|cell| {
        let mut opt = cell.borrow_mut();
        let jit = opt.as_mut().unwrap();

        // Compile all functions in the IR
        match cps::compile_cps_to_clif(jit, ir_slice) {
            Ok(module) => {
                if jit.finalize_definitions().is_err() {
                    eprintln!("CPS: jit.finalize_definitions() failed");
                    return std::ptr::null();
                }

                // Get first function (by order in Vec)
                if let Some(first) = module.functions.first() {
                    let ptr = jit.get_finalized_function(first.id) as *const c_void;
                    eprintln!("CPS: Compiled first function '{}' at {:?}", first.name, ptr);
                    ptr
                } else {
                    eprintln!("CPS: No functions in module");
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

/// Compile named function from CPS binary
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
    let name_str = match std::str::from_utf8(name_slice) {
        Ok(s) => s.trim_end_matches('\0'),
        Err(_) => return std::ptr::null(),
    };

    JIT.with(|cell| {
        let mut opt = cell.borrow_mut();
        let jit = opt.as_mut().unwrap();

        // Compile all functions
        match cps::compile_cps_to_clif(jit, ir_slice) {
            Ok(module) => {
                if jit.finalize_definitions().is_err() {
                    eprintln!("CPS: jit.finalize_definitions() failed");
                    return std::ptr::null();
                }

                // Look up function by name
                if let Some(&func_id) = module.name_map.get(name_str) {
                    let ptr = jit.get_finalized_function(func_id) as *const c_void;
                    eprintln!("CPS: Compiled function '{}' at {:?}", name_str, ptr);
                    ptr
                } else {
                    eprintln!("CPS: Function '{}' not found in name_map: {:?}", name_str, module.name_map.keys());
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

#[no_mangle] pub extern "C" fn cranelift_version() -> u32 { 0x0083000 }
#[no_mangle] pub extern "C" fn cranelift_is_ready() -> i32 { JIT.with(|c| if c.borrow().is_some() {1} else {0}) }
#[no_mangle] pub extern "C" fn cranelift_shutdown() { JIT.with(|c| *c.borrow_mut() = None); }
