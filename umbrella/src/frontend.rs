//! U07 — the fail-closed compile-only CPS artifact, from the host side.
//!
//! The front-end is a subprocess, so a zero exit status is not evidence by
//! itself: the prototype passed `CPS_JIT_ADVISORY=1` and `--allow-all`, let the
//! child attempt a JIT compile, and then read whatever happened to be at a
//! `$TMPDIR/umbrella-<pid>.cps` path. That accepts an empty file, a file from
//! an earlier run, and a child that exited 0 after printing a lowering error.
//!
//! This module makes the artifact the *only* success condition:
//!
//! * the compiler runs with `--emit-cps-only`, which never initializes the
//!   bridge and never invokes target code (see `lib/Compiler.ml`);
//! * no `--allow-all`: a compile-only artifact does not invoke the
//!   authorization engine, and must not widen the grants a later run uses;
//! * both outputs live in a fresh private staging directory and are removed
//!   afterwards, so a stale file cannot be read as this run's output;
//! * the child's own completion lines are parsed and checked against the bytes
//!   on disk — missing output, zero functions, a short write and a line naming
//!   another path are all refusals, not warnings;
//! * liquid-required mode is threaded explicitly (`AUSTRAL_LIQUID_REQUIRED`),
//!   and the ambient value is removed otherwise. Optional checking therefore
//!   stays optional, and required checking cannot be silently downgraded.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::process::{self, Exit};
use crate::staging::Staging;

/// A hang is bounded, not policed: the same deadline U06 gave the prototype.
pub const FRONT_END_DEADLINE: Duration = Duration::from_secs(300);

/// Whether liquid checking was requested for this artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liquid {
    Optional,
    Required,
}

/// One typed export in the artifact: the name the JIT will resolve and the CPS
/// wire types it takes and returns. Parameter *names* are dropped here because
/// the caller passes values by position.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Export {
    pub name: String,
    pub params: Vec<String>,
    pub returns: String,
}

/// A compile-only CPS artifact and the facts the child reported about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// The source modules this run was given (the artifact covers all of them).
    pub modules: Vec<PathBuf>,
    /// Module names the compiler reported while emitting.
    pub module_names: Vec<String>,
    /// Typed exports from every module compiled in this prefix, deduplicated by
    /// name. Two different signatures for one name are a collision, not a
    /// merge.
    pub exports: Vec<Export>,
    pub cps: Vec<u8>,
    pub cps_sha256: String,
    pub functions: usize,
    /// The adapter descriptor document, still unparsed by this module.
    pub adapters: String,
    pub liquid: Liquid,
    /// `liquid:` diagnostic lines the child printed. Empty on a clean run.
    pub liquid_notes: Vec<String>,
}

