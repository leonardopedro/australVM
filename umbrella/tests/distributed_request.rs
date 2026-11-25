//! D17 acceptance: the linear remote-resource wrappers execute through the
//! umbrella front-end (compile-only CPS) and the bridge's in-process JIT.
//!
//! The module is deliberately the umbrella's own copy (`modules/`), not the
//! `examples/` demo: the point is that an authored umbrella module can use the
//! stdlib wrappers without the host learning about the wrapper's fields.

use std::path::PathBuf;

#[test]
fn a_distributed_request_runs_through_the_umbrella_front_end() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().expect("umbrella is inside australVM");
    let pair = PathBuf::from(format!(
        "{}/stdlib/net/Distributed.aui,{}/stdlib/net/Distributed.aum",
        root.display(),
        root.display()
    ));
    let module = manifest.join("modules/DistributedRequest.aum");
    let austral = root.join("austral");

    let dependency = umbrella::frontend::compile(&austral, &[pair.clone()], false)
        .expect("the umbrella front-end must emit the Distributed module's CPS");
    let artifact = umbrella::frontend::compile(&austral, &[pair, module], false)
        .expect("the umbrella front-end must emit the D17 module's CPS");
    assert!(
        artifact.exports.iter().any(|e| e.name == "run"),
        "the front-end must report the distributed entry export"
    );

    austral_cranelift_bridge::auth::set_allow_all();
    let dependency_fn = austral_cranelift_bridge::compile_to_function_named(
        dependency.cps.as_ptr(),
        dependency.cps.len(),
        std::ptr::null(),
        0,
    );
    assert!(
        !dependency_fn.is_null(),
        "the bridge must compile the Distributed definitions"
    );
    let entry = b"run";
    let function = austral_cranelift_bridge::compile_entry_i64(
        artifact.cps.as_ptr(),
        artifact.cps.len(),
        entry.as_ptr(),
        entry.len(),
        0,
    );
    if function.is_null() {
        let err = austral_cranelift_bridge::cranelift_last_error();
        let text = if err.is_null() {
            "<no bridge error>".to_string()
        } else {
            unsafe { std::ffi::CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned()
        };
        panic!("the bridge must compile the D17 entry after the front-end emitted CPS: {text}");
    }
    let result = austral_cranelift_bridge::execute_function(function);
    assert_eq!(result, 0, "the D17 request must finish applied");
}
