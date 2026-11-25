//! U10 linkage: an adapter crate built against the admitted provider resolves
//! its `rs_*` call through that image while both are mapped `RTLD_LOCAL`. Before
//! this, the adapter relied on the provider being in the global scope, so a
//! preloaded same-named export could answer instead.

use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::{adapters, interface, policy, provider};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("umbrella-linking-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn provider_library(dir: &Path) -> PathBuf {
    let source = dir.join("provider.c");
    std::fs::write(
        &source,
        "#include <stdint.h>\nint64_t rs_link_probe(void) { return 7; }\n",
    )
    .expect("provider source");
    let library = dir.join("liblink_probe.so");
    let cc = std::env::var_os("CC").unwrap_or_else(|| "cc".into());
    let out = Command::new(cc)
        .arg("-shared")
        .arg("-fPIC")
        .arg("-o")
        .arg(&library)
        .arg(&source)
        .output()
        .expect("spawn the C compiler");
    assert!(
        out.status.success(),
        "fixture build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    library
}

#[test]
fn a_linked_adapter_resolves_the_locally_mapped_provider() {
    let dir = scratch("local");
    let library = provider_library(&dir);
    let set = provider::ProviderSet::load(&[library.clone()], &["rs_link_probe".to_string()])
        .expect("admit the provider");
    assert_eq!(set.mapped(), 1);

    let adapter = interface::Adapter::new(
        "rs_link_probe__adapter",
        "rs_link_probe",
        Vec::new(),
        interface::Scalar::I64,
    );
    let generated = interface::GeneratedAdapter::Scalar(&adapter);
    let built = adapters::build_next_against(&[generated], Some(&library))
        .expect("the adapter links the admitted provider");
    adapters::load_for_jit(&built).expect("map the adapter locally");

    // The provider is RTLD_LOCAL, so this only resolves if the adapter linked
    // it. A global-scope dependency would have failed the local load above.
    let image = policy::dlopen_local(&built.path).expect("reuse the mapped adapter");
    let name = std::ffi::CString::new("rs_link_probe__adapter").unwrap();
    let address = unsafe { libc::dlsym(image.as_ptr(), name.as_ptr()) };
    assert!(!address.is_null(), "the adapter symbol is exported");
    let call: extern "C" fn() -> u64 = unsafe { std::mem::transmute(address) };
    assert_eq!(call(), 7, "the provider's implementation ran");
}