/// Run the compiler over `modules` in compile-only mode.
pub fn compile(
    austral: &Path,
    modules: &[PathBuf],
    liquid_required: bool,
) -> Result<Artifact, String> {
    if modules.is_empty() {
        return Err("compile-only artifact: no modules".to_string());
    }
    let staging = Staging::create("front").map_err(|e| format!("staging: {e}"))?;
    let cps_path = staging.fresh_path("module.cps")?;
    let adapters_path = staging.fresh_path("module.adapters")?;

    let mut cmd = Command::new(austral);
    // The dune-built front-end links libaustral_cranelift_bridge.so even in
    // emit-only mode; the loader still has to find it.
    if let Some(dir) = austral.parent() {
        let bridge_dir = dir.join("safestos/cranelift/target/release");
        if bridge_dir.join("libaustral_cranelift_bridge.so").exists() {
            let mut lp = bridge_dir.display().to_string();
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                lp.push(':');
                lp.push_str(&old.to_string_lossy());
            }
            cmd.env("LD_LIBRARY_PATH", lp);
        }
    }
    // This mode must not depend on the advisory opt-out at all. Removing it
    // means an ambient value cannot turn a lowering failure into a pass.
    cmd.env_remove("CPS_JIT_ADVISORY");
    if liquid_required {
        cmd.env("AUSTRAL_LIQUID_REQUIRED", "1");
    } else {
        cmd.env_remove("AUSTRAL_LIQUID_REQUIRED");
    }
    cmd.arg("compile");
    for m in modules {
        cmd.arg(m);
    }
    cmd.arg("--use-cps-jit")
        .arg("--emit-cps-only")
        .arg("--target-type=tc")
        .arg(format!("--emit-cps={}", cps_path.display()))
        .arg(format!("--emit-adapters={}", adapters_path.display()));

    let outcome = process::run(cmd, FRONT_END_DEADLINE)
        .map_err(|e| format!("run {}: {e}", austral.display()))?;
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    if outcome.timed_out {
        return Err(format!(
            "austral front-end exceeded its {FRONT_END_DEADLINE:?} deadline and was \
             killed with its process group\nstdout:\n{}\nstderr:\n{stderr}",
            String::from_utf8_lossy(&outcome.stdout)
        ));
    }
    if outcome.exit != Exit::Code(0) {
        return Err(format!(
            "austral front-end failed: {:?}\nstdout:\n{}\nstderr:\n{stderr}",
            outcome.exit,
            String::from_utf8_lossy(&outcome.stdout)
        ));
    }

    let (functions, claimed_bytes) = parse_generated(&stderr)?;
    if functions == 0 {
        return Err(format!(
            "the compiler exited 0 but lowered no functions; refusing an empty artifact\n\
             stderr:\n{stderr}"
        ));
    }
    let emitted_paths = parse_emitted_paths(&stderr)?;
    if let Some(other) = emitted_paths.iter().find(|p| **p != cps_path) {
        return Err(format!(
            "the compiler named {} as its CPS artifact, not the staged {}; refusing \
             to read a different file",
            other.display(),
            cps_path.display()
        ));
    }
    let cps = fs::read(&cps_path).map_err(|e| {
        format!(
            "the compiler exited 0 but no CPS artifact exists at {}: {e}\nstderr:\n{stderr}",
            cps_path.display()
        )
    })?;
    if cps.is_empty() {
        return Err(format!(
            "the compiler exited 0 but wrote an empty CPS artifact to {}\nstderr:\n{stderr}",
            cps_path.display()
        ));
    }
    if cps.len() != claimed_bytes {
        return Err(format!(
            "the compiler reported {claimed_bytes} CPS bytes but wrote {} to {}; the \
             artifact is incomplete or was replaced\nstderr:\n{stderr}",
            cps.len(),
            cps_path.display()
        ));
    }
    let adapters = fs::read_to_string(&adapters_path).map_err(|e| {
        format!(
            "the compiler exited 0 but no adapter document exists at {}: {e}\nstderr:\n{stderr}",
            adapters_path.display()
        )
    })?;
    let module_names = parse_module_names(&stderr);
    let exports = parse_exports(&stderr)?;
    let liquid_notes: Vec<String> = stderr
        .lines()
        .filter(|l| l.trim_start().starts_with("liquid:"))
        .map(|l| l.trim().to_string())
        .collect();
    let liquid = if liquid_required {
        Liquid::Required
    } else {
        Liquid::Optional
    };
    let cps_sha256 = sha256_hex(&cps);
    Ok(Artifact {
        modules: modules.to_vec(),
        module_names,
        exports,
        cps,
        cps_sha256,
        functions,
        adapters,
        liquid,
        liquid_notes,
    })
}

