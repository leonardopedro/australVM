//! U05 acceptance against a **real** Cargo stream.
//!
//! The workspace in `tests/fixtures/cargo-workspace` exists for its shape: a
//! proc-macro (also a `.so`), a transitive library (only an `.rlib`), and two
//! cdylib targets the old "first shared filename wins" rule could confuse.
//! Nothing here is synthetic: the JSON is what `cargo build` printed, into a
//! custom `--target-dir`, so a selection is pinned to files that exist.

use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::cargo::{
    build_args, select_shared_artifact, wanted_from_metadata, wanted_from_metadata_for,
    BuildRequest, Profile,
};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-workspace")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("umbrella-cargo-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn metadata() -> String {
    let out = Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version=1")
        .arg("--manifest-path")
        .arg(workspace().join("Cargo.toml"))
        .output()
        .expect("spawn cargo metadata");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Build the fixture workspace into `target_dir` and return cargo's JSON stream.
fn build(target_dir: &Path) -> String {
    let out = Command::new("cargo")
        .arg("build")
        .arg("--offline")
        .arg("--message-format=json")
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--manifest-path")
        .arg(workspace().join("Cargo.toml"))
        .output()
        .expect("spawn cargo build");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn artifacts(stream: &str) -> Vec<serde_json::Value> {
    stream
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v["reason"] == "compiler-artifact")
        .collect()
}

fn wanted_for(package_dir: &str) -> umbrella::cargo::Wanted {
    wanted_from_metadata(&metadata(), &workspace().join(package_dir)).expect("identity")
}

/// Preconditions: if the fixture stopped producing a proc-macro `.so` or a
/// shared lib with no cdylib, the selection tests below would pass for the
/// wrong reason.
#[test]
fn the_fixture_stream_really_carries_the_confusable_artifacts() {
    let dir = scratch("preconditions");
    let stream = build(&dir);
    let seen = artifacts(&stream);
    assert!(
        seen.iter().any(|a| {
            a["target"]["kind"]
                .as_array()
                .map(|k| k.iter().any(|k| k == "proc-macro"))
                .unwrap_or(false)
                && a["filenames"]
                    .as_array()
                    .map(|f| f.iter().any(|p| p.as_str().unwrap_or("").ends_with(".so")))
                    .unwrap_or(false)
        }),
        "the fixture must emit a proc-macro .so"
    );
    assert!(
        seen.iter().any(|a| {
            a["target"]["name"] == "fixture_shared"
                && a["filenames"]
                    .as_array()
                    .map(|f| f.iter().all(|p| !p.as_str().unwrap_or("").ends_with(".so")))
                    .unwrap_or(false)
        }),
        "the fixture must emit a transitive library with no shared artifact"
    );
    for name in ["fixture_alpha", "fixture_beta"] {
        assert!(
            seen.iter().any(|a| {
                a["filenames"].as_array().is_some_and(|f| {
                    f.iter()
                        .any(|p| p.as_str().unwrap_or("").ends_with(&format!("lib{name}.so")))
                })
            }),
            "the fixture must emit lib{name}.so"
        );
    }
}

#[test]
fn each_cdylib_is_selected_from_its_own_package_identity() {
    let dir = scratch("select");
    let stream = build(&dir);
    for (package_dir, package, name) in [
        ("alpha", "fixture-alpha", "fixture_alpha"),
        ("beta", "fixture-beta", "fixture_beta"),
    ] {
        let wanted = wanted_for(package_dir);
        assert_eq!(wanted.package, package);
        assert_eq!(wanted.target, name);
        let selected = select_shared_artifact(&stream, &wanted).expect("exact cdylib");
        assert_eq!(
            selected,
            dir.join(if cfg!(target_os = "macos") {
                format!("debug/lib{name}.dylib")
            } else {
                format!("debug/lib{name}.so")
            })
        );
        assert!(
            selected.starts_with(&dir),
            "selection must come from the custom target dir: {}",
            selected.display()
        );
    }
}

#[test]
fn a_package_and_target_from_different_identities_do_not_match() {
    let dir = scratch("mixed");
    let stream = build(&dir);
    let alpha = wanted_for("alpha");
    let beta = wanted_for("beta");
    assert_ne!(alpha.package_id, beta.package_id);
    let mixed = umbrella::cargo::Wanted {
        package_id: beta.package_id,
        package: alpha.package,
        target: alpha.target,
    };
    let err = select_shared_artifact(&stream, &mixed).unwrap_err();
    assert!(
        err.contains("no artifact for `fixture-alpha`/`fixture_alpha`"),
        "{err}"
    );
}

/// U05 bullet 2, against the real workspace: an explicit package scopes the
/// build, the profile is stated, and `--locked --offline` reach cargo as
/// separate arguments.
#[test]
fn explicit_package_profile_and_policy_reach_the_real_build() {
    let dir = scratch("explicit");
    let mut request = BuildRequest::new(workspace());
    request.package = Some("fixture-beta".into());
    request.target_dir = Some(dir.clone());
    request.profile = Profile::Dev;
    request.locked = true;
    request.offline = true;
    let out = Command::new("cargo")
        .args(build_args(&request))
        .current_dir(workspace())
        .output()
        .expect("spawn scoped cargo build");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stream = String::from_utf8_lossy(&out.stdout).into_owned();
    let seen = artifacts(&stream);
    assert!(
        seen.iter().any(|a| a["target"]["name"] == "fixture_beta"),
        "the requested package must be built"
    );
    assert!(
        seen.iter().all(|a| a["target"]["name"] != "fixture_alpha"),
        "--package must keep the other cdylib out of the build"
    );
    let wanted = wanted_from_metadata_for(&metadata(), &workspace(), Some("fixture-beta"))
        .expect("identity by package name");
    let selected = select_shared_artifact(&stream, &wanted).expect("beta only");
    assert!(
        selected.starts_with(&dir) && selected.ends_with("libfixture_beta.so"),
        "dev profile must land under the custom target dir: {}",
        selected.display()
    );
}

#[test]
fn the_transitive_library_is_refused_as_not_shared() {
    let dir = scratch("transitive");
    let stream = build(&dir);
    // `fixture-shared` has no cdylib, so identity reading refuses it. Take its
    // id from the same metadata anyway, so the selector's second guard (a
    // matching artifact with no shared filename) is pinned too.
    let meta: serde_json::Value = serde_json::from_str(&metadata()).expect("metadata json");
    let package = meta["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| p["name"] == "fixture-shared")
        .expect("fixture-shared in metadata");
    let forced = umbrella::cargo::Wanted {
        package_id: package["id"].as_str().expect("id").to_string(),
        package: "fixture-shared".into(),
        target: "fixture_shared".into(),
    };
    let err = select_shared_artifact(&stream, &forced).unwrap_err();
    assert!(err.contains("no shared-library filename"), "{err}");
}
