//! Scratch space shared by the umbrella integration tests.
//!
//! Each test file builds its fixtures under `$TMPDIR/<prefix>-<pid>-*`. Some of
//! those fixtures are debug cdylibs of the whole bridge — a generated wrapper
//! crate is ~585 MB — so leftovers accumulate across runs, and once the disk
//! fills every later `rustc` stalls and a green suite looks hung. The pid in the
//! name is what makes cleanup safe: a process only removes the directories its
//! own pid minted, so two runs sharing a temp dir never delete each other's.

use std::sync::{Mutex, Once, OnceLock};

/// Register `<prefix>-<pid>-*` for removal when this test binary exits.
///
/// `atexit` covers the normal exit a green run takes; a killed process can
/// still leave a directory behind, which is why the pid scoping matters. Safe to
/// call more than once, and from several threads.
pub fn cleanup_on_exit(prefix: &str) {
    static HANDLER: Once = Once::new();
    let mut registered = prefixes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if registered.iter().any(|p| p == prefix) {
        return;
    }
    registered.push(prefix.to_string());
    drop(registered);
    HANDLER.call_once(|| unsafe {
        libc::atexit(remove_scratch_dirs);
    });
}

fn prefixes() -> &'static Mutex<Vec<String>> {
    static PREFIXES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    PREFIXES.get_or_init(|| Mutex::new(Vec::new()))
}

extern "C" fn remove_scratch_dirs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let pid = std::process::id();
    let registered = prefixes()
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default();
    let names: Vec<String> = registered
        .iter()
        .map(|prefix| format!("{prefix}-{pid}-"))
        .collect();
    for entry in entries.flatten() {
        let name = entry.file_name();
        if names.iter().any(|n| name.to_string_lossy().starts_with(n)) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
