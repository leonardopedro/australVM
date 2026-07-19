//! Auto-sync tests: verify the bridge's registered symbol set matches
//! unfer_ffi's `EXPECTED_SYMBOLS.txt` / `EXPECTED_SYMBOLS_ZENODO.txt`.
//!
//! Run with: `cargo test --features unfer-kernel` (included in default features).
//!
//! When adding a new `uk_*` or `uz_*` symbol to unfer_ffi, you must also:
//! 1. Add it to `EXPECTED_SYMBOLS.txt` (or `EXPECTED_SYMBOLS_ZENODO.txt`) in unfer.
//! 2. Add it to the `UNFER_SYMBOLS` / `ZENODO_SYMBOLS` table in `lib.rs`.
//! This test will catch any mismatch.

use std::collections::BTreeSet;
use std::path::Path;
use std::fs;

/// Path to the sibling unfer repo, relative to CARGO_MANIFEST_DIR.
fn unfer_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../unfer")
        .as_path()
        // We leak to get a &'static Path; this runs once per test binary.
        .to_path_buf()
        .leak()
}

fn read_expected_symbols(rel_path: &str) -> BTreeSet<String> {
    let path = unfer_dir().join(rel_path);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    content.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
#[cfg(feature = "unfer-kernel")]
fn uk_symbols_match_expected() {
    let expected: BTreeSet<String> = read_expected_symbols("unfer_ffi/EXPECTED_SYMBOLS.txt");
    let registered: BTreeSet<String> = austral_cranelift_bridge::registered_unfer_symbols()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let missing_in_bridge: Vec<_> = expected.difference(&registered).collect();
    let extra_in_bridge: Vec<_> = registered.difference(&expected).collect();

    if !missing_in_bridge.is_empty() || !extra_in_bridge.is_empty() {
        let mut msg = String::new();
        if !missing_in_bridge.is_empty() {
            msg.push_str(&format!(
                "\n  MISSING from bridge (in EXPECTED_SYMBOLS.txt but not registered): {:?}",
                missing_in_bridge
            ));
        }
        if !extra_in_bridge.is_empty() {
            msg.push_str(&format!(
                "\n  EXTRA in bridge (registered but not in EXPECTED_SYMBOLS.txt): {:?}",
                extra_in_bridge
            ));
        }
        msg.push_str("\n\n  Fix: add missing symbols to `UNFER_SYMBOLS` in lib.rs, or update EXPECTED_SYMBOLS.txt.");
        panic!("{}", msg);
    }
}

#[test]
#[cfg(feature = "zenodo-store")]
fn uz_symbols_match_expected() {
    let expected: BTreeSet<String> = read_expected_symbols("unfer_ffi/EXPECTED_SYMBOLS_ZENODO.txt");
    let registered: BTreeSet<String> = austral_cranelift_bridge::registered_zenodo_symbols()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let missing_in_bridge: Vec<_> = expected.difference(&registered).collect();
    let extra_in_bridge: Vec<_> = registered.difference(&expected).collect();

    if !missing_in_bridge.is_empty() || !extra_in_bridge.is_empty() {
        let mut msg = String::new();
        if !missing_in_bridge.is_empty() {
            msg.push_str(&format!(
                "\n  MISSING from bridge (in EXPECTED_SYMBOLS_ZENODO.txt but not registered): {:?}",
                missing_in_bridge
            ));
        }
        if !extra_in_bridge.is_empty() {
            msg.push_str(&format!(
                "\n  EXTRA in bridge (registered but not in EXPECTED_SYMBOLS_ZENODO.txt): {:?}",
                extra_in_bridge
            ));
        }
        msg.push_str("\n\n  Fix: add missing symbols to `ZENODO_SYMBOLS` in lib.rs, or update EXPECTED_SYMBOLS_ZENODO.txt.");
        panic!("{}", msg);
    }
}

/// Linkage smoke tests beyond uk_version/uk_init.
#[test]
#[cfg(feature = "unfer-kernel")]
fn uk_model_create_free_round_trip() {
    // null/0 is an invalid JSON spec → returns UK-1001 as a negative error
    // handle. Freeing a negative handle is a defined no-op.
    let handle = unfer_ffi::uk_model_create(std::ptr::null(), 0);
    assert!(handle < 0, "uk_model_create(null,0) should return negative error code, got {handle}");
    unfer_ffi::uk_model_free(handle);
}

#[test]
#[cfg(feature = "unfer-kernel")]
fn uk_last_error_initially_empty() {
    let mut buf = [0u8; 8];
    let n = unfer_ffi::uk_last_error(buf.as_mut_ptr(), buf.len() as i64);
    // Before any error, the buffer should be empty (n=0) or contain a
    // zero-length string (n > 0 but first byte is '\0').
    assert!(
        n == 0 || buf[0] == 0,
        "uk_last_error() before any error: expected empty string, got len={n}"
    );
}
