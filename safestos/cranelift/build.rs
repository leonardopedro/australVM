fn main() {
    println!("cargo:rerun-if-changed=../runtime");
    println!("cargo:rerun-if-changed=../include");

    // Guard: when the unfer_ffi path dep is enabled, verify the sibling
    // unfer repo is present and its Cargo.toml is parseable. The actual
    // compilation check happens at link time (cargo resolves deps); this
    // guard provides a clear early error message for the common case of a
    // missing or misaligned unfer tree.
    #[cfg(feature = "unfer-kernel")]
    {
        let unfer_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../unfer");
        let unfer_toml = unfer_dir.join("Cargo.toml");
        if !unfer_toml.exists() {
            panic!(
                "unfer sibling repo not found at {}.\n\
                 Expected `unfer_ffi` path dep to resolve to `../../../unfer/unfer_ffi`.\n\
                 Clone unfer next to australVM so the path dep works.",
                unfer_dir.display()
            );
        }
    }

    cc::Build::new()
        .include("../include")
        .file("../runtime/scheduler.c")
        .file("../runtime/cell_loader.c")
        .file("../runtime/serialize.c")
        .file("../runtime/region.c")
        .file("../runtime/capabilities.c")
        .compile("safestos_runtime");
}
