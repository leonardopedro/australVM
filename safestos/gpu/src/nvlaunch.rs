//! G21a: launch the published NVIDIA artifact on real hardware.
//!
//! The PTX `austral compile --target-type=mlir` publishes is a *virtual* ISA
//! artifact; this module takes it to an actual device. Two halves:
//!
//! * [`preflight`] is pure and always compiled. It reads the facts a PTX module
//!   actually carries (`.target`, `.address_size`, the `.entry` name and its
//!   parameter count, whether every pointer role is converted with
//!   `cvta.to.global`) and the plan the launch will use, and refuses a mismatch
//!   **before** the driver is called: wrong extent, wrong arity, wrong
//!   parameter count, missing address conversion, or a target newer than the
//!   device. A refusal here is a finding about the artifact, not a crash.
//! * [`driver`] (feature `nvidia`) loads `libcuda.so.1` with `dlopen` and drives
//!   the CUDA Driver API directly: no toolkit, no `nvcc`, no linked CUDA, so the
//!   default CPU-only build is unchanged. Every call's `CUresult` is checked
//!   and named; a failed module load never reaches `cuModuleGetFunction`.
//!
//! What the emitted kernel is: one thread per element, indexed by the global
//! id (`block_id * block_dim + thread_id`, since G21a; it used to be the
//! block-local `tid.x`) and guarded by `gid > extent - 1`. The plan launches
//! the block the module declared (`gpu.known_block_size`, recorded in the
//! bundle as `workgroup`) and `grid = ceil(extent / block)` blocks, refusing a
//! grid over a PTX that never reads `%ctaid` — a multi-block launch of an
//! artifact written for one block would silently compute the first block's
//! elements over and over. A `--block` override is checked, never inferred.

/// The published ABI names its kernel `main` (the MLIR entry is renamed by the
/// NVVM lowering, and the bundle records that).
pub const ENTRY: &str = "main";

/// Largest block a device accepts (CUDA's documented limit).
pub const MAX_BLOCK: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub entry: String,
    pub params: usize,
    pub target_chip: String,
    pub address_size: u32,
    pub converts_pointers: bool,
    /// The module derives its element index from the block id, i.e. it is safe
    /// to launch with more than one block.
    pub uses_block_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    EntryMissing {
        expected: String,
    },
    AddressSize {
        got: u32,
    },
    Arity {
        expected: usize,
        got: usize,
    },
    ParamCount {
        recorded: usize,
        ptx: usize,
    },
    ExtentMismatch {
        recorded: usize,
        requested: usize,
    },
    ExtentZero,
    BlockZero,
    BlockTooLarge {
        block: usize,
    },
    GridTooLarge {
        extent: usize,
        block: usize,
    },
    /// More than one block, but the PTX never reads `%ctaid`, so every block
    /// would compute the same block-local indices.
    NeedsBlockIndex {
        extent: usize,
        block: usize,
    },
    NoAddressConversion,
    TargetNewerThanDevice {
        chip: String,
        device: (i32, i32),
    },
    TargetUnreadable {
        chip: String,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::EntryMissing { expected } => write!(
                f,
                "the PTX declares no `.entry {expected}`; the artifact's ABI names that kernel"
            ),
            Refusal::AddressSize { got } => {
                write!(f, "the PTX address size is {got}, not 64")
            }
            Refusal::Arity { expected, got } => write!(
                f,
                "the launch supplies {got} operand(s); the bundle records {expected}"
            ),
            Refusal::ParamCount { recorded, ptx } => write!(
                f,
                "the PTX entry takes {ptx} parameter(s); the bundle records {recorded}"
            ),
            Refusal::ExtentMismatch { recorded, requested } => write!(
                f,
                "the launch extent is {requested}; the bundle records {recorded}. The emitted \
                 kernel guards `gid > extent - 1`, so a smaller extent would leave the guard \
                 open past the buffer and a larger one would skip elements"
            ),
            Refusal::ExtentZero => write!(f, "the extent must be positive"),
            Refusal::BlockZero => write!(f, "the block must be positive"),
            Refusal::BlockTooLarge { block } => write!(
                f,
                "the block is {block}; no device accepts a block above {MAX_BLOCK}"
            ),
            Refusal::GridTooLarge { extent, block } => write!(
                f,
                "extent {extent} at block {block} needs more blocks than the grid \
                 dimension holds"
            ),
            Refusal::NeedsBlockIndex { extent, block } => write!(
                f,
                "extent {extent} at block {block} needs a grid, but the PTX never reads \
                 `%ctaid`: every block would compute the same element indices"
            ),
            Refusal::NoAddressConversion => write!(
                f,
                "no `cvta.to.global` in the PTX: the parameters are not converted to global \
                 pointers, so the recorded memory model does not apply"
            ),
            Refusal::TargetNewerThanDevice { chip, device } => write!(
                f,
                "the PTX targets {chip} but the device is compute capability {}.{}; the driver \
                 will not load it",
                device.0, device.1
            ),
            Refusal::TargetUnreadable { chip } => {
                write!(f, "cannot read compute capability from `.target {chip}`")
            }
        }
    }
}

