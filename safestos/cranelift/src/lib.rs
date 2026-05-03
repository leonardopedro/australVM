use cranelift_jit::{JITModule, JITBuilder};
use cranelift_module::Module;
use std::cell::RefCell;
use std::ffi::{c_void, CString};

pub mod cps;

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
                        let func_id = if name_str.is_empty() {
                            module.name_map.values().next().copied()
                        } else {
                            module.name_map.get(name_str).copied()
                        };

                        if let Some(fid) = func_id {
                            let ptr = jit.get_finalized_function(fid) as *const c_void;
                            eprintln!("CPS: SUCCESS compiled at {:?}", ptr);
                            ptr
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
