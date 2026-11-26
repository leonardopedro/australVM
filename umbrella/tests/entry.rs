//! U04 acceptance: the umbrella's entry invocation goes through the bridge's
//! strict path (`compile_entry_i64`), so a module whose entry does not match
//! the caller's `fn() -> i64` contract is refused before any invocation.
//!
//! The front-end is a script that writes a hand-built CPS buffer, which makes
//! the declared signature the test's variable. A `RESULT` line on stdout is
//! the invocation marker: it can only appear after the entry actually ran.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("umbrella-entry-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    support::cleanup_on_exit("umbrella-entry");
    dir
}

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    let mut perm = std::fs::metadata(path).expect("meta").permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(path, perm).expect("chmod");
}

/// A one-function CPS v1 module. `ret` is the declared return-type byte
/// (0x01 Int64 in the admitted case); the body returns the constant 7.
fn cps(entry: &str, params: u32, ret: u8) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&0x43505331u32.to_le_bytes()); // magic v1
    buf.extend_from_slice(&1u32.to_le_bytes()); // one function
    buf.extend_from_slice(&(entry.len() as u32).to_le_bytes());
    buf.extend_from_slice(entry.as_bytes());
    buf.extend_from_slice(&params.to_le_bytes());
    for i in 0..params {
        let p = format!("p{i}");
        buf.extend_from_slice(&(p.len() as u32).to_le_bytes());
        buf.extend_from_slice(p.as_bytes());
    }
    buf.push(ret);
    buf.extend_from_slice(&10u32.to_le_bytes()); // body length
    buf.push(0x07); // Return
    buf.push(0x01); // Int_lit
    buf.extend_from_slice(&7i64.to_le_bytes());
    buf
}

/// POSIX `printf` octal escapes keep the script free of a base64 dependency.
fn octal(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:03o}")).collect()
}

/// A front-end that writes `buffer` to the path in `--emit-cps=…`, writes the
/// empty `--emit-adapters` document (a module with no scalar crossing is a real
/// state, not a missing file) and exits 0.
fn fake_front_end(dir: &Path, buffer: &[u8]) -> PathBuf {
    let path = dir.join("fake-austral");
    executable(
        &path,
        &format!(
            "#!/bin/sh\n\
             for a in \"$@\"; do\n\
               case \"$a\" in\n\
                 --emit-cps=*) out=\"${{a#--emit-cps=}}\" ;;\n\
                 --emit-adapters=*) adapters=\"${{a#--emit-adapters=}}\" ;;\n\
               esac\n\
             done\n\
             [ -n \"$out\" ] && [ -n \"$adapters\" ] || exit 9\n\
             printf '%b' '{}' > \"$out\"\n\
             : > \"$adapters\"\n\
             printf 'CPS JIT: module Entry\\nCPS JIT: Generated 1 functions ({} bytes)\\nCPS JIT: export\\tcompute\\t\\tInt64\\nCPS JIT: Emitted CPS binary to %s\\n' \"$out\" >&2\n\
             exit 0\n",
            octal(buffer),
            buffer.len()
        ),
    );
    path
}

fn project_with_provider(dir: &Path, so: &Path) -> PathBuf {
    project_with_providers(dir, &[so])
}

fn project_with_providers(dir: &Path, providers: &[&Path]) -> PathBuf {
    project_document(dir, providers, None, None)
}

