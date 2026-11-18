//! Naga parsing/validation and wrapper reflection for a loaded bundle.

use crate::schema::KernelBundle;

pub fn validate_wgsl(wgsl: &str, bundle: &KernelBundle) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(wgsl)
        .map_err(|e| format!("WGSL parse error: {}", e.emit_to_string(wgsl)))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    validator
        .validate(&module)
        .map_err(|e| format!("Naga validation failed: {e:?}"))?;
    crate::wrapper::audit(&module, bundle.input_arity)
        .map_err(|e| format!("memory-wrapper audit failed: {e}"))
}

/// Check the bundle end to end: schema/digests already happened at load time;
/// this adds the shader validation and wrapper audit.
pub fn validate_bundle(loaded: &crate::LoadedBundle) -> Result<(), String> {
    crate::artifact::check_requirements(&loaded.bundle)?;
    validate_wgsl(&loaded.wgsl, &loaded.bundle)
}