/// Read the facts a PTX module carries. Text parsing, no execution.
pub fn ptx_facts(text: &str) -> Result<Facts, String> {
    let mut target_chip = None;
    let mut address_size = None;
    let mut entry = None;
    let mut params = 0usize;
    let mut in_entry = false;
    let mut converts_pointers = false;
    let mut uses_block_id = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(".target ") {
            target_chip = Some(rest.split_whitespace().next().unwrap_or("").to_string());
        } else if let Some(rest) = t.strip_prefix(".address_size ") {
            address_size = rest.trim().parse::<u32>().ok();
        } else if let Some(rest) = t.strip_prefix(".visible .entry ").or_else(|| t.strip_prefix(".entry ")) {
            let name = rest.split(|c: char| c == '(' || c.is_whitespace()).next().unwrap_or("");
            entry = Some(name.to_string());
            in_entry = rest.contains('(') && !rest.contains(')');
        } else if in_entry {
            if t.starts_with(".param") {
                params += 1;
            } else if t.starts_with(')') {
                in_entry = false;
            }
        }
        if t.starts_with("cvta.to.global") {
            converts_pointers = true;
        }
        if t.contains("%ctaid") {
            uses_block_id = true;
        }
    }
    Ok(Facts {
        entry: entry.ok_or_else(|| "the PTX declares no .entry".to_string())?,
        params,
        target_chip: target_chip.ok_or_else(|| "the PTX declares no .target".to_string())?,
        address_size: address_size.ok_or_else(|| "the PTX declares no .address_size".to_string())?,
        converts_pointers,
        uses_block_id,
    })
}

/// Parse `sm_XY` into `(X, Y)`. `sm_90a` style suffixes are accepted by
/// truncating at the first non-digit after the major number.
pub fn capability_of(chip: &str) -> Option<(i32, i32)> {
    let digits = chip.strip_prefix("sm_")?;
    // `sm_61` is major 6, minor 1; `sm_90a` is (9, 0) — the suffix is an
    // architecture variant, not part of the minor number.
    let mut chars = digits.chars();
    let major: String = chars.by_ref().take_while(char::is_ascii_digit).take(1).collect();
    let minor: String = chars.take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// The launch a preflight-checked artifact implies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub extent: usize,
    pub block: usize,
    pub grid: usize,
    /// One 5-word memref descriptor per buffer: alloc ptr, aligned ptr, offset,
    /// element count (bytes), stride — the layout the bundle records.
    pub descriptors: usize,
    pub words_per_descriptor: usize,
}

impl Plan {
    pub fn params(&self) -> usize {
        self.descriptors * self.words_per_descriptor
    }
}

/// Check the artifact and the requested launch against each other, and the
/// target against the device. Pure: every refusal is a statement about the
/// inputs, and none of them needs a driver to evaluate.
/// The largest grid the x dimension holds (CUDA's `maxGridSize[0]`).
pub const MAX_GRID: usize = 2_147_483_647;

