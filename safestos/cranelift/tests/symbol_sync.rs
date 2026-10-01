//! Auto-sync tests: verify the bridge's registered symbol set matches
//! unfer_ffi's `EXPECTED_SYMBOLS.txt` / `EXPECTED_SYMBOLS_ZENODO.txt`.
//!
//! Run with: `cargo test --features unfer-kernel` (included in default features).
//!
//! When adding a new `uk_*` or `uz_*` symbol to unfer_ffi, you must also:
//! 1. Add it to `EXPECTED_SYMBOLS.txt` (or `EXPECTED_SYMBOLS_ZENODO.txt`) in unfer.
//! 2. Add it to the `UNFER_SYMBOLS` / `ZENODO_SYMBOLS` table in `lib.rs`.
//!    This test will catch any mismatch.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

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
    let content =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    content
        .lines()
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
    assert!(
        handle < 0,
        "uk_model_create(null,0) should return negative error code, got {handle}"
    );
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

/// E6 linkage smoke test: the engram symbols are reachable from the bridge and
/// round-trip through the real ABI, not just present in the census.
///
/// The key is 84 opaque bytes in `logos::engram`'s canonical `ENGM` layout.
/// Both sides pin that width independently — deriving a key is `logos`' job,
/// and neither crate depends on the other to hold bytes — so this test uses the
/// same layout constants rather than importing them.
#[test]
#[cfg(feature = "unfer-kernel")]
fn uk_engram_round_trips_through_the_bridge() {
    const KEY_BYTES: usize = 84;

    // A real session spec, not a stub: the engram table lives on `Session`, so
    // there has to be a session for the symbols to act on.
    let spec = br#"{
      "hamiltonian": {
        "kind": "builtin",
        "name": "harmonic_chain",
        "params": {"n_modes": 2, "omega": 1.0}
      },
      "prior": {
        "kind": "superposition",
        "terms": [
          {"re": 0.5, "im": 0.0, "spec": {"kind": "vacuum"}},
          {"re": 0.5, "im": 0.0, "spec": {"kind": "bosons", "modes": [[0, 1]]}}
        ]
      },
      "solver": {
        "krylov_dim": 4,
        "prune_eps": 1e-12,
        "max_components": 50000,
        "restarts": 1,
        "device": {"kind": "cpu"}
      }
    }"#;
    let model = unfer_ffi::uk_model_create(spec.as_ptr(), spec.len() as i64);
    assert!(model > 0, "uk_model_create failed");

    let mut key = vec![0u8; KEY_BYTES];
    key[0..4].copy_from_slice(b"ENGM");
    key[4..6].copy_from_slice(&1u16.to_le_bytes());
    key[32..40].copy_from_slice(&0xA5A5_A5A5_A5A5_A5A5u64.to_le_bytes());
    key[76..84].copy_from_slice(&0.25f64.to_le_bytes());

    // A miss is UK-4403, distinct from a stored zero.
    assert_eq!(
        -4403,
        unfer_ffi::uk_engram_lookup(model, key.as_ptr(), KEY_BYTES as i64),
        "an unstored engram is RESOURCE_NOT_FOUND, not a zero weight"
    );

    assert_eq!(
        0,
        unfer_ffi::uk_engram_store(model, key.as_ptr(), KEY_BYTES as i64, 0.25f64.to_bits() as i64)
    );
    assert_eq!(0, unfer_ffi::uk_engram_lookup(model, key.as_ptr(), KEY_BYTES as i64));

    // Storing again replaces rather than accumulates, so a repeated ingest
    // cannot inflate an engram's weight.
    assert_eq!(
        0,
        unfer_ffi::uk_engram_store(model, key.as_ptr(), KEY_BYTES as i64, 0.25f64.to_bits() as i64)
    );
    let needed = unfer_ffi::uk_get_result(model, std::ptr::null_mut(), 0);
    assert!(needed > 0);
    let mut buf = vec![0u8; needed as usize];
    unfer_ffi::uk_get_result(model, buf.as_mut_ptr(), buf.len() as i64);
    let result = String::from_utf8_lossy(&buf);
    assert!(
        result.contains(r#""entries":1"#),
        "re-storing must not create a second entry: {result}"
    );
    assert!(
        result.contains(r#""replaced":0.25"#),
        "the displaced weight is reported, not silently dropped: {result}"
    );

    // A wrong-length key is refused at the boundary.
    assert!(
        unfer_ffi::uk_engram_store(model, key.as_ptr(), 83, 0.25f64.to_bits() as i64) < 0,
        "an 83-byte key must not be accepted"
    );

    unfer_ffi::uk_model_free(model);
}
