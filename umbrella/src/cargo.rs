//! U05: select the *exact* Cargo artifact the project asked for.
//!
//! The old selection took the first shared-library filename anywhere in
//! cargo's JSON stream. That is not an identity: a proc-macro is also a
//! `.so`, a dependency can emit a cdylib of its own, and two `cdylib`
//! targets in one workspace are both plausible. The fix is to state what
//! was wanted — package and target name, taken from `cargo metadata` for the
//! crate directory — and then refuse when the stream offers zero or several
//! matches instead of silently picking one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The Cargo profile umbrella builds with. `Dev` is cargo's default profile,
/// `Release` is `--release`; the prototype hardcoded release, so it stays the
/// default when the flag is absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    Dev,
    #[default]
    Release,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Dev => "dev",
            Profile::Release => "release",
        }
    }

    pub fn parse(text: &str) -> Result<Profile, String> {
        match text {
            "dev" | "debug" => Ok(Profile::Dev),
            "release" => Ok(Profile::Release),
            other => Err(format!(
                "unknown profile `{other}`; the build profiles are `dev` and `release`"
            )),
        }
    }
}

/// Everything the build step is allowed to vary, stated explicitly rather
/// than read from the environment or hardcoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    pub crate_dir: PathBuf,
    /// `Some` selects one workspace member; `None` builds the crate directory's
    /// default members.
    pub package: Option<String>,
    pub target_dir: Option<PathBuf>,
    pub profile: Profile,
    pub features: Vec<String>,
    pub locked: bool,
    pub offline: bool,
}

impl BuildRequest {
    pub fn new(crate_dir: PathBuf) -> Self {
        BuildRequest {
            crate_dir,
            package: None,
            target_dir: None,
            profile: Profile::default(),
            features: Vec::new(),
            locked: false,
            offline: false,
        }
    }
}

/// The exact `cargo build` argv (after `cargo`) for `request`.
///
/// Arguments are kept as separate `OsString`s end to end: a feature list that
/// contains a space stays one argument, and no shell ever sees them.
pub fn build_args(request: &BuildRequest) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["build".into(), "--message-format=json".into()];
    if request.profile == Profile::Release {
        args.push("--release".into());
    }
    if let Some(dir) = &request.target_dir {
        args.push("--target-dir".into());
        args.push(dir.as_os_str().to_owned());
    }
    if let Some(package) = &request.package {
        args.push("--package".into());
        args.push(package.into());
    }
    for feature in &request.features {
        args.push("--features".into());
        args.push(feature.into());
    }
    if request.locked {
        args.push("--locked".into());
    }
    if request.offline {
        args.push("--offline".into());
    }
    args
}

/// Variables that change what `rustc` receives without appearing in umbrella's
/// argv. They are *recorded* so a build can be reproduced, and wrappers are
/// *refused*: a wrapper replaces the compiler, so the artifact no longer
/// corresponds to the toolchain the rest of the evidence names.
pub const BUILD_AFFECTING_VARS: &[&str] = &[
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_TARGET",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTFLAGS",
];

/// Returns the recorded `(name, value)` pairs for the build-affecting
/// variables that are set, or refuses a compiler wrapper.
pub fn build_environment<'a, I>(vars: I) -> Result<Vec<(String, String)>, String>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut recorded = Vec::new();
    for (name, value) in vars {
        if !BUILD_AFFECTING_VARS.contains(&name) {
            continue;
        }
        if name == "RUSTC_WRAPPER" || name == "RUSTC_WORKSPACE_WRAPPER" {
            return Err(format!(
                "{name} is set to `{value}`; a compiler wrapper changes what the loaded \
                 artifact was built by, so umbrella refuses to build through one"
            ));
        }
        recorded.push((name.to_string(), value.to_string()));
    }
    recorded.sort();
    Ok(recorded)
}

/// Cargo config files that would silently change the build: every
/// `.cargo/config{,.toml}` from the crate directory up to the filesystem
/// root, nearest first. Cargo merges them, so all of them are reported.
pub fn cargo_config_files(crate_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in crate_dir.ancestors() {
        for name in ["config.toml", "config"] {
            let path = dir.join(".cargo").join(name);
            if path.is_file() {
                found.push(path);
            }
        }
    }
    found
}

/// The package and target umbrella intends to load.
///
/// `package_id` is compared verbatim: cargo's id for a path dependency can be
/// `path+file:///dir#0.1.0`, with no package name in it at all, so extracting
/// a name from the id is not an identity. `package` is only for messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub package_id: String,
    pub package: String,
    pub target: String,
}

fn is_shared_library(path: &str) -> bool {
    path.ends_with(".so") || path.ends_with(".dylib") || path.ends_with(".dll")
}