fn project_document(
    dir: &Path,
    providers: &[&Path],
    signatures: Option<serde_json::Value>,
    descriptor_sha256: Option<&str>,
) -> PathBuf {
    let path = dir.join("project.json");
    let so = providers[0];
    // D09: a load authorization names the exact library bytes it approves, so the
    // fixture pins the digest of the artifact it just built.
    let library_sha256 = umbrella::trust::sha256_file(so).expect("hash the fixture library");
    let mut trusted = serde_json::json!({
        "lock_sha256": "0".repeat(64),
        "features": [],
        "library_sha256": library_sha256,
        "exports": ["stageprobe_version"]
    });
    if let Some(signatures) = signatures {
        trusted["signatures"] = signatures;
    }
    if let Some(digest) = descriptor_sha256 {
        trusted["descriptor_sha256"] = serde_json::Value::String(digest.to_string());
    }
    std::fs::write(
        &path,
        serde_json::json!({
            "schema": 1,
            "profile": "dev",
            "module": {"name": "entry", "sources": ["unused.aum"]},
            "build": {"allowed": false},
            "load": {"allowed": true, "providers": providers},
            "grants": [{"symbol": "rs_probe"}],
            "trusted": trusted
        })
        .to_string(),
    )
    .expect("write project");
    path
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

/// Run the umbrella against `buffer`, with `--entry <entry>` and `--expect 7`.
fn run(dir: &Path, buffer: &[u8], entry: &str) -> Output {
    let so = build_stageprobe(dir);
    let project = project_with_provider(dir, &so);
    run_with_project(dir, buffer, entry, &project, false)
}

fn run_with_project(dir: &Path, buffer: &[u8], entry: &str, project: &Path, json: bool) -> Output {
    let austral = fake_front_end(dir, buffer);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_umbrella"));
    if json {
        cmd.arg("--json");
    }
    cmd.arg("--project")
        .arg(project)
        .arg("--austral")
        .arg(&austral)
        .arg("--entry")
        .arg(entry)
        .arg("--expect")
        .arg("7")
        .arg("--")
        .arg(dir.join("M.aum"))
        .output()
        .expect("run umbrella")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The provider fixture's initializer appends `load` to `stages.log` when it is
/// actually mapped, so the log tells "refused before mapping" apart from
/// "mapped and then refused".
fn stages(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("stages.log")).unwrap_or_default()
}

#[test]
fn two_provider_libraries_are_refused_before_mapping() {
    let dir = scratch("two-providers");
    let so = build_stageprobe(&dir);
    let project = project_with_providers(&dir, &[&so, &so]);
    let austral = fake_front_end(&dir, &cps("compute", 0, 0x01));
    let out = Command::new(env!("CARGO_BIN_EXE_umbrella"))
        .arg("--project")
        .arg(&project)
        .arg("--austral")
        .arg(&austral)
        .arg("--entry")
        .arg("compute")
        .arg("--expect")
        .arg("7")
        .arg("--")
        .arg(dir.join("M.aum"))
        .output()
        .expect("run umbrella");
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("declares 2 provider libraries"),
        "{}",
        stderr(&out)
    );
    assert!(!stdout(&out).contains("RESULT"), "{}", stdout(&out));
}

#[test]
fn a_matching_entry_runs_and_reports_the_result() {
    let dir = scratch("ok");
    let out = run(&dir, &cps("compute", 0, 0x01), "compute");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out).trim(),
        "RESULT 7",
        "the entry must actually run"
    );
}

#[test]
fn a_wrong_arity_entry_is_refused_before_invocation() {
    let dir = scratch("arity");
    let out = run(&dir, &cps("compute", 1, 0x01), "compute");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("declares 1 parameter") && stderr(&out).contains("passes 0"),
        "{}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains("RESULT"),
        "a refused entry must never be invoked: {}",
        stdout(&out)
    );
}

#[test]
fn a_non_i64_return_is_refused_before_invocation() {
    let dir = scratch("float");
    let out = run(&dir, &cps("compute", 0, 0x05), "compute");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("Float64"), "{}", stderr(&out));
    assert!(!stdout(&out).contains("RESULT"), "{}", stdout(&out));
}

#[test]
fn a_missing_entry_is_refused_by_name() {
    let dir = scratch("missing");
    let out = run(&dir, &cps("compute", 0, 0x01), "other");
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("'other'") && err.contains("compute"), "{err}");
    assert!(!stdout(&out).contains("RESULT"), "{}", stdout(&out));
}