pub fn plan(
    facts: &Facts,
    recorded_params: usize,
    recorded_arity: usize,
    recorded_extent: usize,
    requested_extent: usize,
    requested_arity: usize,
    recorded_block: usize,
    requested_block: Option<usize>,
    device: Option<(i32, i32)>,
) -> Result<Plan, Refusal> {
    if facts.entry != ENTRY {
        return Err(Refusal::EntryMissing {
            expected: ENTRY.to_string(),
        });
    }
    if facts.address_size != 64 {
        return Err(Refusal::AddressSize {
            got: facts.address_size,
        });
    }
    if requested_arity != recorded_arity {
        return Err(Refusal::Arity {
            expected: recorded_arity,
            got: requested_arity,
        });
    }
    if facts.params != recorded_params {
        return Err(Refusal::ParamCount {
            recorded: recorded_params,
            ptx: facts.params,
        });
    }
    if requested_extent == 0 {
        return Err(Refusal::ExtentZero);
    }
    if requested_extent != recorded_extent {
        return Err(Refusal::ExtentMismatch {
            recorded: recorded_extent,
            requested: requested_extent,
        });
    }
    if !facts.converts_pointers {
        return Err(Refusal::NoAddressConversion);
    }
    if let Some((major, minor)) = device {
        let chip = capability_of(&facts.target_chip)
            .ok_or_else(|| Refusal::TargetUnreadable {
                chip: facts.target_chip.clone(),
            })?;
        if chip > (major, minor) {
            return Err(Refusal::TargetNewerThanDevice {
                chip: facts.target_chip.clone(),
                device: (major, minor),
            });
        }
    }
    // The block is the one the module declared (its `gpu.known_block_size`),
    // or a smaller override. A larger block than the module declared is only
    // accepted when it is explicitly requested, because the emitter's guard is
    // the global index and the declaration is a promise about the workgroup.
    let block = requested_block.unwrap_or(recorded_block.min(requested_extent));
    if block == 0 {
        return Err(Refusal::BlockZero);
    }
    if block > MAX_BLOCK {
        return Err(Refusal::BlockTooLarge { block });
    }
    let grid = requested_extent.div_ceil(block);
    if grid > MAX_GRID {
        return Err(Refusal::GridTooLarge {
            extent: requested_extent,
            block,
        });
    }
    if grid > 1 && !facts.uses_block_id {
        return Err(Refusal::NeedsBlockIndex {
            extent: requested_extent,
            block,
        });
    }
    let descriptors = (recorded_params / 5).max(1);
    Ok(Plan {
        extent: requested_extent,
        block,
        grid,
        descriptors,
        words_per_descriptor: 5,
    })
}

#[cfg(feature = "nvidia")]
pub mod driver {
    //! CUDA Driver API through `dlopen`, with every `CUresult` checked.
    //!
    //! Symbols are resolved on demand: an absent function is a named error, not
    //! a panic, and a failed load never reaches the next call.

    use super::Plan;
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::ptr;

    type CuResult = c_int;
    type CuDevice = c_int;
    type CuContext = *mut c_void;
    type CuModule = *mut c_void;
    type CuFunction = *mut c_void;
    type CuDevicePtr = u64;
    type CuLinkState = *mut c_void;

    const CUDA_SUCCESS: CuResult = 0;
    /// `CU_JIT_INPUT_PTX`, the `cuLinkAddData` kind for PTX text.
    const CU_JIT_INPUT_PTX: c_int = 1;

    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *mut c_char;
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Device {
        pub name: String,
        pub capability: (i32, i32),
        pub driver_version: i32,
    }

    pub struct Driver {
        handle: *mut c_void,
        init: unsafe extern "C" fn(u32) -> CuResult,
        device_count: unsafe extern "C" fn(*mut c_int) -> CuResult,
        device_get: unsafe extern "C" fn(*mut CuDevice, c_int) -> CuResult,
        device_name: unsafe extern "C" fn(*mut c_char, c_int, CuDevice) -> CuResult,
        device_attr: unsafe extern "C" fn(*mut c_int, c_int, CuDevice) -> CuResult,
        driver_version: unsafe extern "C" fn(*mut c_int) -> CuResult,
        ctx_create: unsafe extern "C" fn(*mut CuContext, u32, CuDevice) -> CuResult,
        ctx_destroy: unsafe extern "C" fn(CuContext) -> CuResult,
        module_load: unsafe extern "C" fn(*mut CuModule, *const c_void) -> CuResult,
        module_unload: unsafe extern "C" fn(CuModule) -> CuResult,
        function_get: unsafe extern "C" fn(*mut CuFunction, CuModule, *const c_char) -> CuResult,
        mem_alloc: unsafe extern "C" fn(*mut CuDevicePtr, usize) -> CuResult,
        mem_free: unsafe extern "C" fn(CuDevicePtr) -> CuResult,
        memcpy_htod: unsafe extern "C" fn(CuDevicePtr, *const c_void, usize) -> CuResult,
        memcpy_dtoh: unsafe extern "C" fn(*mut c_void, CuDevicePtr, usize) -> CuResult,
        launch: unsafe extern "C" fn(
            CuFunction,
            u32,
            u32,
            u32,
            u32,
            u32,
            u32,
            u32,
            *mut c_void,
            *mut *mut c_void,
            *mut c_void,
        ) -> CuResult,
        sync: unsafe extern "C" fn() -> CuResult,
        error_name: unsafe extern "C" fn(CuResult, *mut *const c_char) -> CuResult,
        link_create: unsafe extern "C" fn(c_int, *mut c_int, *mut *mut c_void, *mut CuLinkState) -> CuResult,
        link_add: unsafe extern "C" fn(
            CuLinkState,
            c_int,
            *const c_void,
            usize,
            *const c_char,
            c_int,
            *mut c_int,
            *mut *mut c_void,
        ) -> CuResult,
        link_complete: unsafe extern "C" fn(CuLinkState, *mut *mut c_void, *mut usize) -> CuResult,
        link_destroy: unsafe extern "C" fn(CuLinkState) -> CuResult,
        context: CuContext,
        pub device: Device,
    }

