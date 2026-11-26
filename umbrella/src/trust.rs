//! Trusted artifact identity (D09 step 1).
//!
//! Building an arbitrary crate is executing its build scripts, and dlopening an
//! arbitrary cdylib is running its initializer. Both are therefore only done for
//! an artifact whose identity was pinned *in advance* in the project document:
//!
//! * `trusted.lock_sha256`   — the crate's `Cargo.lock` the build must be from;
//! * `trusted.features`      — the exact feature set to build with (no more);
//! * `trusted.library_sha256`— the built library that may be mapped;
//! * `trusted.exports`       — the ABI exports the library must define.
//!
//! The split matters: the lock and the features are checked *before* cargo runs
//! (a drifted lock is refused with nothing executed), while the library digest
//! and the export list are checked *after* the build and *before* `dlopen` (the
//! bytes on disk are the ones that were pinned, not merely the ones that were
//! asked for).
//!
//! What this is not: a signature, or a claim about what the crate's code does
//! once called. A pinned digest says "this is the artifact the project approved",
//! which is the decision D09 asks for; it says nothing about the safety of the
//! code inside it, and it is not a substitute for the separate untrusted-native
//! tier.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::frontend::Export;
use crate::project::Project;

/// SHA-256 of bytes, lowercase hex.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// SHA-256 of a file, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The crate's `Cargo.lock` must be the pinned one. Run before `cargo build`:
/// a lock that drifted means the build would compile a different dependency
/// graph than the one that was approved.
pub fn verify_lock(crate_dir: &Path, expected: &str) -> Result<(), String> {
    let lock = crate_dir.join("Cargo.lock");
    let actual = sha256_file(&lock)
        .map_err(|e| format!("trusted build refused: {e} (the pinned lock cannot be checked)"))?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "trusted build refused: {} has sha256 {actual}, the project pins {expected}. \
             Refusing to run cargo against a dependency graph that is not the approved one.",
            lock.display()
        ));
    }
    Ok(())
}

/// The feature set the build will use must be exactly the pinned one. A feature
/// the caller adds at the command line would change what is compiled without
/// changing the pin, so the comparison is on the set, not a subset.
pub fn verify_features(expected: &[String], requested: &[String]) -> Result<(), String> {
    let mut want: Vec<&str> = expected.iter().map(String::as_str).collect();
    let mut got: Vec<&str> = requested.iter().map(String::as_str).collect();
    want.sort_unstable();
    got.sort_unstable();
    if want != got {
        return Err(format!(
            "trusted build refused: features {:?} were requested but the project pins {:?}",
            got, want
        ));
    }
    Ok(())
}

/// The library that may be mapped must be the pinned artifact.
pub fn verify_library(library: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_file(library)
        .map_err(|e| format!("trusted load refused: {e} (the pinned library cannot be checked)"))?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "trusted load refused: {} has sha256 {actual}, the project pins {expected}",
            library.display()
        ));
    }
    Ok(())
}