/// One digest over every module's CPS, in emission order, length-prefixed so
/// two different splits of the same bytes cannot collide.
pub fn digest(artifacts: &[Artifact]) -> String {
    let mut h = Sha256::new();
    for a in artifacts {
        h.update((a.cps.len() as u64).to_le_bytes());
        h.update(&a.cps);
    }
    hex(h.finalize().as_slice())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(h.finalize().as_slice())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// `CPS JIT: Generated <n> functions (<m> bytes)`. A prefix compile runs one
/// `compile_mod` per module and every one emits a line for the same
/// `--emit-cps` path, so the *last* line describes the artifact on disk.
fn parse_generated(stderr: &str) -> Result<(usize, usize), String> {
    let line = stderr
        .lines()
        .filter_map(|l| l.trim().strip_prefix("CPS JIT: Generated "))
        .next_back()
        .ok_or_else(|| {
            format!(
                "the compiler exited 0 without reporting a generated CPS artifact\nstderr:\n{stderr}"
            )
        })?;
    let (count, rest) = line
        .split_once(" functions (")
        .ok_or_else(|| format!("cannot parse the CPS completion line `{line}`"))?;
    let bytes = rest
        .strip_suffix(" bytes)")
        .ok_or_else(|| format!("cannot parse the CPS size in `{line}`"))?;
    let functions = count
        .parse::<usize>()
        .map_err(|_| format!("cannot parse the function count in `{line}`"))?;
    let bytes = bytes
        .parse::<usize>()
        .map_err(|_| format!("cannot parse the byte count in `{line}`"))?;
    Ok((functions, bytes))
}

/// Every `CPS JIT: Emitted CPS binary to <path>` line; every one must name the
/// staged file (the last module is what survives the overwrite).
fn parse_emitted_paths(stderr: &str) -> Result<Vec<PathBuf>, String> {
    let paths: Vec<PathBuf> = stderr
        .lines()
        .filter_map(|l| l.trim().strip_prefix("CPS JIT: Emitted CPS binary to "))
        .map(|p| PathBuf::from(p.trim()))
        .collect();
    if paths.is_empty() {
        return Err(format!(
            "the compiler exited 0 without naming the CPS artifact it wrote\nstderr:\n{stderr}"
        ));
    }
    Ok(paths)
}

const CPS_TYPES: &[&str] = &["Unit", "Int64", "Int32", "Bool", "String", "Float64"];

/// `CPS JIT: export\t<name>\t<Type>,<Type>\t<Type>`. Tabs, not spaces,
/// because a monomorph's name is `monomorph_(Id.MonoId 15)` and contains
/// spaces/parentheses. A prefix compile emits one set per module; an identical
/// repeated declaration is deduplicated, while the same name with a different
/// signature is refused (no first-wins).
fn parse_exports(stderr: &str) -> Result<Vec<Export>, String> {
    let mut exports: BTreeMap<String, Export> = BTreeMap::new();
    for line in stderr
        .lines()
        .filter_map(|l| l.trim().strip_prefix("CPS JIT: export\t"))
    {
        let mut fields = line.split('\t');
        let name = fields
            .next()
            .ok_or_else(|| format!("cannot parse the typed export line `{line}`"))?;
        let params = fields
            .next()
            .ok_or_else(|| format!("cannot parse the typed export line `{line}`"))?;
        let returns = fields
            .next()
            .ok_or_else(|| format!("cannot parse the typed export line `{line}`"))?;
        if fields.next().is_some() {
            return Err(format!(
                "typed export line `{line}` has more than name/parameters/return"
            ));
        }
        if name.is_empty() {
            return Err(format!("typed export has an empty name in `{line}`"));
        }
        let params: Vec<String> = if params.is_empty() {
            Vec::new()
        } else {
            params.split(',').map(str::to_string).collect()
        };
        for ty in params
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(returns))
        {
            if !CPS_TYPES.contains(&ty) {
                return Err(format!(
                    "typed export `{name}` names unknown CPS type `{ty}`; the artifact \
                     and this reader disagree about the wire types"
                ));
            }
        }
        let export = Export {
            name: name.to_string(),
            params,
            returns: returns.to_string(),
        };
        match exports.get(name) {
            Some(previous) if previous == &export => {}
            Some(previous) => {
                return Err(format!(
                    "typed export `{name}` is declared twice with different signatures \
                     ({:?} -> {} and {:?} -> {}); a short-name collision must be refused, \
                     not resolved first-wins",
                    previous.params, previous.returns, export.params, export.returns
                ));
            }
            None => {
                exports.insert(name.to_string(), export);
            }
        }
    }
    if exports.is_empty() {
        return Err(format!(
            "the compiler exited 0 but reported no typed exports; a compile-only \
             artifact without its export identities cannot be admitted\nstderr:\n{stderr}"
        ));
    }
    Ok(exports.into_values().collect())
}