    impl Driver {
        /// Open the driver (default `libcuda.so.1`, or `AUSTRAL_LIBCUDA`), take
        /// device 0 and create a context. An absent library or device is an
        /// `Err`, which the callers report as an unavailable probe.
        pub fn open(device_index: i32) -> Result<Driver, String> {
            let path = std::env::var("AUSTRAL_LIBCUDA").unwrap_or_else(|_| "libcuda.so.1".to_string());
            let cpath = CString::new(path.clone()).map_err(|_| "AUSTRAL_LIBCUDA contains NUL".to_string())?;
            let handle = unsafe { dlopen(cpath.as_ptr(), 2 /* RTLD_NOW */) };
            if handle.is_null() {
                let err = unsafe {
                    let e = dlerror();
                    if e.is_null() { String::new() } else { CStr::from_ptr(e).to_string_lossy().into_owned() }
                };
                return Err(format!("cannot load {path}: {err}"));
            }
            macro_rules! sym {
                ($name:literal, $ty:ty) => {{
                    let c = CString::new($name).unwrap();
                    let p = unsafe { dlsym(handle, c.as_ptr()) };
                    if p.is_null() {
                        return Err(format!("{path} exports no {}", $name));
                    }
                    unsafe { std::mem::transmute::<*mut c_void, $ty>(p) }
                }};
            }
            let init: unsafe extern "C" fn(u32) -> CuResult = sym!("cuInit", _);
            let device_count: unsafe extern "C" fn(*mut c_int) -> CuResult = sym!("cuDeviceGetCount", _);
            let device_get: unsafe extern "C" fn(*mut CuDevice, c_int) -> CuResult = sym!("cuDeviceGet", _);
            let device_name: unsafe extern "C" fn(*mut c_char, c_int, CuDevice) -> CuResult =
                sym!("cuDeviceGetName", _);
            let device_attr: unsafe extern "C" fn(*mut c_int, c_int, CuDevice) -> CuResult =
                sym!("cuDeviceGetAttribute", _);
            let driver_version: unsafe extern "C" fn(*mut c_int) -> CuResult = sym!("cuDriverGetVersion", _);
            let ctx_create: unsafe extern "C" fn(*mut CuContext, u32, CuDevice) -> CuResult =
                sym!("cuCtxCreate_v2", _);
            let ctx_destroy: unsafe extern "C" fn(CuContext) -> CuResult = sym!("cuCtxDestroy_v2", _);
            let module_load: unsafe extern "C" fn(*mut CuModule, *const c_void) -> CuResult =
                sym!("cuModuleLoadData", _);
            let module_unload: unsafe extern "C" fn(CuModule) -> CuResult = sym!("cuModuleUnload", _);
            let function_get: unsafe extern "C" fn(*mut CuFunction, CuModule, *const c_char) -> CuResult =
                sym!("cuModuleGetFunction", _);
            let mem_alloc: unsafe extern "C" fn(*mut CuDevicePtr, usize) -> CuResult = sym!("cuMemAlloc_v2", _);
            let mem_free: unsafe extern "C" fn(CuDevicePtr) -> CuResult = sym!("cuMemFree_v2", _);
            let memcpy_htod: unsafe extern "C" fn(CuDevicePtr, *const c_void, usize) -> CuResult =
                sym!("cuMemcpyHtoD_v2", _);
            let memcpy_dtoh: unsafe extern "C" fn(*mut c_void, CuDevicePtr, usize) -> CuResult =
                sym!("cuMemcpyDtoH_v2", _);
            let launch: unsafe extern "C" fn(
                CuFunction, u32, u32, u32, u32, u32, u32, u32, *mut c_void, *mut *mut c_void, *mut c_void,
            ) -> CuResult = sym!("cuLaunchKernel", _);
            let sync: unsafe extern "C" fn() -> CuResult = sym!("cuCtxSynchronize", _);
            let error_name: unsafe extern "C" fn(CuResult, *mut *const c_char) -> CuResult =
                sym!("cuGetErrorName", _);
            let link_create: unsafe extern "C" fn(
                c_int,
                *mut c_int,
                *mut *mut c_void,
                *mut CuLinkState,
            ) -> CuResult = sym!("cuLinkCreate_v2", _);
            let link_add: unsafe extern "C" fn(
                CuLinkState,
                c_int,
                *const c_void,
                usize,
                *const c_char,
                c_int,
                *mut c_int,
                *mut *mut c_void,
            ) -> CuResult = sym!("cuLinkAddData_v2", _);
            // `cuLinkComplete`/`cuLinkDestroy` are unversioned in the driver
            // library (only create/add carry `_v2`). Try the versioned name
            // first so a driver that adds one is still resolved.
            macro_rules! sym_any {
                ($name:literal, $fallback:literal, $ty:ty) => {{
                    let mut found: *mut c_void = std::ptr::null_mut();
                    for name in [$name, $fallback] {
                        let c = CString::new(name).unwrap();
                        found = unsafe { dlsym(handle, c.as_ptr()) };
                        if !found.is_null() {
                            break;
                        }
                    }
                    if found.is_null() {
                        return Err(format!("{path} exports neither {} nor {}", $name, $fallback));
                    }
                    unsafe { std::mem::transmute::<*mut c_void, $ty>(found) }
                }};
            }
            let link_complete: unsafe extern "C" fn(CuLinkState, *mut *mut c_void, *mut usize) -> CuResult =
                sym_any!("cuLinkComplete_v2", "cuLinkComplete", _);
            let link_destroy: unsafe extern "C" fn(CuLinkState) -> CuResult =
                sym_any!("cuLinkDestroy_v2", "cuLinkDestroy", _);

            let mut driver = Driver {
                handle,
                init,
                device_count,
                device_get,
                device_name,
                device_attr,
                driver_version,
                ctx_create,
                ctx_destroy,
                module_load,
                module_unload,
                function_get,
                mem_alloc,
                mem_free,
                memcpy_htod,
                memcpy_dtoh,
                launch,
                sync,
                error_name,
                link_create,
                link_add,
                link_complete,
                link_destroy,
                context: ptr::null_mut(),
                device: Device {
                    name: String::new(),
                    capability: (0, 0),
                    driver_version: 0,
                },
            };
            unsafe { driver.init_driver(device_index)? };
            Ok(driver)
        }

