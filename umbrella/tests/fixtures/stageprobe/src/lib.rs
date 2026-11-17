//! Marker written by the library initializer — i.e. when the cdylib is
//! actually mapped into a process. A build that is authorized but never
//! loaded leaves `stages.log` at `build` alone, which is what lets a test tell
//! "built" apart from "loaded".

use std::io::Write;

// Baked in by build.rs from `STAGEPROBE_DIR`: the initializer must not read
// the loader's environment, or a test could not point two parallel fixtures at
// different files.
const MARKER: Option<&str> = option_env!("STAGEPROBE_FILE");

extern "C" fn stageprobe_init() {
    let Some(path) = MARKER else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "load");
    }
}

#[used]
#[link_section = ".init_array"]
static STAGEPROBE_INIT: extern "C" fn() = stageprobe_init;

/// Keeps the cdylib from being an empty object and gives the test something
/// to resolve after `dlopen`.
#[no_mangle]
pub extern "C" fn stageprobe_version() -> i64 {
    1
}
