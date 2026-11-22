//! U03 acceptance tests (CLI01–CLI06) against the real binary.
//!
//! Every malformed invocation must exit 2 *before* any stage: a fake `cargo`
//! and a recording `--austral` are on PATH/argv, and their invocation markers
//! must stay absent. Parser-level tests for OS-native paths live in
//! `src/cli.rs`; the last test here also drives a non-UTF-8 path through the
//! binary to the front-end.


mod support;

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_umbrella"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("umbrella-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    support::cleanup_on_exit("umbrella-cli");
    dir
}

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    let mut perm = std::fs::metadata(path).expect("meta").permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(path, perm).expect("chmod");
}

/// A PATH whose `cargo` records that it was spawned, then fails.
fn fake_cargo(dir: &Path) -> (PathBuf, PathBuf) {
    let tools = dir.join("tools");
    std::fs::create_dir_all(&tools).expect("tools dir");
    let marker = dir.join("cargo-was-spawned");
    executable(
        &tools.join("cargo"),
        &format!("#!/bin/sh\n: > '{}'\nexit 1\n", marker.display()),
    );
    (tools, marker)
}

fn run_with_path(cmd: &mut Command, tools: &Path) -> Output {
    cmd.env("PATH", format!("{}:/usr/bin:/bin", tools.display()))
        .output()
        .expect("run umbrella")
}

#[test]
fn help_and_version_exit_zero_without_touching_a_stage() {
    let dir = scratch("help");
    let (tools, cargo_marker) = fake_cargo(&dir);
    for args in [
        vec!["--help"],
        vec!["--version"],
        vec!["modules/D.aum", "--help"],
        vec!["--manifest", "m.toml", "--version"],
    ] {
        let out = run_with_path(bin().args(&args), &tools);
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("usage: umbrella") || stdout.starts_with("umbrella 0.1.0"),
            "{args:?}: {stdout}"
        );
    }
    assert!(
        !cargo_marker.exists(),
        "no build may be spawned for help/version"
    );
}

#[test]
fn malformed_invocations_exit_2_and_spawn_nothing() {
    let dir = scratch("malformed");
    let (tools, cargo_marker) = fake_cargo(&dir);
    let cases: &[(&[&str], &str)] = &[
        (&[], "at least one .aum"),
        (
            &["--manifest-garbage", "g.toml", "D.aum"],
            "unknown option `--manifest-garbage`",
        ),
        (
            &["--entry", "--expect", "7", "D.aum"],
            "`--entry` needs a value",
        ),
        (&["--entry=", "D.aum"], "`--entry` must not be empty"),
        (&["-x", "D.aum"], "unknown option `-x`"),
        (&["--expect", "abc", "D.aum"], "wants an i64"),
        (
            &["--entry", "a", "--entry", "b", "D.aum"],
            "duplicate option `--entry`",
        ),
        (&["--json=yes", "D.aum"], "`--json` takes no value"),
    ];
    for (args, needle) in cases {
        let out = run_with_path(bin().args(*args), &tools);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(needle), "{args:?}: {stderr}");
        assert!(!cargo_marker.exists(), "{args:?} must not spawn cargo");
    }
}

#[test]
fn json_mode_prints_one_typed_event_and_diagnostics_on_stderr() {
    let dir = scratch("json");
    let missing = dir.join("absent-project.json");
    let out = bin()
        .arg("--json")
        .arg("--project")
        .arg(&missing)
        .output()
        .expect("run umbrella");
    assert_eq!(out.status.code(), Some(1), "operational failure");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "stdout must carry only protocol: {stdout}");
    let event: serde_json::Value = serde_json::from_str(lines[0]).expect("typed event");
    assert_eq!(event["event"], "error");
    assert!(!out.stderr.is_empty(), "human diagnostic stays on stderr");
    assert!(!stdout.contains("RESULT"), "{stdout}");
}

#[test]
fn a_usage_error_writes_no_protocol_at_all() {
    let out = bin()
        .args(["--json", "--entry", "--expect", "7", "D.aum"])
        .output()
        .expect("run umbrella");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// CLI05: the legacy demo invocation still parses, and a failure after
/// parsing is operational (1), not usage (2).
#[test]
fn the_legacy_demo_invocation_is_not_a_usage_error() {
    let dir = scratch("legacy");
    let (tools, cargo_marker) = fake_cargo(&dir);
    let mut cmd = bin();
    cmd.args([
        "--manifest",
        dir.join("module.toml").to_str().unwrap(),
        "--entry",
        "compute",
        "modules/Demo.aum",
    ]);
    let out = run_with_path(&mut cmd, &tools);
    assert_eq!(out.status.code(), Some(1), "cargo failure is operational");
    assert!(
        cargo_marker.exists(),
        "the parse succeeded, so the build stage ran"
    );
}

/// CLI04 end to end: an OS path that is not valid UTF-8 reaches the tool with
/// its bytes intact. The project authorizes load (a real cdylib) but not
/// build, so the stage before the front-end needs no compiler.
#[test]
fn a_non_utf8_path_reaches_the_front_end_unchanged() {
    let dir = scratch("os-paths");
    let so = build_stageprobe(&dir);
    let project = dir.join("project.json");
    std::fs::write(
        &project,
        serde_json::json!({
            "schema": 1,
            "profile": "dev",
            "module": {"name": "probe", "sources": ["unused.aum"]},
            "build": {"allowed": false},
            "load": {"allowed": true, "providers": [so]},
            "grants": [{"symbol": "rs_probe"}],
            // D09: a load authorization pins the bytes it approves.
            "trusted": {
                "lock_sha256": "0".repeat(64),
                "features": [],
                "library_sha256": umbrella::trust::sha256_file(&so)
                    .expect("hash the fixture library"),
                "exports": ["stageprobe_version"]
            }
        })
        .to_string(),
    )
    .expect("write project");

    let seen = dir.join("front-end-argv");
    let fake_austral = dir.join("fake-austral");
    executable(
        &fake_austral,
        &format!(
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > '{}'\nexit 1\n",
            seen.display()
        ),
    );

    let raw = vec![
        b'/', b't', b'm', b'p', b'/', 0xff, b'-', b'm', b'o', b'd', b'.', b'a', b'u', b'm',
    ];
    let weird = OsString::from_vec(raw.clone());
    let out = bin()
        .arg("--project")
        .arg(&project)
        .arg("--austral")
        .arg(&fake_austral)
        .arg("--")
        .arg(&weird)
        .output()
        .expect("run umbrella");

    // The front-end refused (exit 1), so umbrella's status is operational.
    assert_eq!(
        out.status.code(),
        Some(1),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let argv = std::fs::read(&seen).expect("front-end was invoked and recorded argv");
    assert!(
        argv.windows(raw.len()).any(|w| w == raw.as_slice()),
        "the exact bytes must reach the tool: {:?}",
        String::from_utf8_lossy(&argv)
    );
}

fn build_stageprobe(dir: &Path) -> PathBuf {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stageprobe/Cargo.toml");
    let target = dir.join("probe-target");
    let out = Command::new("cargo")
        .args(["build", "--release", "--offline", "--manifest-path"])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .env("STAGEPROBE_DIR", dir)
        .output()
        .expect("spawn cargo for fixture");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    target.join("release/libstageprobe.so")
}