/// Read the crate directory's own identity from `cargo metadata --no-deps`
/// output: exactly one package at that directory, and exactly one `cdylib`
/// target in it.
pub fn wanted_from_metadata(metadata: &str, package_dir: &Path) -> Result<Wanted, String> {
    wanted_from_metadata_for(metadata, package_dir, None)
}

/// Same, but `package` (from `--cargo-package`) selects the workspace member
/// by name instead of requiring exactly one package at the directory.
pub fn wanted_from_metadata_for(
    metadata: &str,
    package_dir: &Path,
    package: Option<&str>,
) -> Result<Wanted, String> {
    let v: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|e| format!("cargo metadata is not valid JSON: {e}"))?;
    let packages = v["packages"]
        .as_array()
        .ok_or_else(|| "cargo metadata carries no `packages` array".to_string())?;
    let mut found: Vec<&serde_json::Value> = Vec::new();
    for candidate in packages {
        if let Some(name) = package {
            if candidate["name"].as_str() == Some(name) {
                found.push(candidate);
            }
            continue;
        }
        let Some(manifest) = candidate["manifest_path"].as_str() else {
            continue;
        };
        if Path::new(manifest).parent() == Some(package_dir) {
            found.push(candidate);
        }
    }
    let package = match found.as_slice() {
        [] => {
            return Err(match package {
                Some(name) => format!(
                    "cargo metadata names no package `{name}` for {}",
                    package_dir.display()
                ),
                None => format!(
                    "cargo metadata names no package at {}",
                    package_dir.display()
                ),
            })
        }
        [one] => *one,
        many => {
            return Err(format!(
                "cargo metadata names {} packages ({}); pass one explicitly",
                many.len(),
                many.iter()
                    .filter_map(|p| p["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    };
    let package_name = package["name"]
        .as_str()
        .ok_or_else(|| "cargo metadata package has no name".to_string())?;
    let package_id = package["id"]
        .as_str()
        .ok_or_else(|| format!("cargo metadata package `{package_name}` has no id"))?;
    let targets = package["targets"]
        .as_array()
        .ok_or_else(|| format!("cargo metadata package `{package_name}` has no targets"))?;
    let cdylibs: Vec<&str> = targets
        .iter()
        .filter(|t| {
            t["kind"]
                .as_array()
                .map(|kinds| kinds.iter().any(|k| k == "cdylib"))
                .unwrap_or(false)
        })
        .filter_map(|t| t["name"].as_str())
        .collect();
    match cdylibs.as_slice() {
        [] => Err(format!(
            "cargo metadata package `{package_name}` declares no cdylib target; \
             umbrella can only load a shared library"
        )),
        [target] => Ok(Wanted {
            package_id: package_id.to_string(),
            package: package_name.to_string(),
            target: target.to_string(),
        }),
        many => Err(format!(
            "cargo metadata package `{package_name}` declares {} cdylib targets ({}); \
             which one umbrella should load is not stated",
            many.len(),
            many.join(", ")
        )),
    }
}

/// Select the shared-library artifact for `wanted`, or explain why the stream
/// does not identify exactly one. A dependency's cdylib, a proc-macro and a
/// second target in the same package are all *not* this artifact.
pub fn select_shared_artifact(stream: &str, wanted: &Wanted) -> Result<PathBuf, String> {
    let mut selected: Vec<PathBuf> = Vec::new();
    let mut matched_target_without_library = false;
    let mut seen: Vec<String> = Vec::new();
    for (index, line) in stream.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            format!(
                "cargo stream line {} is not JSON ({e}); refusing to guess the artifact",
                index + 1
            )
        })?;
        if v["reason"] != "compiler-artifact" {
            continue;
        }
        let package_id = v["package_id"].as_str().unwrap_or("");
        let target = v["target"]["name"].as_str().unwrap_or("");
        if package_id != wanted.package_id || target != wanted.target {
            let entry = format!("{package_id}:{target}");
            if !seen.contains(&entry) {
                seen.push(entry);
            }
            continue;
        }
        let mut library: Option<PathBuf> = None;
        for filename in v["filenames"].as_array().into_iter().flatten() {
            if let Some(path) = filename.as_str() {
                if is_shared_library(path) {
                    library = Some(PathBuf::from(path));
                }
            }
        }
        match library {
            Some(path) => {
                if !selected.contains(&path) {
                    selected.push(path);
                }
            }
            None => matched_target_without_library = true,
        }
    }
    match selected.as_slice() {
        [one] => Ok(one.clone()),
        [] if matched_target_without_library => Err(format!(
            "cargo compiled `{}`/`{}` but reported no shared-library filename; \
             the target is not a cdylib umbrella can load",
            wanted.package, wanted.target
        )),
        [] => Err(format!(
            "cargo reported no artifact for `{}`/`{}`; artifacts seen: {}",
            wanted.package,
            wanted.target,
            if seen.is_empty() {
                "none".to_string()
            } else {
                seen.join(", ")
            }
        )),
        many => Err(format!(
            "cargo reported {} shared libraries for `{}`/`{}` ({}); \
             refusing to pick one",
            many.len(),
            wanted.package,
            wanted.target,
            many.iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(package_id: &str, target: &str, filenames: &[&str]) -> String {
        serde_json::json!({
            "reason": "compiler-artifact",
            "package_id": package_id,
            "target": {"name": target, "kind": ["cdylib"]},
            "filenames": filenames,
        })
        .to_string()
    }

    fn wanted() -> Wanted {
        Wanted {
            package_id: "path+file:///w#rscrate@0.1.0".into(),
            package: "rscrate".into(),
            target: "rscrate".into(),
        }
    }

    #[test]
    fn selects_the_wanted_target_and_ignores_other_shared_libraries() {
        let stream = [
            // A proc-macro is a shared library too, and cargo lists it first.
            r#"{"reason":"compiler-artifact","package_id":"path+file:///w#procmacro@0.1.0","target":{"name":"procmacro","kind":["proc-macro"]},"filenames":["/w/target/release/libprocmacro.so"]}"#.to_string(),
            // A dependency's cdylib must not win either.
            artifact("path+file:///w#dep@0.1.0", "dep", &["/w/target/release/libdep.so"]),
            artifact("path+file:///w#rscrate@0.1.0", "rscrate", &["/w/target/release/librscrate.so"]),
            r#"{"reason":"build-finished","success":true}"#.to_string(),
        ]
        .join("\n");
        let path = select_shared_artifact(&stream, &wanted()).expect("one exact match");
        assert_eq!(path, PathBuf::from("/w/target/release/librscrate.so"));
    }

    #[test]
    fn refuses_when_the_wanted_target_is_absent() {
        let stream = artifact("path+file:///w#dep@0.1.0", "dep", &["/w/libdep.so"]);
        let err = select_shared_artifact(&stream, &wanted()).unwrap_err();
        assert!(err.contains("no artifact for `rscrate`/`rscrate`"), "{err}");
        assert!(
            err.contains("#dep@0.1.0:dep"),
            "the diagnostic must name what was seen: {err}"
        );
    }

    #[test]
    fn refuses_two_shared_libraries_for_one_target() {
        let stream = [
            artifact(
                "path+file:///w#rscrate@0.1.0",
                "rscrate",
                &["/a/librscrate.so"],
            ),
            artifact(
                "path+file:///w#rscrate@0.1.0",
                "rscrate",
                &["/b/librscrate.so"],
            ),
        ]
        .join("\n");
        let err = select_shared_artifact(&stream, &wanted()).unwrap_err();
        assert!(err.contains("2 shared libraries"), "{err}");
        assert!(
            err.contains("/a/librscrate.so") && err.contains("/b/librscrate.so"),
            "{err}"
        );
    }

    #[test]
    fn refuses_a_matching_artifact_that_is_not_shared() {
        let stream = artifact(
            "path+file:///w#rscrate@0.1.0",
            "rscrate",
            &["/w/librscrate.rlib"],
        );
        let err = select_shared_artifact(&stream, &wanted()).unwrap_err();
        assert!(err.contains("no shared-library filename"), "{err}");
    }

    #[test]
    fn refuses_a_stream_it_cannot_parse() {
        let err = select_shared_artifact("not json\n", &wanted()).unwrap_err();
        assert!(err.contains("line 1 is not JSON"), "{err}");
    }

    /// A name-less id (`#0.1.0`) is the shape this cargo really emits for a
    /// path dependency, and it must still match: the id is compared verbatim.
    #[test]
    fn selects_by_id_when_the_id_carries_no_package_name() {
        let stream = artifact(
            "path+file:///w#0.1.0",
            "rscrate",
            &["/w/target/release/librscrate.so"],
        );
        let wanted = Wanted {
            package_id: "path+file:///w#0.1.0".into(),
            package: "rscrate".into(),
            target: "rscrate".into(),
        };
        assert_eq!(
            select_shared_artifact(&stream, &wanted).expect("exact id match"),
            PathBuf::from("/w/target/release/librscrate.so")
        );
    }

    fn request() -> BuildRequest {
        BuildRequest::new(PathBuf::from("/w/crate"))
    }

    #[test]
    fn the_default_build_request_is_the_old_hardcoded_invocation() {
        assert_eq!(
            build_args(&request()),
            vec![
                OsString::from("build"),
                OsString::from("--message-format=json"),
                OsString::from("--release")
            ]
        );
    }

    #[test]
    fn a_feature_list_stays_one_argument() {
        let mut req = request();
        req.features = vec!["a b".into(), "c".into()];
        let args = build_args(&req);
        assert_eq!(
            args,
            vec![
                OsString::from("build"),
                OsString::from("--message-format=json"),
                OsString::from("--release"),
                OsString::from("--features"),
                OsString::from("a b"),
                OsString::from("--features"),
                OsString::from("c"),
            ]
        );
    }

    #[test]
    fn every_explicit_field_reaches_the_argv() {
        let mut req = request();
        req.package = Some("app".into());
        req.target_dir = Some(PathBuf::from("/w/targets/custom"));
        req.profile = Profile::Dev;
        req.locked = true;
        req.offline = true;
        assert_eq!(
            build_args(&req),
            vec![
                OsString::from("build"),
                OsString::from("--message-format=json"),
                OsString::from("--target-dir"),
                OsString::from("/w/targets/custom"),
                OsString::from("--package"),
                OsString::from("app"),
                OsString::from("--locked"),
                OsString::from("--offline"),
            ],
            "dev is the absence of --release, not a --profile flag"
        );
    }

    #[test]
    fn a_compiler_wrapper_is_refused_and_flags_are_recorded() {
        let recorded = build_environment([
            ("RUSTFLAGS", "-Cdebuginfo=1"),
            ("UNRELATED", "x"),
            ("CARGO_BUILD_TARGET", "x86_64-unknown-linux-gnu"),
        ])
        .expect("no wrapper");
        assert_eq!(
            recorded,
            vec![
                (
                    "CARGO_BUILD_TARGET".to_string(),
                    "x86_64-unknown-linux-gnu".to_string()
                ),
                ("RUSTFLAGS".to_string(), "-Cdebuginfo=1".to_string()),
            ]
        );
        for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
            let err = build_environment([(wrapper, "sccache")]).unwrap_err();
            assert!(err.contains(wrapper) && err.contains("sccache"), "{err}");
        }
    }

    #[test]
    fn the_package_name_selects_a_workspace_member() {
        let meta = serde_json::json!({
            "packages": [
                {
                    "id": "path+file:///w#alpha@0.1.0",
                    "name": "alpha",
                    "manifest_path": "/w/alpha/Cargo.toml",
                    "targets": [{"name": "alpha", "kind": ["cdylib"]}]
                },
                {
                    "id": "path+file:///w#beta@0.1.0",
                    "name": "beta",
                    "manifest_path": "/w/beta/Cargo.toml",
                    "targets": [{"name": "beta", "kind": ["cdylib"]}]
                }
            ]
        })
        .to_string();
        let chosen =
            wanted_from_metadata_for(&meta, Path::new("/w"), Some("beta")).expect("by name");
        assert_eq!(chosen.package, "beta");
        assert_eq!(chosen.package_id, "path+file:///w#beta@0.1.0");
        // The directory alone sees no package when the members live below it.
        let err = wanted_from_metadata_for(&meta, Path::new("/w"), None).unwrap_err();
        assert!(err.contains("names no package at /w"), "{err}");
        let err = wanted_from_metadata_for(&meta, Path::new("/w"), Some("gamma")).unwrap_err();
        assert!(err.contains("no package `gamma`"), "{err}");
    }

    fn metadata(manifest: &str, name: &str, kinds: &[&[&str]]) -> String {
        let targets: Vec<serde_json::Value> = kinds
            .iter()
            .map(|kind| serde_json::json!({"name": name, "kind": kind}))
            .collect();
        serde_json::json!({
            "packages": [{
                "id": format!("path+file:///w#{name}@0.1.0"),
                "name": name,
                "manifest_path": manifest,
                "targets": targets,
            }]
        })
        .to_string()
    }

    #[test]
    fn reads_the_cdylib_target_for_the_crate_directory() {
        let meta = metadata("/w/crate/Cargo.toml", "rscrate", &[&["lib"], &["cdylib"]]);
        let wanted = wanted_from_metadata(&meta, Path::new("/w/crate")).expect("identity");
        assert_eq!(
            wanted,
            Wanted {
                package_id: "path+file:///w#rscrate@0.1.0".into(),
                package: "rscrate".into(),
                target: "rscrate".into()
            }
        );
    }

    #[test]
    fn refuses_a_package_without_a_cdylib_target() {
        let meta = metadata("/w/crate/Cargo.toml", "rscrate", &[&["lib"]]);
        let err = wanted_from_metadata(&meta, Path::new("/w/crate")).unwrap_err();
        assert!(err.contains("no cdylib target"), "{err}");
    }

    #[test]
    fn refuses_an_ambiguous_crate_directory() {
        let meta = metadata(
            "/w/crate/Cargo.toml",
            "rscrate",
            &[&["cdylib"], &["cdylib"]],
        );
        let err = wanted_from_metadata(&meta, Path::new("/w/crate")).unwrap_err();
        assert!(err.contains("2 cdylib targets"), "{err}");
    }
}
