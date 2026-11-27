//! Command-line grammar (PLAN_umbrella_production U03, finding UR12).
//!
//! The prototype parser matched options by prefix and consumed the next token
//! unconditionally, so `--manifest-garbage` read as `--manifest`,
//! `--entry --expect 7` set the entry name to `--expect`, `-x` became a module
//! path and `--help` was an unknown flag. This module replaces it with exact
//! matching over OS-native arguments:
//!
//! * options are exactly `--name` or `--name=value`; no prefix match;
//! * `--` ends option parsing, so a module really named `--x.aum` is reachable;
//! * a value is never taken from a following token that looks like an option
//!   (`--entry --expect 7` is an error), except a negative integer for
//!   `--expect` (`--expect -7`, unambiguous);
//! * paths remain `PathBuf` from `OsString` — no lossy UTF-8 round trip; only
//!   option *names* and the entry name must be UTF-8;
//! * repeating a single-valued option is a usage error, not "last wins";
//! * `--help`/`--version` win anywhere before `--` and exit 0.
//!
//! Usage errors are a distinct exit status (2) from operational failures (1).

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::cargo::Profile;

pub const USAGE: &str = "usage: umbrella [--project <json>] [--manifest <toml>] [--entry <name>]\n\
       \x20            [--austral <bin>] [--cratedir <dir>] [--expect <i64>] [--json]\n\
       \x20            [--cargo-package <name>] [--target-dir <dir>]\n\
       \x20            [--profile dev|release] [--features <list>]... [--locked] [--offline]\n\
       \x20            [--liquid-required] [--emit-trust-record <json>]\n\
       \x20            [--cache-dir <dir>]\n\
       \x20            [--descriptor-document <file>] [--emit-wrappers <dir>]\n\
       \x20            [--build-wrappers <dir> --provider <lib>]\n\
       \x20            [--] <module.aum>...\n\
       \x20      umbrella --help | --version";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cli {
    pub modules: Vec<PathBuf>,
    pub manifest: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub entry: String,
    pub austral: Option<PathBuf>,
    pub crate_dir: Option<PathBuf>,
    pub expect: Option<i64>,
    /// JSON event mode: one typed event per stdout line, human text on stderr.
    pub json: bool,
    // U05: the build is described explicitly instead of read from the
    // environment or hardcoded.
    pub cargo_package: Option<String>,
    pub target_dir: Option<PathBuf>,
    pub profile: Profile,
    pub features: Vec<String>,
    pub locked: bool,
    pub offline: bool,
    /// U07: make a missing liquid engine a hard artifact failure instead of a
    /// structured skip. Off by default, matching the compiler's optional mode.
    pub liquid_required: bool,
    /// U10: write the signature and descriptor pins the admitted artifact
    /// implies, so a project document can be generated instead of copied by
    /// hand.
    pub emit_trust_record: Option<PathBuf>,
    /// U18: where the run's derived artifacts live. Left unset, a project
    /// document defaults it to `<project dir>/.umbrella-cache`; a run with no
    /// project document keeps the cache off. The key is derived from every
    /// input the artifacts depend on.
    pub cache_dir: Option<PathBuf>,
    /// U21 bounded start: the compiler-emitted adapter document to render
    /// wrappers from, without compiling or loading a module.
    pub descriptor_document: Option<PathBuf>,
    /// Where the rendered `wrappers.rs` is written. One of this and
    /// `build_wrappers` is required with `descriptor_document`.
    pub emit_wrappers: Option<PathBuf>,
    /// U21: compile the rendered wrappers into a loadable cdylib instead of
    /// only writing their source. Needs `--provider` to link the native image.
    pub build_wrappers: Option<PathBuf>,
    /// The provider image the built adapter crate links by absolute path.
    /// Only meaningful with `build_wrappers`.
    pub provider: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Run,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub action: Action,
    pub cli: Cli,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl UsageError {
    fn new(msg: impl Into<String>) -> Self {
        UsageError(msg.into())
    }
}

/// Is this token something the grammar could read as an option? A bare `-` is
/// not (it is a path), and `-7` is only a value, never an option.
fn looks_like_option(arg: &OsStr) -> bool {
    match arg.to_str() {
        Some(s) => s.len() > 1 && s.starts_with('-'),
        None => false,
    }
}

/// A value in option position that may legitimately be negative (only used by
/// `--expect`): `-7` / `-42`, but not `-x` or `--expect`.
fn looks_like_negative_integer(arg: &OsStr) -> bool {
    match arg.to_str() {
        Some(s) => s.len() > 1 && s.starts_with('-') && s[1..].bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

fn option_name(arg: &OsStr) -> Option<(&str, Option<&str>)> {
    let s = arg.to_str()?;
    let rest = s.strip_prefix("--")?;
    if rest.is_empty() {
        return None; // `--` itself
    }
    match rest.split_once('=') {
        Some((name, value)) => Some((name, Some(value))),
        None => Some((rest, None)),
    }
}

/// Parse `args` (without argv[0]) into an action. `Err` is always a usage
/// error: the caller exits 2 and must not spawn anything.
pub fn parse<I>(args: I) -> Result<Parsed, UsageError>
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let mut cli = Cli {
        entry: "compute".to_string(),
        ..Cli::default()
    };
    let mut seen: Vec<&'static str> = Vec::new();
    let mut positional_only = false;
    let mut i = 0;

    // Set once, so a duplicate is refused rather than silently overwritten.
    fn once(seen: &mut Vec<&'static str>, flag: &'static str) -> Result<(), UsageError> {
        if seen.contains(&flag) {
            return Err(UsageError::new(format!(
                "duplicate option `--{flag}` (single-valued; pass it once)"
            )));
        }
        seen.push(flag);
        Ok(())
    }

    while i < args.len() {
        let arg = &args[i];
        if positional_only {
            push_module(&mut cli, arg)?;
            i += 1;
            continue;
        }
        if arg == OsStr::new("--") {
            positional_only = true;
            i += 1;
            continue;
        }
        let Some((name, inline)) = option_name(arg) else {
            // Not an option. A bare token is a module path; anything starting
            // with `-` (other than the `--` handled above) is refused, because
            // the old parser accepted `-x` as a module.
            if looks_like_option(arg) {
                return Err(UsageError::new(format!(
                    "unknown option `{}` (use `--` before a path that begins with `-`)",
                    arg.to_string_lossy()
                )));
            }
            push_module(&mut cli, arg)?;
            i += 1;
            continue;
        };

        // A value is taken inline (`--k=v`) or from the next token, but never
        // from a token in option position — that was CLI03's defect.
        let mut take_value = |flag: &str, allow_negative: bool| -> Result<OsString, UsageError> {
            if let Some(v) = inline {
                return Ok(OsString::from(v));
            }
            match args.get(i + 1) {
                Some(next)
                    if !looks_like_option(next)
                        || (allow_negative && looks_like_negative_integer(next)) =>
                {
                    i += 1;
                    Ok(next.clone())
                }
                _ => Err(UsageError::new(format!("`--{flag}` needs a value"))),
            }
        };

        match name {
            "help" => {
                if inline.is_some() {
                    return Err(UsageError::new("`--help` takes no value"));
                }
                return Ok(Parsed {
                    action: Action::Help,
                    cli,
                });
            }
            "version" => {
                if inline.is_some() {
                    return Err(UsageError::new("`--version` takes no value"));
                }
                return Ok(Parsed {
                    action: Action::Version,
                    cli,
                });
            }
            "json" => {
                if inline.is_some() {
                    return Err(UsageError::new("`--json` takes no value"));
                }
                once(&mut seen, "json")?;
                cli.json = true;
            }
            "project" => {
                once(&mut seen, "project")?;
                cli.project = Some(path_value(take_value("project", false)?, "project")?);
            }
            "manifest" => {
                once(&mut seen, "manifest")?;
                cli.manifest = Some(path_value(take_value("manifest", false)?, "manifest")?);
            }
            "austral" => {
                once(&mut seen, "austral")?;
                cli.austral = Some(path_value(take_value("austral", false)?, "austral")?);
            }
            "cratedir" => {
                once(&mut seen, "cratedir")?;
                cli.crate_dir = Some(path_value(take_value("cratedir", false)?, "cratedir")?);
            }
            "entry" => {
                once(&mut seen, "entry")?;
                let raw = take_value("entry", false)?;
                let name = raw
                    .to_str()
                    .ok_or_else(|| UsageError::new("`--entry` must be valid UTF-8"))?;
                if name.is_empty() {
                    return Err(UsageError::new("`--entry` must not be empty"));
                }
                cli.entry = name.to_string();
            }
            "cargo-package" => {
                once(&mut seen, "cargo-package")?;
                let raw = take_value("cargo-package", false)?;
                let name = raw
                    .to_str()
                    .ok_or_else(|| UsageError::new("`--cargo-package` must be valid UTF-8"))?;
                if name.is_empty() {
                    return Err(UsageError::new("`--cargo-package` must not be empty"));
                }
                cli.cargo_package = Some(name.to_string());
            }
            "target-dir" => {
                once(&mut seen, "target-dir")?;
                cli.target_dir = Some(path_value(take_value("target-dir", false)?, "target-dir")?);
            }
            "profile" => {
                once(&mut seen, "profile")?;
                let raw = take_value("profile", false)?;
                let text = raw
                    .to_str()
                    .ok_or_else(|| UsageError::new("`--profile` must be valid UTF-8"))?;
                cli.profile = Profile::parse(text).map_err(UsageError::new)?;
            }
            "features" => {
                let raw = take_value("features", false)?;
                let text = raw
                    .to_str()
                    .ok_or_else(|| UsageError::new("`--features` must be valid UTF-8"))?;
                if text.is_empty() {
                    return Err(UsageError::new("`--features` must not be empty"));
                }
                // Repeatable on purpose: each occurrence is passed to cargo as
                // its own `--features` argument, so no list is re-split.
                cli.features.push(text.to_string());
            }
            "locked" => {
                if inline.is_some() {
                    return Err(UsageError::new("`--locked` takes no value"));
                }
                once(&mut seen, "locked")?;
                cli.locked = true;
            }
            "offline" => {
                if inline.is_some() {
                    return Err(UsageError::new("`--offline` takes no value"));
                }
                once(&mut seen, "offline")?;
                cli.offline = true;
            }
            "liquid-required" => {
                if inline.is_some() {
                    return Err(UsageError::new("`--liquid-required` takes no value"));
                }
                once(&mut seen, "liquid-required")?;
                cli.liquid_required = true;
            }
            "emit-trust-record" => {
                once(&mut seen, "emit-trust-record")?;
                cli.emit_trust_record = Some(path_value(
                    take_value("emit-trust-record", false)?,
                    "emit-trust-record",
                )?);
            }
            "cache-dir" => {
                once(&mut seen, "cache-dir")?;
                cli.cache_dir = Some(path_value(take_value("cache-dir", false)?, "cache-dir")?);
            }
            "descriptor-document" => {
                once(&mut seen, "descriptor-document")?;
                cli.descriptor_document = Some(path_value(
                    take_value("descriptor-document", false)?,
                    "descriptor-document",
                )?);
            }
            "emit-wrappers" => {
                once(&mut seen, "emit-wrappers")?;
                cli.emit_wrappers = Some(path_value(
                    take_value("emit-wrappers", false)?,
                    "emit-wrappers",
                )?);
            }
            "build-wrappers" => {
                once(&mut seen, "build-wrappers")?;
                cli.build_wrappers = Some(path_value(
                    take_value("build-wrappers", false)?,
                    "build-wrappers",
                )?);
            }
            "provider" => {
                once(&mut seen, "provider")?;
                cli.provider = Some(path_value(take_value("provider", false)?, "provider")?);
            }
            "expect" => {
                once(&mut seen, "expect")?;
                let raw = take_value("expect", true)?;
                let text = raw
                    .to_str()
                    .ok_or_else(|| UsageError::new("`--expect` must be valid UTF-8"))?;
                cli.expect = Some(text.parse::<i64>().map_err(|_| {
                    UsageError::new(format!("`--expect` wants an i64, got `{text}`"))
                })?);
            }
            _ => {
                return Err(UsageError::new(format!(
                    "unknown option `--{name}` (unknown strict fields are not ignored)"
                )));
            }
        }
        i += 1;
    }

    let wrapper_outputs = cli
        .emit_wrappers
        .iter()
        .chain(cli.build_wrappers.iter())
        .count();
    if cli.descriptor_document.is_none() {
        if cli.emit_wrappers.is_some() {
            return Err(UsageError::new(
                "`--emit-wrappers` is used together with `--descriptor-document`, which \
                 names the compiler's document",
            ));
        }
        if cli.build_wrappers.is_some() {
            return Err(UsageError::new(
                "`--build-wrappers` is used together with `--descriptor-document`, which \
                 names the compiler's document",
            ));
        }
        if cli.provider.is_some() {
            return Err(UsageError::new(
                "`--provider` is only meaningful with `--build-wrappers`: it is the image \
                 the generated adapter crate links",
            ));
        }
    }
    if cli.descriptor_document.is_some() {
        if wrapper_outputs == 0 {
            return Err(UsageError::new(
                "`--descriptor-document` is used together with exactly one of \
                 `--emit-wrappers <dir>` (render the source) or `--build-wrappers <dir>` \
                 (compile it); none was given",
            ));
        }
        if wrapper_outputs > 1 {
            return Err(UsageError::new(
                "`--emit-wrappers` and `--build-wrappers` write different artifacts; \
                 pass exactly one",
            ));
        }
        if cli.build_wrappers.is_some() && cli.provider.is_none() {
            return Err(UsageError::new(
                "`--build-wrappers` needs `--provider <lib>`: the adapter crate links \
                 that image and leaves its symbols to it",
            ));
        }
        if cli.build_wrappers.is_none() && cli.provider.is_some() {
            return Err(UsageError::new(
                "`--provider` is only meaningful with `--build-wrappers`; \
                 `--emit-wrappers` writes source and links nothing",
            ));
        }
        if !cli.modules.is_empty() || cli.project.is_some() {
            return Err(UsageError::new(
                "`--descriptor-document` produces wrappers only; do not pass modules \
                 or `--project` in the same run",
            ));
        }
    } else if cli.modules.is_empty() && cli.project.is_none() {
        return Err(UsageError::new(
            "at least one .aum module source is required (or `--project`)",
        ));
    }
    if cli.project.is_some() && cli.manifest.is_some() {
        return Err(UsageError::new(
            "`--project` and `--manifest` are different policy sources; pass one",
        ));
    }

    Ok(Parsed {
        action: Action::Run,
        cli,
    })
}

fn push_module(cli: &mut Cli, arg: &OsStr) -> Result<(), UsageError> {
    if arg.is_empty() {
        return Err(UsageError::new("empty module path"));
    }
    cli.modules.push(PathBuf::from(arg));
    Ok(())
}

fn path_value(value: OsString, flag: &str) -> Result<PathBuf, UsageError> {
    if value.is_empty() {
        return Err(UsageError::new(format!(
            "`--{flag}` needs a non-empty path"
        )));
    }
    Ok(PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn p(args: &[&str]) -> Result<Parsed, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    fn run(args: &[&str]) -> Cli {
        let parsed = p(args).expect("valid grammar");
        assert_eq!(parsed.action, Action::Run);
        parsed.cli
    }

    #[test]
    fn the_legacy_demo_invocation_still_parses() {
        let cli = run(&[
            "--manifest",
            "module.toml",
            "--entry",
            "compute",
            "modules/Demo.aum",
        ]);
        assert_eq!(cli.manifest, Some(PathBuf::from("module.toml")));
        assert_eq!(cli.entry, "compute");
        assert_eq!(cli.modules, vec![PathBuf::from("modules/Demo.aum")]);
    }

    #[test]
    fn equals_and_separated_forms_agree() {
        let a = run(&["--manifest=module.toml", "--expect=7", "D.aum"]);
        let b = run(&["--manifest", "module.toml", "--expect", "7", "D.aum"]);
        assert_eq!(a, b);
    }

    #[test]
    fn help_and_version_win_anywhere_before_the_separator() {
        assert_eq!(p(&["--help"]).unwrap().action, Action::Help);
        assert_eq!(p(&["D.aum", "--version"]).unwrap().action, Action::Version);
        // After `--` they are paths, not actions.
        let cli = run(&["--", "--help"]);
        assert_eq!(cli.modules, vec![PathBuf::from("--help")]);
    }

    #[test]
    fn a_prefix_lookalike_is_not_an_option() {
        let err = p(&["--manifest-garbage", "g.toml", "D.aum"]).expect_err("lookalike");
        assert!(
            err.0.contains("unknown option `--manifest-garbage`"),
            "{err}"
        );
    }

    #[test]
    fn an_option_is_never_consumed_as_a_value() {
        let err = p(&["--entry", "--expect", "7", "D.aum"]).expect_err("missing entry");
        assert_eq!(err.0, "`--entry` needs a value");
        // And `--expect` did not quietly take `7` either: parsing stopped at
        // the first error, so no flag saw the following token.
        let err = p(&["--entry", "--expect"]).expect_err("still missing");
        assert!(err.0.contains("`--entry` needs a value"), "{err}");
    }

    #[test]
    fn a_negative_expectation_is_unambiguous() {
        let cli = run(&["--expect", "-7", "D.aum"]);
        assert_eq!(cli.expect, Some(-7));
        let cli = run(&["--expect=-7", "D.aum"]);
        assert_eq!(cli.expect, Some(-7));
        let err = p(&["--expect", "-x", "D.aum"]).expect_err("not a number");
        assert!(err.0.contains("needs a value"), "{err}");
    }

    #[test]
    fn malformed_values_are_refused_by_name() {
        assert!(p(&["--entry=", "D.aum"])
            .unwrap_err()
            .0
            .contains("must not be empty"));
        assert!(p(&["--manifest=", "D.aum"])
            .unwrap_err()
            .0
            .contains("non-empty path"));
        assert!(p(&["--expect", "abc", "D.aum"])
            .unwrap_err()
            .0
            .contains("wants an i64"));
    }

    #[test]
    fn a_duplicate_single_valued_option_is_refused() {
        let err = p(&["--entry", "a", "--entry", "b", "D.aum"]).expect_err("duplicate");
        assert!(err.0.contains("duplicate option `--entry`"), "{err}");
    }

    #[test]
    fn dash_leading_tokens_need_the_separator() {
        let err = p(&["-x", "D.aum"]).expect_err("-x is not a module");
        assert!(err.0.contains("unknown option `-x`"), "{err}");
        let cli = run(&["--", "-x.aum"]);
        assert_eq!(cli.modules, vec![PathBuf::from("-x.aum")]);
    }

    #[test]
    fn os_paths_survive_without_a_utf8_round_trip() {
        // A path that is not valid UTF-8 must arrive byte-for-byte.
        let raw = vec![b'/', b't', b'm', b'p', b'/', 0xff, b'.', b'a', b'u', b'm'];
        let weird = OsString::from_vec(raw.clone());
        let parsed = parse(vec![weird.clone()]).expect("non-UTF-8 positional");
        assert_eq!(parsed.cli.modules[0].as_os_str(), weird);
        assert_eq!(parsed.cli.modules[0].as_os_str().as_encoded_bytes(), raw);

        // The same applies to a path-valued option.
        let mut args = vec![OsString::from("--manifest")];
        args.push(weird.clone());
        args.push(OsString::from("D.aum"));
        let parsed = parse(args).expect("non-UTF-8 path value");
        assert_eq!(parsed.cli.manifest.unwrap().as_os_str(), weird);
    }

    #[test]
    fn empty_and_missing_selection_is_usage() {
        assert!(p(&[]).unwrap_err().0.contains("at least one .aum"));
        assert!(p(&[""]).unwrap_err().0.contains("empty module path"));
        // Selection is error, not an empty run.
        assert!(p(&["--json"]).unwrap_err().0.contains("at least one .aum"));
    }

    #[test]
    fn policy_sources_are_not_mixed() {
        let err = p(&["--project", "p.json", "--manifest", "m.toml", "D.aum"])
            .expect_err("two policy sources");
        assert!(err.0.contains("pass one"), "{err}");
    }

    #[test]
    fn build_options_are_explicit_and_keep_their_boundaries() {
        let cli = run(&[
            "--cargo-package",
            "app",
            "--target-dir",
            "/w/targets/custom",
            "--profile",
            "dev",
            "--features",
            "a b",
            "--features",
            "c",
            "--locked",
            "--offline",
            "D.aum",
        ]);
        assert_eq!(cli.cargo_package.as_deref(), Some("app"));
        assert_eq!(cli.target_dir, Some(PathBuf::from("/w/targets/custom")));
        assert_eq!(cli.profile, Profile::Dev);
        assert_eq!(cli.features, vec!["a b".to_string(), "c".to_string()]);
        assert!(cli.locked && cli.offline);
        assert!(!cli.liquid_required);
        assert!(run(&["--liquid-required", "D.aum"]).liquid_required);
        // The default stays the prototype's release build.
        assert_eq!(run(&["D.aum"]).profile, Profile::Release);
        assert_eq!(Profile::parse("release"), Ok(Profile::Release));
    }

    #[test]
    fn a_trust_record_path_is_single_valued_and_never_empty() {
        assert_eq!(
            run(&["--emit-trust-record", "pins.json", "D.aum"]).emit_trust_record,
            Some(PathBuf::from("pins.json"))
        );
        assert_eq!(
            run(&["--emit-trust-record=pins.json", "D.aum"]).emit_trust_record,
            Some(PathBuf::from("pins.json"))
        );
        assert!(p(&[
            "--emit-trust-record",
            "a.json",
            "--emit-trust-record",
            "b.json",
            "D.aum"
        ])
        .unwrap_err()
        .0
        .contains("duplicate option `--emit-trust-record`"));
        assert!(p(&["--emit-trust-record=", "D.aum"])
            .unwrap_err()
            .0
            .contains("non-empty path"));
    }

    #[test]
    fn build_option_errors_are_usage_errors() {
        for (args, needle) in [
            (vec!["--profile", "fast", "D.aum"], "unknown profile"),
            (
                vec!["--profile", "dev", "--profile", "dev", "D.aum"],
                "duplicate",
            ),
            (vec!["--cargo-package", "", "D.aum"], "must not be empty"),
            (vec!["--features", "", "D.aum"], "must not be empty"),
            (vec!["--locked=1", "D.aum"], "takes no value"),
            (vec!["--offline=1", "D.aum"], "takes no value"),
            (vec!["--target-dir", "", "D.aum"], "non-empty path"),
            (vec!["--features", "--locked", "D.aum"], "needs a value"),
        ] {
            let err = p(&args).unwrap_err();
            assert!(err.0.contains(needle), "{args:?} -> {err}");
        }
    }

    #[test]
    fn the_cache_directory_is_opt_in_and_single_valued() {
        assert_eq!(run(&["D.aum"]).cache_dir, None);
        assert_eq!(
            run(&["--cache-dir", "/w/cache", "D.aum"]).cache_dir,
            Some(PathBuf::from("/w/cache"))
        );
        assert!(p(&["--cache-dir", "", "D.aum"])
            .unwrap_err()
            .0
            .contains("non-empty path"));
        assert!(p(&["--cache-dir", "a", "--cache-dir", "b", "D.aum"])
            .unwrap_err()
            .0
            .contains("duplicate option `--cache-dir`"));
    }

    #[test]
    fn json_mode_is_a_flag_and_repeats_are_refused() {
        assert!(run(&["--json", "D.aum"]).json);
        assert!(p(&["--json", "--json", "D.aum"])
            .unwrap_err()
            .0
            .contains("duplicate"));
        assert!(p(&["--json=yes", "D.aum"])
            .unwrap_err()
            .0
            .contains("takes no value"));
    }

    #[test]
    fn descriptor_document_and_emit_wrappers_are_paired() {
        let cli = run(&[
            "--descriptor-document",
            "adapters.txt",
            "--emit-wrappers",
            "out",
        ]);
        assert_eq!(
            cli.descriptor_document.as_deref(),
            Some(std::path::Path::new("adapters.txt"))
        );
        assert_eq!(
            cli.emit_wrappers.as_deref(),
            Some(std::path::Path::new("out"))
        );
        assert!(cli.modules.is_empty(), "render-only needs no module");

        assert!(p(&["--descriptor-document", "adapters.txt"])
            .unwrap_err()
            .0
            .contains("used together"));
        assert!(p(&["--emit-wrappers", "out"])
            .unwrap_err()
            .0
            .contains("used together"));
        assert!(p(&[
            "--descriptor-document",
            "adapters.txt",
            "--emit-wrappers",
            "out",
            "D.aum",
        ])
        .unwrap_err()
        .0
        .contains("produces wrappers only"));
    }

    #[test]
    fn descriptor_document_can_build_wrappers_against_a_provider() {
        let cli = run(&[
            "--descriptor-document",
            "adapters.txt",
            "--build-wrappers",
            "out",
            "--provider",
            "libp.so",
        ]);
        assert_eq!(
            cli.build_wrappers.as_deref(),
            Some(std::path::Path::new("out"))
        );
        assert_eq!(
            cli.provider.as_deref(),
            Some(std::path::Path::new("libp.so"))
        );
        assert!(cli.emit_wrappers.is_none());

        // The build needs a provider to link; the render path links nothing and
        // refuses one, so neither flag silently ignores the other.
        assert!(p(&[
            "--descriptor-document",
            "adapters.txt",
            "--build-wrappers",
            "out",
        ])
        .unwrap_err()
        .0
        .contains("needs `--provider"));
        assert!(p(&[
            "--descriptor-document",
            "adapters.txt",
            "--emit-wrappers",
            "out",
            "--provider",
            "libp.so",
        ])
        .unwrap_err()
        .0
        .contains("only meaningful with `--build-wrappers`"));
        assert!(p(&[
            "--descriptor-document",
            "adapters.txt",
            "--emit-wrappers",
            "a",
            "--build-wrappers",
            "b",
            "--provider",
            "libp.so",
        ])
        .unwrap_err()
        .0
        .contains("pass exactly one"));
        assert!(p(&["--build-wrappers", "out", "--provider", "libp.so"])
            .unwrap_err()
            .0
            .contains("used together"));
        assert!(p(&["--provider", "libp.so", "D.aum"])
            .unwrap_err()
            .0
            .contains("only meaningful with `--build-wrappers`"));
    }
}
