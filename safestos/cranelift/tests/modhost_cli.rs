//! CLI contracts for the `modhost` binary (adapted from celld).
//!
//! These run the real binary, not the library: usage errors must exit 2 with
//! a message + usage line, `--help` must exit 0 with usage on stdout, option
//! values must be strict (no silent defaults, no silently dropped `--args`
//! tokens), and `--json` must emit one parseable object per line on stdout
//! while the human transcript outside it stays unchanged.

use std::path::{Path, PathBuf};
use std::process::Command;

fn modhost(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_modhost"))
        .args(args)
        .output()
        .expect("modhost binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn demo_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/modules/demo_hosted/module.toml")
}

// --- minimal module fixture, same shape tests/module_tests.rs builds ------

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

fn cps_v1_with_name(func_name: &str, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_u32(&mut buf, 0x43505331);
    write_u32(&mut buf, 1);
    write_str(&mut buf, func_name);
    write_u32(&mut buf, 0);
    buf.push(0);
    write_u32(&mut buf, body.len() as u32);
    buf.extend_from_slice(body);
    buf
}

fn body_return_const(val: i64) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0x07);
    body.push(0x01);
    write_i64(&mut body, val);
    body
}

fn make_module_dir(dir: &Path, name: &str, grants: &[&str], cps_data: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    let grants_toml: Vec<String> = grants.iter().map(|g| format!("    \"{g}\",")).collect();
    let toml = format!(
        "[module]\nname = \"{name}\"\nversion = \"0.1.0\"\narchetypes = [\"austral_cps\"]\narchetype = \"austral_cps\"\nentry = \"run\"\n\n[grants]\nkernel = [\n{}\n]\n",
        grants_toml.join("\n")
    );
    std::fs::write(dir.join("module.toml"), toml).unwrap();
    std::fs::write(dir.join("module.cps"), cps_data).unwrap();
}

fn temp_module(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("modhost_cli_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    make_module_dir(
        &dir,
        name,
        &["uk_version"],
        &cps_v1_with_name("run", &body_return_const(42)),
    );
    dir
}

// --- usage discipline -----------------------------------------------------

#[test]
fn help_exits_zero_with_usage_on_stdout() {
    for args in [vec!["--help"], vec!["-h"], vec!["host", "--help"]] {
        let (code, stdout, stderr) = modhost(&args);
        assert_eq!(code, 0, "{args:?} should exit 0, stderr={stderr}");
        assert!(stdout.contains("usage:"), "{args:?}: usage on stdout");
        assert!(stdout.contains("modhost authorize"), "{args:?}");
    }
}

#[test]
fn unknown_option_is_a_usage_error() {
    let (code, stdout, stderr) = modhost(&["host", "/nonexistent", "--bogus"]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty(), "usage errors stay off stdout");
    assert!(stderr.contains("unknown option"), "{stderr}");
    assert!(stderr.contains("usage:"), "{stderr}");
}

