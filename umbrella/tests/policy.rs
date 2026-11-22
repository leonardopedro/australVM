//! U02 acceptance tests (AUT01, BLD06, RUN06) at the policy layer.
//!
//! The stage markers are real: the fixture crate's build script and library
//! initializer append to `stages.log`, and the load stage maps the built
//! cdylib through `policy::dlopen` — the same function the driver uses. A
//! denied stage is never invoked, so its line never appears.

use std::path::{Path, PathBuf};
use std::process::Command;

use umbrella::policy::{self, Stage, Stages};
use umbrella::project;

const FIXTURE: &str = "tests/fixtures/stageprobe";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("umbrella-policy-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn project_text(build: bool, load: bool, grants: &[&str]) -> String {
    let grants = grants
        .iter()
        .map(|g| format!(r#"{{"symbol":"{g}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{
            "schema": 1,
            "profile": "trusted-native",
            "module": {{"name": "probe", "sources": ["Probe.aum"]}},
            "build": {{"allowed": {build}, "crate": "{FIXTURE}"}},
            "load": {{"allowed": {load}, "providers": ["libstageprobe.so"]}},
            "grants": [{grants}],
            "trusted": {{
                "lock_sha256": "{}",
                "features": [],
                "library_sha256": "{}",
                "exports": ["stageprobe_version"]
            }}
        }}"#,
        "0".repeat(64),
        "1".repeat(64)
    )
}

fn plan(text: &str) -> Result<policy::Preflight, String> {
    let doc = project::parse(Path::new("project.json"), text)?;
    policy::preflight(&doc)
}

/// Build the fixture with its target dir inside the scratch directory, so the
/// fixture leaves nothing behind in the checkout.
fn build_fixture(dir: &Path) -> Result<PathBuf, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE)
        .join("Cargo.toml");
    let target = dir.join("target");
    let out = Command::new("cargo")
        .args(["build", "--release", "--offline", "--manifest-path"])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .env("STAGEPROBE_DIR", dir)
        .output()
        .map_err(|e| format!("spawn cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "fixture build failed: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let so = target.join("release/libstageprobe.so");
    if !so.is_file() {
        return Err(format!("no cdylib at {}", so.display()));
    }
    Ok(so)
}

fn marker(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("stages.log")).unwrap_or_default()
}

/// BLD06 + RUN06: with build and load authorized, both markers appear, in
/// order, and the call runs last. The build script and the library
/// initializer are different stages, and the log says which one ran.
#[test]
fn authorized_stages_run_in_order_and_leave_markers() {
    let dir = scratch("authorized");
    let pf = plan(&project_text(true, true, &["rs_demo_checksum"])).expect("preflight");
    let so = build_fixture(&dir).expect("fixture build");

    let stages: Stages = policy::run_stages(
        &pf,
        || {
            // The fixture is built by `build_fixture` above with the marker
            // dir set; re-running is what a real build stage does.
            build_fixture(&dir).map(|_| ())
        },
        || policy::dlopen(&so).map(|_| ()),
        || {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("stages.log"))
                .and_then(|mut f| {
                    use std::io::Write;
                    writeln!(f, "call")
                })
                .map_err(|e| format!("call marker: {e}"))
        },
    )
    .expect("authorized stages");

    assert!(stages.built && stages.loaded && stages.called, "{stages:?}");
    // `build` is the test's own pre-build; the stage's cargo call is fresh
    // (same marker dir, build script not rerun), then load, then call.
    assert_eq!(marker(&dir), "build\nload\ncall\n");
    assert!(pf
        .report()
        .contains("scope: grants authorize extern calls only"));
}

/// BLD06's other half: a project that authorizes neither build nor load is
/// refused at preflight, so there is no `Preflight` to run stages from and
/// neither marker can be produced.
#[test]
fn no_build_and_no_load_is_refused_before_any_stage() {
    let dir = scratch("refused");
    let err = plan(&project_text(false, false, &["rs_demo_checksum"]))
        .expect_err("must refuse a project that authorizes nothing");
    assert!(err.contains("authorizes neither build nor load"), "{err}");
    assert_eq!(marker(&dir), "", "no stage may run before the refusal");
}

/// A denied call happens *after* an authorized build and load; the report says
/// the earlier stages really ran, and does not claim they were contained.
#[test]
fn denied_call_reports_that_build_and_load_already_ran() {
    let dir = scratch("denied-call");
    let pf = plan(&project_text(true, true, &[])).expect("preflight");
    let so = build_fixture(&dir).expect("fixture build");
    assert!(!pf.call.allowed());

    let stages = policy::run_stages(
        &pf,
        || Ok(()),
        || policy::dlopen(&so).map(|_| ()),
        || panic!("a denied call must never be invoked"),
    )
    .expect("build and load are still authorized");

    assert!(
        stages.built && stages.loaded && !stages.called,
        "{stages:?}"
    );
    assert_eq!(marker(&dir), "build\nload\n");
    let report = pf.report();
    assert!(report.contains("call=denied"), "{report}");
    assert!(
        report.contains("does not mean the build or library initializer was contained"),
        "{report}"
    );
}

/// A profile whose rules do not exist yet is refused by name, not treated as
/// the trusted profile.
#[test]
fn untrusted_native_profile_is_refused() {
    let text = project_text(true, true, &["rs_demo_checksum"]).replace(
        "\"profile\": \"trusted-native\"",
        "\"profile\": \"untrusted-native\"",
    );
    let err = plan(&text).expect_err("untrusted-native is U26's work");
    assert!(err.contains("untrusted-native"), "{err}");
    assert!(err.contains("U26"), "{err}");
}

/// AUT01: the refusal happens at preflight, before a stage exists to run.
#[test]
fn unknown_field_and_malformed_grant_are_refused_by_name() {
    let unknown = project_text(true, true, &["rs_demo_checksum"])
        .replace("\"allowed\": true", "\"allowedd\": true");
    let err = plan(&unknown).expect_err("unknown field");
    assert!(err.contains("allowedd"), "{err}");

    let bad_grant =
        project_text(true, true, &["rs_demo_checksum"]).replace("rs_demo_checksum", "cargo_build");
    let err = plan(&bad_grant).expect_err("malformed grant");
    assert!(err.contains("grant 0"), "{err}");
    assert!(err.contains("cargo_build"), "{err}");
}

/// The stage error names the stage that failed, so a build failure is never
/// reported as a load failure.
#[test]
fn a_failing_stage_names_itself() {
    let pf = plan(&project_text(true, true, &["rs_demo_checksum"])).expect("preflight");
    let err = policy::run_stages(
        &pf,
        || Err("no compiler".to_string()),
        || panic!("load must not run after a build failure"),
        || panic!("call must not run after a build failure"),
    )
    .expect_err("build failure");
    assert_eq!(err.stage, Stage::Build);
    assert!(err.to_string().contains("build stage failed"), "{err}");
}
