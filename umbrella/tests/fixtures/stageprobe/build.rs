//! Marker written when the crate's build script runs — i.e. when *build*
//! authorization was actually exercised. Silent when `STAGEPROBE_DIR` is
//! unset, so a stray compile outside the test writes nothing.
//!
//! The same directory is also baked into the library as `STAGEPROBE_FILE`, so
//! the initializer does not depend on the *loading* process's environment
//! (which a test cannot set per-thread): the two stages still share one file.

fn main() {
    println!("cargo:rerun-if-env-changed=STAGEPROBE_DIR");
    let Ok(dir) = std::env::var("STAGEPROBE_DIR") else {
        return;
    };
    let path = std::path::Path::new(&dir).join("stages.log");
    println!("cargo:rustc-env=STAGEPROBE_FILE={}", path.display());
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "build");
    }
}
