use cranelift_jit::{JITModule, JITBuilder};
use cranelift_module::Module;
use std::cell::RefCell;
use std::ffi::c_void;

pub mod cps;

thread_local! {
    static JIT: RefCell<Option<JITModule>> = RefCell::new(None);
}

#[no_mangle]
pub extern "C" fn cranelift_init() -> i32 {
    JIT.with(|cell| {
        if cell.borrow().is_some() { return 0; }
        let resolver = |_libcall| String::new();
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

#[no_mangle]
pub extern "C" fn compile_to_function_named(
    ir_ptr: *const u8,
    ir_len: usize,
    name_ptr: *const u8,
    name_len: usize,
) -> *const c_void {
    if JIT.with(|c| c.borrow().is_none()) {
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

        // Compile entire module
        match cps::compile_cps_to_clif(jit, ir_slice) {
            Ok(module) => {
                if jit.finalize_definitions().is_err() {
                    eprintln!("CPS: finalize failed");
                    return std::ptr::null();
                }
                
                // Lookup requested function
                if let Some(&func_id) = module.name_map.get(name_str) {
                    let ptr = jit.get_finalized_function(func_id) as *const c_void;
                    eprintln!("CPS: SUCCESS '{}' compiled at {:?}", name_str, ptr);
                    ptr
                } else {
                    eprintln!("CPS: Function '{}' not found in map (available: {:?})", 
                             name_str, module.name_map.keys());
                    std::ptr::null()
                }
            }
            Err(e) => {
                eprintln!("CPS: compile_to_function_named failed: {}", e);
                std::ptr::null()
            }
        }
    })
}

/// Compile first function (alternative API)
#[no_mangle]
pub extern "C" fn compile_to_function(
    ir_ptr: *const u8,
    ir_len: usize,
) -> *const c_void {
    compile_to_function_named(ir_ptr, ir_len, std::ptr::null(), 0)
}

#[no_mangle] pub extern "C" fn cranelift_version() -> u32 { 0x0083000 }
#[no_mangle] pub extern "C" fn cranelift_is_ready() -> i32 { JIT.with(|c| if c.borrow().is_some() {1} else {0}) }
#[no_mangle] pub extern "C" fn cranelift_shutdown() { JIT.with(|c| *c.borrow_mut() = None); }