        unsafe fn init_driver(&mut self, index: i32) -> Result<(), String> {
            self.check((self.init)(0), "cuInit")?;
            let mut count: c_int = 0;
            self.check((self.device_count)(&mut count), "cuDeviceGetCount")?;
            if count <= index || index < 0 {
                return Err(format!("no CUDA device at index {index} ({count} present)"));
            }
            let mut dev: CuDevice = 0;
            self.check((self.device_get)(&mut dev, index), "cuDeviceGet")?;
            let mut name = [0 as c_char; 256];
            self.check((self.device_name)(name.as_mut_ptr(), name.len() as c_int, dev), "cuDeviceGetName")?;
            let name = CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();
            let mut major = 0;
            let mut minor = 0;
            // 75/76 are CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_{MAJOR,MINOR}.
            self.check((self.device_attr)(&mut major, 75, dev), "cuDeviceGetAttribute(major)")?;
            self.check((self.device_attr)(&mut minor, 76, dev), "cuDeviceGetAttribute(minor)")?;
            let mut version = 0;
            self.check((self.driver_version)(&mut version), "cuDriverGetVersion")?;
            let mut ctx: CuContext = ptr::null_mut();
            self.check((self.ctx_create)(&mut ctx, 0, dev), "cuCtxCreate")?;
            self.context = ctx;
            self.device = Device {
                name,
                capability: (major, minor),
                driver_version: version,
            };
            Ok(())
        }

