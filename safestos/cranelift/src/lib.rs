/// Cranelift Backend for SafestOS - v0.131
pub mod cps;

use cranelift::prelude::*;
use cranelift_codegen::isa::CallConv;
use cranelift_jit::JITBuilder;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use std::cell::RefCell;
use std::ffi::c_void;

thread_local! {
    static JIT: RefCell<Option<cranelift_jit::JITModule>> = RefCell::new(None);
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
    if JIT.with(|cell| cell.borrow().is_none()) {
        if cranelift_init() != 0 {
            return std::ptr::null();
        }
    }

    JIT.with(|cell| {
        let mut opt = cell.borrow_mut();
        let jit = opt.as_mut().unwrap();

        // For now, compile simple function returning 42
        // This proves the bridge works
        if _ir_ptr.is_null() || _ir_len == 0 {
            // Demo mode
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

        // TODO: Real IR parsing
        std::ptr::null()
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
