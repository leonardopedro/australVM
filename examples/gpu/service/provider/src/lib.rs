//! G14 provider: the GPU ABI is implemented in `safestos/gpu`; this crate only
//! makes it a `cdylib` the manifest-authorized umbrella build can load.
//!
//! The re-export is what forces the `#[no_mangle]` symbols into the shared
//! object. Do not wrap them under the same names in this crate: the dependency
//! already exports them, and a second definition is a duplicate-symbol link
//! error.

pub use avm_gpu::service::{
    rs_gpu_buffer_free, rs_gpu_buffer_new, rs_gpu_buffer_read, rs_gpu_buffer_write,
    rs_gpu_cancel, rs_gpu_close, rs_gpu_open, rs_gpu_poll, rs_gpu_result, rs_gpu_run,
    rs_gpu_service_version, rs_gpu_submit,
};