        fn error_text(&self, code: CuResult) -> String {
            let mut p: *const c_char = ptr::null();
            let named = unsafe { (self.error_name)(code, &mut p) };
            if named != CUDA_SUCCESS || p.is_null() {
                return format!("CUresult {code}");
            }
            unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
        }

        fn check(&self, code: CuResult, what: &str) -> Result<(), String> {
            if code == CUDA_SUCCESS {
                Ok(())
            } else {
                Err(format!("{what} failed: {} ({code})", self.error_text(code)))
            }
        }

        /// Load the module and resolve `main`, without launching: the driver's
        /// own verdict on whether this PTX is acceptable for this device.
        pub fn load_only(&self, ptx: &str) -> Result<(), String> {
            let ptx_c = CString::new(ptx).map_err(|_| "the PTX text contains NUL".to_string())?;
            unsafe {
                let mut module: CuModule = ptr::null_mut();
                self.check((self.module_load)(&mut module, ptx_c.as_ptr() as *const c_void), "cuModuleLoadData")?;
                let entry = CString::new(super::ENTRY).unwrap();
                let mut function: CuFunction = ptr::null_mut();
                let got = (self.function_get)(&mut function, module, entry.as_ptr());
                let _ = (self.module_unload)(module);
                self.check(got, "cuModuleGetFunction")
            }
        }