/// The ABI exports the library defines, read from its dynamic symbol table.
///
/// `nm` is used instead of dlopening the library to inspect it: mapping it is
/// the very step being deferred, and an initializer runs on load. A host without
/// `nm` gets a refusal, not a pass — a trust gate that cannot see the symbols has
/// not checked them.
pub fn exports(library: &Path) -> Result<Vec<String>, String> {
    let out = std::process::Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(library)
        .output();
    let out = match out {
        Ok(out) => out,
        Err(e) => {
            return Err(format!(
                "trusted load refused: cannot list the exports of {} ({e}); \
                 a trust gate must not pass on an unchecked artifact",
                library.display()
            ))
        }
    };
    if !out.status.success() {
        return Err(format!(
            "trusted load refused: nm failed on {} ({})",
            library.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut names: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `<address> <type> <name>` — the last field is the symbol.
        if let Some(name) = line.split_whitespace().nth(2) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// Check that every pinned export is defined by the library. Missing an export
/// is a refusal: the caller's ABI expectations are not met by this artifact.
pub fn verify_exports(library: &Path, expected: &[String]) -> Result<(), String> {
    let defined = exports(library)?;
    let missing: Vec<&String> = expected
        .iter()
        .filter(|name| !defined.iter().any(|d| d == *name))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "trusted load refused: {} does not export {}",
            library.display(),
            missing
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

/// Every pre-build check for a project that authorizes a build. A project that
/// authorizes neither build nor load has nothing to pin here.
pub fn verify_before_build(project: &Project, crate_dir: &Path) -> Result<(), String> {
    match &project.trusted {
        Some(trusted) => {
            verify_lock(crate_dir, &trusted.lock_sha256)?;
            verify_features(&trusted.features, &project.requested_features)
        }
        None => Err(
            "trusted build refused: the project authorizes building but pins no `trusted` \
             artifact identity (lock_sha256, features)"
                .to_string(),
        ),
    }
}

/// Every post-build, pre-load check. The library digest is checked first: if the
/// bytes are not the pinned ones there is nothing to say about their exports.
pub fn verify_before_load(project: &Project, library: &Path) -> Result<(), String> {
    match &project.trusted {
        Some(trusted) => {
            verify_library(library, &trusted.library_sha256)?;
            verify_exports(library, &trusted.exports)
        }
        None => Err(
            "trusted load refused: the project authorizes loading but pins no `trusted` \
             artifact identity (library_sha256, exports)"
                .to_string(),
        ),
    }
}

/// U10: the compile-only artifact's emitted adapter document must be the pinned
/// contract. This runs after the front-end and *before* any wrapper is built or
/// loaded, so a changed descriptor cannot reach native code. An absent pin is
/// not a refusal: a project that does not pin a descriptor has not made that
/// claim, while a pin that differs is always a refusal.
pub fn verify_descriptor(project: &Project, document: &[u8]) -> Result<(), String> {
    let Some(expected) = project
        .trusted
        .as_ref()
        .and_then(|trusted| trusted.descriptor_sha256.as_deref())
    else {
        return Ok(());
    };
    let actual = sha256_bytes(document);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "trusted descriptor refused: the compiler emitted sha256 {actual}, the project \
             pins {expected}. Refusing to build native adapters from a contract that is \
             not the approved one."
        ));
    }
    Ok(())
}

/// U10: admit exactly the typed signatures the project pinned. A pin is an exact
/// set comparison: a missing, extra, renamed or retyped export is a different
/// contract, even when the export name alone still matches.
pub fn verify_signatures(project: &Project, actual: &[Export]) -> Result<(), String> {
    let Some(expected) = project
        .trusted
        .as_ref()
        .filter(|trusted| !trusted.signatures.is_empty())
        .map(|trusted| &trusted.signatures)
    else {
        return Ok(());
    };

    let expected: BTreeMap<&str, (&[String], &str)> = expected
        .iter()
        .map(|s| (s.name.as_str(), (s.params.as_slice(), s.returns.as_str())))
        .collect();
    let actual: BTreeMap<&str, (&[String], &str)> = actual
        .iter()
        .map(|s| (s.name.as_str(), (s.params.as_slice(), s.returns.as_str())))
        .collect();

    let missing: Vec<&str> = expected
        .keys()
        .filter(|name| !actual.contains_key(*name))
        .copied()
        .collect();
    let unexpected: Vec<&str> = actual
        .keys()
        .filter(|name| !expected.contains_key(*name))
        .copied()
        .collect();
    let changed: Vec<&str> = expected
        .iter()
        .filter_map(|(name, want)| actual.get(name).filter(|got| *got != want).map(|_| *name))
        .collect();

    if missing.is_empty() && unexpected.is_empty() && changed.is_empty() {
        return Ok(());
    }

    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!("missing {missing:?}"));
    }
    if !unexpected.is_empty() {
        parts.push(format!("unexpected {unexpected:?}"));
    }
    for name in changed {
        let (want_params, want_returns) = expected[name];
        let (got_params, got_returns) = actual[name];
        parts.push(format!(
            "`{name}` pinned {want_params:?} -> {want_returns}, artifact has \
             {got_params:?} -> {got_returns}"
        ));
    }
    Err(format!(
        "trusted signatures refused: the artifact's typed exports do not match the \
         project pin ({})",
        parts.join("; ")
    ))
}

/// U10: the pins an admitted artifact implies, as the fragment a project
/// document's `trusted` block carries. The caller produces this only after the
/// artifact was verified (or had nothing pinned), so a generated record always
/// describes the bytes that ran, never a contract the run refused.
pub fn trust_record(exports: &[Export], descriptor_document: &[u8]) -> serde_json::Value {
    let mut sorted: Vec<&Export> = exports.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let signatures: Vec<serde_json::Value> = sorted
        .iter()
        .map(|export| {
            serde_json::json!({
                "name": export.name,
                "params": export.params,
                "returns": export.returns,
            })
        })
        .collect();
    serde_json::json!({
        "signatures": signatures,
        "descriptor_sha256": sha256_bytes(descriptor_document),
    })
}