fn parse_module_names(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|l| l.trim().strip_prefix("CPS JIT: module "))
        .map(|s| s.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        staging: Staging,
        austral: PathBuf,
        args: PathBuf,
        env: PathBuf,
    }

    impl Fixture {
        fn new(body: &str) -> Fixture {
            let staging = Staging::create("fake-front").unwrap();
            let austral = staging.fresh_path("austral").unwrap();
            let args = staging.fresh_path("args").unwrap();
            let env = staging.fresh_path("env").unwrap();
            let preamble = format!(
                "out=\"\"; ad=\"\"\n\
                 for a in \"$@\"; do\n\
                 case \"$a\" in --emit-cps=*) out=\"${{a#--emit-cps=}}\";; \
                 --emit-adapters=*) ad=\"${{a#--emit-adapters=}}\";; esac\n\
                 done\n\
                 printf '%s\\n' \"$@\" > '{}'\n\
                 printf 'advisory=%s\\n' \"${{CPS_JIT_ADVISORY-unset}}\" > '{}'\n\
                 printf 'liquid=%s\\n' \"${{AUSTRAL_LIQUID_REQUIRED-unset}}\" >> '{}'\n\
                 printf 'abi scalar 1\\n' > \"$ad\"\n",
                args.display(),
                env.display(),
                env.display()
            );
            fs::write(&austral, format!("#!/bin/sh\n{preamble}{body}\n")).unwrap();
            let mut perms = fs::metadata(&austral).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&austral, perms).unwrap();
            Fixture {
                staging,
                austral,
                args,
                env,
            }
        }

        fn modules(&self) -> Vec<PathBuf> {
            vec![self.staging.path().join("M.aum")]
        }

        fn args(&self) -> String {
            fs::read_to_string(&self.args).unwrap()
        }

        fn env(&self) -> String {
            fs::read_to_string(&self.env).unwrap()
        }
    }

    fn emit_ok_body(bytes: usize, functions: usize, claim: usize) -> String {
        let body = "CPS!";
        assert_eq!(body.len(), bytes);
        format!(
            "printf '{body}' > \"$out\"\n\
             printf 'CPS JIT: module M\\nCPS JIT: Generated {functions} functions ({claim} bytes)\\n\
             CPS JIT: export\\tmain\\t\\tInt64\\n\
             CPS JIT: Emitted CPS binary to %s\\n' \"$out\" >&2\n\
             exit 0"
        )
    }

    #[test]
    fn a_complete_artifact_is_accepted_without_the_advisory_or_allow_all() {
        let f = Fixture::new(&emit_ok_body(4, 1, 4));
        let a = compile(&f.austral, &f.modules(), false).expect("artifact");
        assert_eq!(a.cps, b"CPS!");
        assert_eq!(a.functions, 1);
        assert_eq!(a.cps_sha256.len(), 64);
        assert_eq!(a.module_names, vec!["M"]);
        assert_eq!(
            a.exports,
            vec![Export {
                name: "main".to_string(),
                params: Vec::new(),
                returns: "Int64".to_string(),
            }]
        );
        let args = f.args();
        assert!(args.contains("--emit-cps-only"), "{args}");
        assert!(!args.contains("--allow-all"), "{args}");
        let env = f.env();
        assert!(env.contains("advisory=unset"), "{env}");
        assert!(env.contains("liquid=unset"), "{env}");
        assert_eq!(digest(std::slice::from_ref(&a)).len(), 64);
    }

    #[test]
    fn liquid_required_is_threaded_and_optional_clears_ambient_values() {
        let f = Fixture::new(&emit_ok_body(4, 1, 4));
        let a = compile(&f.austral, &f.modules(), true).expect("artifact");
        assert_eq!(a.liquid, Liquid::Required);
        assert!(f.env().contains("liquid=1"), "{}", f.env());
        // A second fixture proves the ambient value would otherwise leak in.
        let f2 = Fixture::new(&emit_ok_body(4, 1, 4));
        assert!(compile(&f2.austral, &f2.modules(), false).is_ok());
        assert!(f2.env().contains("liquid=unset"), "{}", f2.env());
    }

    #[test]
    fn a_prefix_compile_takes_the_last_completion_and_checks_every_path() {
        let body = "printf 'CPS!' > \"$out\"\n\
                    printf 'CPS JIT: module Dep\nCPS JIT: Generated 9 functions (9 bytes)\n\
                    CPS JIT: export\\tdep\\tInt64,Bool\\tInt32\\n\
                    CPS JIT: Emitted CPS binary to %s\n' \"$out\" >&2\n\
                    printf 'CPS JIT: module M\nCPS JIT: Generated 1 functions (4 bytes)\n\
                    CPS JIT: export\\tmain\\t\\tInt64\\n\
                    CPS JIT: Emitted CPS binary to %s\n' \"$out\" >&2\n\
                    exit 0";
        let f = Fixture::new(body);
        let a = compile(&f.austral, &f.modules(), false).expect("artifact");
        assert_eq!(a.functions, 1);
        assert_eq!(a.module_names, vec!["Dep", "M"]);
        assert_eq!(a.exports.len(), 2);
        assert_eq!(a.exports[0].name, "dep");
        assert_eq!(a.exports[0].params, vec!["Int64", "Bool"]);
        assert_eq!(a.exports[1].name, "main");
        assert_eq!(a.cps, b"CPS!");
    }

    #[test]
    fn a_missing_typed_export_is_refused() {
        let body = "printf 'CPS!' > \"$out\"\n\
                    printf 'CPS JIT: module M\\nCPS JIT: Generated 1 functions (4 bytes)\\n\
                    CPS JIT: Emitted CPS binary to %s\\n' \"$out\" >&2\nexit 0";
        let f = Fixture::new(body);
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(err.contains("reported no typed exports"), "{err}");
    }

    #[test]
    fn the_same_export_with_two_signatures_is_refused() {
        let body = "printf 'CPS!' > \"$out\"\n\
                    printf 'CPS JIT: module M\\nCPS JIT: Generated 1 functions (4 bytes)\\n\
                    CPS JIT: export\\tmain\\t\\tInt64\\n\
                    CPS JIT: export\\tmain\\tInt64\\tInt64\\n\
                    CPS JIT: Emitted CPS binary to %s\\n' \"$out\" >&2\nexit 0";
        let f = Fixture::new(body);
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(
            err.contains("declared twice with different signatures"),
            "{err}"
        );
    }

    #[test]
    fn a_child_that_exits_zero_without_output_is_refused() {
        let f = Fixture::new(&emit_ok_body(4, 1, 4).replace("printf 'CPS!' > \"$out\"\n", ""));
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(err.contains("no CPS artifact exists"), "{err}");
    }

    #[test]
    fn an_empty_artifact_is_refused_even_when_the_sizes_agree() {
        let body = "printf '' > \"$out\"\n\
                    printf 'CPS JIT: module M\\nCPS JIT: Generated 1 functions (0 bytes)\\n\
                    CPS JIT: Emitted CPS binary to %s\\n' \"$out\" >&2\nexit 0";
        let f = Fixture::new(body);
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(err.contains("empty CPS artifact"), "{err}");
    }

    #[test]
    fn zero_functions_is_a_refusal_not_an_empty_artifact() {
        let f = Fixture::new(&emit_ok_body(4, 0, 4));
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(err.contains("lowered no functions"), "{err}");
    }

    #[test]
    fn a_short_write_is_refused() {
        let f = Fixture::new(&emit_ok_body(4, 1, 5));
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(err.contains("reported 5 CPS bytes but wrote 4"), "{err}");
    }

    #[test]
    fn a_nonzero_child_is_reported_with_its_exit() {
        let body = "printf 'CPS JIT: Unhandled exception\\n' >&2\nexit 7";
        let f = Fixture::new(body);
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(err.contains("front-end failed: Code(7)"), "{err}");
    }

    #[test]
    fn a_completion_line_naming_another_path_is_refused() {
        let body = "printf 'CPS!' > \"$out\"\n\
                    printf 'CPS JIT: module M\\nCPS JIT: Generated 1 functions (4 bytes)\\n\
                    CPS JIT: Emitted CPS binary to /tmp/other.cps\\n' >&2\nexit 0";
        let f = Fixture::new(body);
        let err = compile(&f.austral, &f.modules(), false).unwrap_err();
        assert!(err.contains("refusing to read a different file"), "{err}");
    }
}