        /// Link the PTX text into a native executable for this device and
        /// return the container bytes. This is the capture PC10 asks for: the
        /// driver's own translator produced these bytes for this device, and
        /// [`crate::target::identify`] checks that what came back is a CUDA
        /// ELF with executable text in it.
        pub fn link(&self, ptx: &str) -> Result<Vec<u8>, String> {
            let ptx_c = CString::new(ptx).map_err(|_| "the PTX text contains NUL".to_string())?;
            let name = CString::new("austral.ptx").unwrap();
            unsafe {
                let mut state: CuLinkState = ptr::null_mut();
                self.check((self.link_create)(0, ptr::null_mut(), ptr::null_mut(), &mut state), "cuLinkCreate")?;
                let added = (self.link_add)(
                    state,
                    CU_JIT_INPUT_PTX,
                    ptx_c.as_ptr() as *const c_void,
                    ptx.len() + 1,
                    name.as_ptr(),
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                if let Err(e) = self.check(added, "cuLinkAddData") {
                    let _ = (self.link_destroy)(state);
                    return Err(e);
                }
                let mut image: *mut c_void = ptr::null_mut();
                let mut size = 0usize;
                let done = (self.link_complete)(state, &mut image, &mut size);
                if let Err(e) = self.check(done, "cuLinkComplete") {
                    let _ = (self.link_destroy)(state);
                    return Err(e);
                }
                if image.is_null() || size == 0 {
                    let _ = (self.link_destroy)(state);
                    return Err("cuLinkComplete returned an empty image".to_string());
                }
                let bytes = std::slice::from_raw_parts(image as *const u8, size).to_vec();
                let _ = (self.link_destroy)(state);
                Ok(bytes)
            }
        }

        /// Load the module from PTX text (the driver JITs it for the device).
        pub fn launch(
            &self,
            ptx: &str,
            plan: &Plan,
            buffers: &[Vec<u32>],
        ) -> Result<Vec<Vec<u32>>, String> {
            if buffers.len() != plan.descriptors {
                return Err(format!(
                    "the plan describes {} buffer(s), the launch supplies {}",
                    plan.descriptors,
                    buffers.len()
                ));
            }
            let ptx_c = CString::new(ptx).map_err(|_| "the PTX text contains NUL".to_string())?;
            unsafe {
                let mut module: CuModule = ptr::null_mut();
                self.check((self.module_load)(&mut module, ptx_c.as_ptr() as *const c_void), "cuModuleLoadData")?;
                let entry = CString::new(super::ENTRY).unwrap();
                let mut function: CuFunction = ptr::null_mut();
                let got = (self.function_get)(&mut function, module, entry.as_ptr());
                if let Err(e) = self.check(got, "cuModuleGetFunction") {
                    let _ = (self.module_unload)(module);
                    return Err(e);
                }

                let mut device_ptrs = Vec::with_capacity(buffers.len());
                for words in buffers {
                    let mut p: CuDevicePtr = 0;
                    if let Err(e) = self.check((self.mem_alloc)(&mut p, words.len() * 4), "cuMemAlloc") {
                        for q in &device_ptrs {
                            let _ = (self.mem_free)(*q);
                        }
                        let _ = (self.module_unload)(module);
                        return Err(e);
                    }
                    device_ptrs.push(p);
                }
                for (words, p) in buffers.iter().zip(&device_ptrs) {
                    if let Err(e) = self.check(
                        (self.memcpy_htod)(*p, words.as_ptr() as *const c_void, words.len() * 4),
                        "cuMemcpyHtoD",
                    ) {
                        self.release(&device_ptrs, module);
                        return Err(e);
                    }
                }

                // One 5-word descriptor per buffer: alloc ptr, aligned ptr,
                // offset, byte length, stride.
                let mut params: Vec<u64> = Vec::with_capacity(plan.params());
                for p in &device_ptrs {
                    let bytes = plan.extent as u64 * 4;
                    params.extend_from_slice(&[*p, *p, 0, bytes, 1]);
                }
                let mut args: Vec<*mut c_void> = params
                    .iter_mut()
                    .map(|p| p as *mut u64 as *mut c_void)
                    .collect();
                let code = (self.launch)(
                    function,
                    plan.grid as u32,
                    1,
                    1,
                    plan.block as u32,
                    1,
                    1,
                    0,
                    ptr::null_mut(),
                    args.as_mut_ptr(),
                    ptr::null_mut(),
                );
                if let Err(e) = self.check(code, "cuLaunchKernel") {
                    self.release(&device_ptrs, module);
                    return Err(e);
                }
                if let Err(e) = self.check((self.sync)(), "cuCtxSynchronize") {
                    self.release(&device_ptrs, module);
                    return Err(e);
                }

                let mut out = Vec::with_capacity(buffers.len());
                for (words, p) in buffers.iter().zip(&device_ptrs) {
                    let mut host = vec![0u32; words.len()];
                    if let Err(e) = self.check(
                        (self.memcpy_dtoh)(host.as_mut_ptr() as *mut c_void, *p, host.len() * 4),
                        "cuMemcpyDtoH",
                    ) {
                        self.release(&device_ptrs, module);
                        return Err(e);
                    }
                    out.push(host);
                }
                self.release(&device_ptrs, module);
                Ok(out)
            }
        }

        unsafe fn release(&self, ptrs: &[CuDevicePtr], module: CuModule) {
            for p in ptrs {
                let _ = (self.mem_free)(*p);
            }
            let _ = (self.module_unload)(module);
        }
    }