#[test]
fn missing_option_value_is_a_usage_error() {
    // A trailing --call used to keep the default entrypoint silently.
    let (code, _, stderr) = modhost(&["host", "/nonexistent", "--call"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("--call requires a value"), "{stderr}");

    // A flag where a value belongs is not a value either.
    let (code, _, stderr) = modhost(&["host", "/nonexistent", "--call", "--repeat"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("--call requires a value"), "{stderr}");
}

#[test]
fn malformed_repeat_is_a_usage_error() {
    // parse().unwrap_or(1) used to fall back to 1 silently.
    let (code, _, stderr) = modhost(&["host", "/nonexistent", "--repeat", "lots"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("--repeat expects"), "{stderr}");
}

#[test]
fn non_integer_args_is_a_usage_error() {
    // Non-integer tokens used to be dropped silently, shifting every later
    // positional argument the module actually received.
    let (code, _, stderr) = modhost(&["host", "/nonexistent", "--args", "1", "oops"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("--args expects integers"), "{stderr}");
}

#[test]
fn authorize_requires_exactly_three_positionals() {
    let (code, _, stderr) = modhost(&["authorize", "only-one.toml"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage: modhost authorize"), "{stderr}");
}

// --- authorize decisions --------------------------------------------------

#[test]
fn authorize_human_allowed() {
    let m = demo_manifest();
    let (code, stdout, stderr) = modhost(&[
        "authorize",
        m.to_str().unwrap(),
        "demo_hosted",
        "uk_version",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.starts_with("ALLOW: module 'demo_hosted' may Call 'uk_version'"));
}

#[test]
fn authorize_json_allowed_is_one_decision_object() {
    let m = demo_manifest();
    let (code, stdout, _) = modhost(&[
        "authorize",
        m.to_str().unwrap(),
        "demo_hosted",
        "uk_version",
        "--json",
    ]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one JSON object");
    assert_eq!(v["event"], "decision");
    assert_eq!(v["allowed"], true);
    assert_eq!(v["principal"], "demo_hosted");
    assert_eq!(v["symbol"], "uk_version");
}

#[test]
fn authorize_denied_is_exit_1_with_reason() {
    let m = demo_manifest();
    let manifest = m.to_str().unwrap();

    let (code, stdout, stderr) = modhost(&["authorize", manifest, "demo_hosted", "uk_not_granted"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "deny is not an ALLOW line");
    assert!(stderr.contains("UK-4001 CallDenied"), "{stderr}");

    let (code, stdout, _) = modhost(&[
        "authorize",
        manifest,
        "demo_hosted",
        "uk_not_granted",
        "--json",
    ]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one JSON object");
    assert_eq!(v["event"], "decision");
    assert_eq!(v["allowed"], false);
    assert!(v["reason"].as_str().is_some(), "deny carries a reason");
}

#[test]
fn authorize_missing_manifest_is_usage_error_and_json_error_event() {
    let (code, _, stderr) = modhost(&["authorize", "/nonexistent/manifest.toml", "m", "uk_x"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("cannot read manifest"), "{stderr}");

    let (code, stdout, _) = modhost(&[
        "authorize",
        "/nonexistent/manifest.toml",
        "m",
        "uk_x",
        "--json",
    ]);
    assert_eq!(code, 2);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one JSON object");
    assert_eq!(v["event"], "error");
    assert_eq!(v["kind"], "manifest_unreadable");
}

// --- host: human transcript and --json events -----------------------------

#[test]
fn host_human_transcript_unchanged() {
    let dir = temp_module("human");
    let d = dir.to_str().unwrap();
    let (code, stdout, stderr) = modhost(&["host", d, "--call", "run", "--repeat", "3"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.contains("modhost: loaded 'human' v0.1.0 (1 functions, entry='run')"),
        "{stdout}"
    );
    assert_eq!(stdout.matches("run: 42").count(), 3, "{stdout}");
    assert!(stdout.contains("--- call 1/3 ---"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn host_json_emits_loaded_and_call_events_per_line() {
    let dir = temp_module("json");
    let d = dir.to_str().unwrap();
    let (code, stdout, stderr) = modhost(&["host", d, "--call", "run", "--repeat", "2", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");

    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("line {l:?}: {e}")))
        .collect();
    assert!(events.iter().all(|e| e["event"].is_string()), "{stdout}");

    let loaded = &events[0];
    assert_eq!(loaded["event"], "loaded");
    assert_eq!(loaded["module"], "json");
    assert_eq!(loaded["functions"], 1);
    assert_eq!(loaded["entry"], "run");

    let calls: Vec<&serde_json::Value> = events.iter().filter(|e| e["event"] == "call").collect();
    assert_eq!(calls.len(), 2, "{stdout}");
    assert_eq!(calls[0]["result"], "42");
    assert_eq!(calls[0]["iteration"], 1);
    assert_eq!(calls[1]["iteration"], 2);
    assert!(
        !stdout.contains("--- call"),
        "no human headers under --json"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn host_load_failure_is_error_event_json_and_exit_1() {
    let (code, _, stderr) = modhost(&["host", "/nonexistent/module-dir", "--call", "run"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("load failed"), "{stderr}");

    let (code, stdout, _) =
        modhost(&["host", "/nonexistent/module-dir", "--call", "run", "--json"]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one JSON object");
    assert_eq!(v["event"], "error");
    assert_eq!(v["kind"], "load_failed");
}

#[test]
fn host_unknown_entrypoint_is_call_failed_event() {
    let dir = temp_module("badep");
    let d = dir.to_str().unwrap();
    let (code, stdout, stderr) = modhost(&["host", d, "--call", "no_such_ep", "--json"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("call failed"), "{stderr}");
    // The stream carries the earlier `loaded` event first; the last object
    // is the error that decided the exit code.
    let last = stdout.lines().next_back().expect("at least one event");
    let v: serde_json::Value = serde_json::from_str(last).unwrap_or_else(|e| panic!("{last}: {e}"));
    assert_eq!(v["event"], "error");
    assert_eq!(v["kind"], "call_failed");
    assert_eq!(v["entrypoint"], "no_such_ep");
    let _ = std::fs::remove_dir_all(&dir);
}