/// U10: a pinned typed signature is admitted exactly. `compute` takes no
/// parameters and returns Int64, so a pin with a parameter must refuse before
/// the entry can execute.
#[test]
fn a_changed_typed_signature_is_refused_before_invocation() {
    let dir = scratch("signature-pin");
    let so = build_stageprobe(&dir);
    let project = project_document(
        &dir,
        &[&so],
        Some(serde_json::json!([
            {"name": "compute", "params": ["Int64"], "returns": "Int64"}
        ])),
        None,
    );
    let out = run_with_project(&dir, &cps("compute", 0, 0x01), "compute", &project, false);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("trusted signatures refused"), "{err}");
    assert!(err.contains("`compute` pinned"), "{err}");
    assert!(!stdout(&out).contains("RESULT"), "{}", stdout(&out));
    // The pin is checked before admission, so the provider's initializer never
    // ran; before this ordering the marker read `load`.
    assert!(
        !stages(&dir).contains("load"),
        "a refused contract must not map a provider: {:?}",
        stages(&dir)
    );
}

#[test]
fn a_changed_descriptor_digest_is_refused_before_native_work() {
    let dir = scratch("descriptor-pin");
    let so = build_stageprobe(&dir);
    let project = project_document(&dir, &[&so], None, Some(&"0".repeat(64)));
    let out = run_with_project(&dir, &cps("compute", 0, 0x01), "compute", &project, false);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains("trusted descriptor refused"), "{err}");
    assert!(!stdout(&out).contains("RESULT"), "{}", stdout(&out));
    assert!(
        !stages(&dir).contains("load"),
        "a refused descriptor must not map a provider: {:?}",
        stages(&dir)
    );
}

/// U10: a successful artifact can produce the pins its project document needs,
/// so the trust record is generated from what actually ran instead of copied by
/// hand. Every pin in the file must match the artifact and descriptor the run
/// admitted.
#[test]
fn a_successful_artifact_can_emit_its_trust_record() {
    let dir = scratch("trust-record");
    let so = build_stageprobe(&dir);
    let project = project_with_provider(&dir, &so);
    let record_path = dir.join("pins.json");
    let austral = fake_front_end(&dir, &cps("compute", 0, 0x01));
    let out = Command::new(env!("CARGO_BIN_EXE_umbrella"))
        .arg("--project")
        .arg(&project)
        .arg("--austral")
        .arg(&austral)
        .arg("--emit-trust-record")
        .arg(&record_path)
        .arg("--entry")
        .arg("compute")
        .arg("--expect")
        .arg("7")
        .arg("--")
        .arg(dir.join("M.aum"))
        .output()
        .expect("run umbrella");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let record: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&record_path).expect("the record was written"),
    )
    .expect("the record is JSON");
    assert_eq!(record["signatures"][0]["name"], "compute");
    assert_eq!(record["signatures"][0]["params"], serde_json::json!([]));
    assert_eq!(record["signatures"][0]["returns"], "Int64");
    assert_eq!(
        record["descriptor_sha256"],
        umbrella::trust::sha256_bytes(b""),
        "the empty adapter document is pinned by its digest"
    );
}

/// U10: the typed artifact event names the exports and the descriptor blocks
/// (with the digest a project can pin), not just their count.
#[test]
fn the_artifact_event_carries_exports_and_descriptor_data() {
    let dir = scratch("artifact-event");
    let so = build_stageprobe(&dir);
    let project = project_with_provider(&dir, &so);
    let out = run_with_project(&dir, &cps("compute", 0, 0x01), "compute", &project, true);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let event = stdout(&out)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["event"] == "artifact")
        .expect("artifact event");
    assert_eq!(event["exports"][0]["name"], "compute");
    assert_eq!(event["exports"][0]["returns"], "Int64");
    assert_eq!(event["adapters"], serde_json::json!([]));
    assert_eq!(
        event["adapters_sha256"],
        umbrella::trust::sha256_bytes(b""),
        "the empty descriptor document is still identified by its digest"
    );
}