    impl Drop for Driver {
        fn drop(&mut self) {
            unsafe {
                if !self.context.is_null() {
                    let _ = (self.ctx_destroy)(self.context);
                }
                let _ = dlopen; // the handle lives for the process
                let _ = self.handle;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clamp_ptx() -> String {
        // The shape the compiler publishes: `.visible .entry main`, 10 x .u64,
        // one cvta per pointer role, 64-bit addresses.
        let mut s = String::from(
            ".version 6.0\n.target sm_70\n.address_size 64\n\n.visible .entry main(\n",
        );
        for i in 0..10 {
            s.push_str(&format!("\t.param .u64 main_param_{i},\n"));
        }
        s.push_str(")\n{\n");
        s.push_str("\tld.param.u64 %rd1, [main_param_1];\n\tcvta.to.global.u64 %rd2, %rd1;\n\tret;\n}\n");
        s
    }

    #[test]
    fn facts_are_read_from_the_module() {
        let f = ptx_facts(&clamp_ptx()).expect("facts");
        assert_eq!(f.entry, "main");
        assert_eq!(f.params, 10);
        assert_eq!(f.target_chip, "sm_70");
        assert_eq!(f.address_size, 64);
        assert!(f.converts_pointers);
    }

    #[test]
    fn capability_parses_the_target() {
        assert_eq!(capability_of("sm_70"), Some((7, 0)));
        assert_eq!(capability_of("sm_61"), Some((6, 1)));
        assert_eq!(capability_of("sm_90a"), Some((9, 0)));
        assert_eq!(capability_of("compute_70"), None);
    }

    /// The emitter's current shape: `ctaid.x * ntid.x + tid.x`.
    fn global_ptx() -> String {
        clamp_ptx().replace(
            "ld.param.u64 %rd1, [main_param_1];",
            "mov.u32 %r1, %ctaid.x;\n\tmov.u32 %r2, %ntid.x;",
        )
    }

    #[test]
    fn a_checked_plan_covers_the_extent_with_blocks() {
        let f = ptx_facts(&global_ptx()).unwrap();
        // Extent 65 at the declared 64-thread block is two blocks: the case the
        // emitter got wrong before G21a.
        let p = plan(&f, 10, 1, 65, 65, 1, 64, None, Some((7, 5))).unwrap();
        assert_eq!((p.grid, p.block, p.extent), (2, 64, 65));
        assert_eq!(p.params(), 10);
        let one = plan(&f, 10, 1, 64, 64, 1, 64, None, None).unwrap();
        assert_eq!((one.grid, one.block), (1, 64));
        let split = plan(&f, 10, 1, 64, 64, 1, 64, Some(32), None).unwrap();
        assert_eq!((split.grid, split.block), (2, 32));
        assert_eq!(split.grid * split.block >= 64, true);
    }

    #[test]
    fn a_global_index_module_is_recognised() {
        let f = ptx_facts(&global_ptx()).unwrap();
        assert!(f.uses_block_id);
        assert!(!ptx_facts(&clamp_ptx()).unwrap().uses_block_id);
    }

    #[test]
    fn a_grid_over_a_block_local_module_is_refused() {
        // One block is fine for either shape; a grid is only fine when the PTX
        // reads `%ctaid`.
        let local = ptx_facts(&clamp_ptx()).unwrap();
        assert!(plan(&local, 10, 1, 65, 65, 1, 65, None, None).is_ok());
        assert!(matches!(
            plan(&local, 10, 1, 65, 65, 1, 64, None, None),
            Err(Refusal::NeedsBlockIndex { extent: 65, block: 64 })
        ));
    }

    #[test]
    fn every_mismatch_is_named_before_the_driver() {
        let f = ptx_facts(&global_ptx()).unwrap();
        assert!(matches!(
            plan(&f, 10, 1, 65, 0, 1, 64, None, None),
            Err(Refusal::ExtentZero)
        ));
        assert!(matches!(
            plan(&f, 10, 1, 65, 64, 1, 64, None, None),
            Err(Refusal::ExtentMismatch { recorded: 65, requested: 64 })
        ));
        assert!(matches!(
            plan(&f, 10, 1, 65, 65, 2, 64, None, None),
            Err(Refusal::Arity { expected: 1, got: 2 })
        ));
        assert!(matches!(
            plan(&f, 15, 2, 65, 65, 2, 64, None, None),
            Err(Refusal::ParamCount { recorded: 15, ptx: 10 })
        ));
        assert!(matches!(
            plan(&f, 10, 1, 65, 65, 1, 64, None, Some((6, 1))),
            Err(Refusal::TargetNewerThanDevice { .. })
        ));
        assert!(matches!(
            plan(&f, 10, 1, 65, 65, 1, 64, Some(2048), None),
            Err(Refusal::BlockTooLarge { block: 2048 })
        ));
        assert!(matches!(
            plan(&f, 10, 1, 65, 65, 1, 64, Some(0), None),
            Err(Refusal::BlockZero)
        ));
        assert!(plan(&f, 10, 1, 65, 65, 1, 64, None, Some((7, 5))).is_ok());
    }

    #[test]
    fn a_module_without_address_conversion_is_refused() {
        let f = ptx_facts(&global_ptx().replace("cvta.to.global.u64 %rd2, %rd1;", "")).unwrap();
        assert!(matches!(
            plan(&f, 10, 1, 65, 65, 1, 64, None, None),
            Err(Refusal::NoAddressConversion)
        ));
    }

    #[test]
    fn a_missing_entry_is_refused() {
        let mut f = ptx_facts(&global_ptx()).unwrap();
        f.entry = "clamp".to_string();
        assert!(matches!(
            plan(&f, 10, 1, 65, 65, 1, 64, None, None),
            Err(Refusal::EntryMissing { .. })
        ));
    }
}
