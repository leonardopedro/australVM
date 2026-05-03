fn main() {
    println!("cargo:rerun-if-changed=../runtime");
    println!("cargo:rerun-if-changed=../include");

    cc::Build::new()
        .include("../include")
        .file("../runtime/scheduler.c")
        .file("../runtime/cell_loader.c")
        .file("../runtime/serialize.c")
        .file("../runtime/region.c")
        .file("../runtime/capabilities.c")
        .compile("safestos_runtime");
}