/// Both U10 artifact pins, before descriptor-driven native work.
pub fn verify_artifact(
    project: &Project,
    exports: &[Export],
    descriptor_document: &[u8],
) -> Result<(), String> {
    verify_signatures(project, exports)?;
    verify_descriptor(project, descriptor_document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project;

    fn tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("umbrella_trust_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn project_with(trusted: &str) -> Project {
        let doc = format!(
            r#"{{"schema":1,"profile":"trusted-native",
                 "module":{{"name":"demo","sources":["Demo.aum"]}},
                 "build":{{"allowed":true,"crate":"crate"}},
                 "load":{{"allowed":true,"providers":["libdemo.so"]}},
                 "grants":[{{"symbol":"rs_demo_checksum"}}],
                 "trusted":{trusted}}}"#
        );
        let mut parsed = project::parse(Path::new("example.json"), &doc).expect("valid project");
        // What the caller asked cargo for: the test says so explicitly, since the
        // document only records the pin.
        parsed.requested_features = parsed
            .trusted
            .as_ref()
            .map(|t| t.features.clone())
            .unwrap_or_default();
        parsed
    }

    #[test]
    fn a_drifted_lock_is_refused_before_anything_runs() {
        let dir = tempdir("lock");
        std::fs::write(dir.join("Cargo.lock"), "version = 3\n").unwrap();
        let pinned = sha256_file(&dir.join("Cargo.lock")).unwrap();
        let project = project_with(&format!(
            r#"{{"lock_sha256":"{pinned}","features":[],"library_sha256":"{}","exports":["rs_demo"]}}"#,
            "0".repeat(64)
        ));
        verify_before_build(&project, &dir).expect("the pinned lock");

        // One byte of drift is a different dependency graph.
        std::fs::write(dir.join("Cargo.lock"), "version = 4\n").unwrap();
        let err = verify_before_build(&project, &dir).expect_err("a drifted lock must be refused");
        assert!(err.contains("trusted build refused"), "{err}");
        assert!(err.contains(&pinned), "the refusal names the pin: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_added_feature_is_refused_even_with_a_pinned_lock() {
        let dir = tempdir("features");
        std::fs::write(dir.join("Cargo.lock"), "version = 3\n").unwrap();
        let pinned = sha256_file(&dir.join("Cargo.lock")).unwrap();
        let mut project = project_with(&format!(
            r#"{{"lock_sha256":"{pinned}","features":["fast"],"library_sha256":"{}","exports":["rs_demo"]}}"#,
            "0".repeat(64)
        ));
        verify_before_build(&project, &dir).expect("the pinned features");
        project.requested_features = vec!["fast".into(), "extra".into()];
        let err =
            verify_before_build(&project, &dir).expect_err("an added feature must be refused");
        assert!(err.contains("features"), "{err}");
        assert!(err.contains("extra"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_without_a_pin_is_refused_rather_than_trusted() {
        let dir = tempdir("nopin");
        std::fs::write(dir.join("Cargo.lock"), "version = 3\n").unwrap();
        // The document itself refuses the combination: an authorization to build
        // or load with nothing pinned is not a deployment anyone approved.
        let doc = r#"{"schema":1,"profile":"trusted-native",
                      "module":{"name":"demo","sources":["Demo.aum"]},
                      "build":{"allowed":true,"crate":"crate"},
                      "load":{"allowed":true,"providers":["libdemo.so"]},
                      "grants":[{"symbol":"rs_demo_checksum"}]}"#;
        let err = project::parse(Path::new("example.json"), doc).expect_err("no pin is refused");
        assert!(err.contains("`trusted` is absent"), "{err}");

        // And the gate refuses one that reached it anyway (a caller can build a
        // Project directly, so the check does not rely on the parser alone).
        let unpinned = Project {
            schema: project::SCHEMA_VERSION,
            profile: project::Profile::TrustedNative,
            module: project::Module {
                name: "demo".to_string(),
                sources: vec![Path::new("Demo.aum").to_path_buf()],
            },
            identity: None,
            build: project::BuildPermission {
                allowed: true,
                krate: Some(Path::new("crate").to_path_buf()),
            },
            load: project::LoadPermission {
                allowed: true,
                providers: vec![Path::new("libdemo.so").to_path_buf()],
            },
            grants: Vec::new(),
            trusted: None,
            capabilities: None,
            requested_features: Vec::new(),
        };
        let err = verify_before_build(&unpinned, &dir).expect_err("no pin must be refused");
        assert!(err.contains("pins no `trusted` artifact identity"), "{err}");
        let err = verify_before_load(&unpinned, &dir.join("libdemo.so"))
            .expect_err("no pin must be refused at load too");
        assert!(err.contains("pins no `trusted` artifact identity"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_library_that_is_not_the_pinned_one_is_refused_and_its_exports_never_consulted() {
        let dir = tempdir("lib");
        let lib = dir.join("libdemo.so");
        std::fs::write(&lib, b"not an elf").unwrap();
        let project = project_with(&format!(
            r#"{{"lock_sha256":"{}","features":[],"library_sha256":"{}","exports":["rs_demo"]}}"#,
            "0".repeat(64),
            "1".repeat(64)
        ));
        let err = verify_before_load(&project, &lib).expect_err("a different artifact is refused");
        assert!(err.contains("trusted load refused"), "{err}");
        assert!(err.contains("sha256"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_typed_signature_is_refused() {
        let project = project_with(&format!(
            r#"{{"lock_sha256":"{}","features":[],"library_sha256":"{}","exports":["rs_demo"], "signatures":[{{"name":"main","params":[],"returns":"Int64"}}]}}"#,
            "0".repeat(64),
            "0".repeat(64)
        ));
        let admitted = Export {
            name: "main".to_string(),
            params: Vec::new(),
            returns: "Int64".to_string(),
        };
        verify_signatures(&project, std::slice::from_ref(&admitted)).expect("the pinned signature");

        let retyped = Export {
            returns: "Int32".to_string(),
            ..admitted.clone()
        };
        let err = verify_signatures(&project, &[retyped]).expect_err("a retyped export");
        assert!(err.contains("trusted signatures refused"), "{err}");
        assert!(err.contains("`main` pinned"), "{err}");
        assert!(err.contains("Int32"), "{err}");

        let extra_param = Export {
            params: vec!["Int64".to_string()],
            ..admitted.clone()
        };
        let err = verify_signatures(&project, &[extra_param]).expect_err("a changed arity");
        assert!(err.contains("`main` pinned"), "{err}");

        let renamed = Export {
            name: "entry".to_string(),
            ..admitted
        };
        let err = verify_signatures(&project, &[renamed]).expect_err("a renamed export");
        assert!(err.contains("missing [\"main\"]"), "{err}");
        assert!(err.contains("unexpected [\"entry\"]"), "{err}");
    }

    #[test]
    fn a_changed_adapter_descriptor_is_refused_before_it_is_used() {
        let document =
            "schema 1\nabi scalar 1\nsymbol f__adapter\ntarget f\nparams \nreturns Int64\n";
        let digest = sha256_bytes(document.as_bytes());
        let project = project_with(&format!(
            r#"{{"lock_sha256":"{}","features":[],"library_sha256":"{}","exports":["rs_demo"], "descriptor_sha256":"{digest}"}}"#,
            "0".repeat(64),
            "0".repeat(64)
        ));
        verify_descriptor(&project, document.as_bytes()).expect("the pinned document");
        let err = verify_descriptor(&project, document.replace("Int64", "Int32").as_bytes())
            .expect_err("a changed contract");
        assert!(err.contains("trusted descriptor refused"), "{err}");
        assert!(err.contains(&digest), "the refusal names the pin: {err}");
    }

    #[test]
    fn the_generated_trust_record_carries_the_admitted_pins() {
        let document =
            "schema 1\nabi scalar 1\nsymbol f__adapter\ntarget f\nparams \nreturns Int64\n";
        let exports = vec![
            Export {
                name: "zeta".to_string(),
                params: vec!["Int64".to_string()],
                returns: "Nat64".to_string(),
            },
            Export {
                name: "alpha".to_string(),
                params: Vec::new(),
                returns: "Int64".to_string(),
            },
        ];
        let record = trust_record(&exports, document.as_bytes());
        // The record is a `trusted` fragment: exactly the two fields the parser
        // accepts for these pins, with signatures ordered by name so a
        // regeneration is byte-identical.
        assert_eq!(record["signatures"][0]["name"], "alpha");
        assert_eq!(record["signatures"][1]["name"], "zeta");
        assert_eq!(
            record["signatures"][1]["params"],
            serde_json::json!(["Int64"])
        );
        assert_eq!(record["signatures"][1]["returns"], "Nat64");
        assert_eq!(
            record["descriptor_sha256"],
            sha256_bytes(document.as_bytes())
        );

        // And the fragment parses back as the pin it claims to be, so a
        // generated record cannot be one the project document refuses.
        let text = format!(
            r#"{{"schema":1,"profile":"trusted-native",
                 "module":{{"name":"demo","sources":["Demo.aum"]}},
                 "build":{{"allowed":false}},
                 "load":{{"allowed":false}},
                 "trusted":{{"lock_sha256":"{}","library_sha256":"{}","exports":["rs_demo"],
                   "signatures":{},"descriptor_sha256":{}}}}}"#,
            "0".repeat(64),
            "0".repeat(64),
            record["signatures"],
            record["descriptor_sha256"]
        );
        project::parse(Path::new("example.json"), &text).expect("the generated pins parse");
    }

    #[test]
    fn an_unreadable_library_is_a_refusal_not_a_pass() {
        let dir = tempdir("missing");
        let project = project_with(&format!(
            r#"{{"lock_sha256":"{}","features":[],"library_sha256":"{}","exports":["rs_demo"]}}"#,
            "0".repeat(64),
            "0".repeat(64)
        ));
        let err = verify_before_load(&project, &dir.join("absent.so"))
            .expect_err("a missing artifact cannot be verified");
        assert!(err.contains("cannot be checked"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
