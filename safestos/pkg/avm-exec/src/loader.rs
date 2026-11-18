//! Cell loader: gate a cell's own calls, then hand the linked image to an
//! executor that the runner spawns.
//!
//! The build path (§4.13) gates *builder* processes: a cell that may not be
//! built is refused with `EPERM` before the compiler exists. This is the
//! other half — the cell that will actually run. A `.avm` cell carries its
//! own Cedar capability table and the list of symbols it calls (its
//! `imports`, which the JIT resolves at load). Loading it therefore has two
//! separate questions to answer:
//!
//! 1. `Execute` — may this cell run at all?
//! 2. `Call:<symbol>` — may it call each symbol it declares, one request per
//!    symbol? A cell that may run but calls `uk_net_connect` without the
//!    grant for it is refused *at that call*, and the refusal names the
//!    symbol.
//!
//! Both are decided against the cell's **own** table and the policy set the
//! loader was given, before any relocation is applied and before any
//! process exists — the same "deny is the default" the runner enforces, and
//! the reason a refusal is a real `EPERM` rather than a warning.
//!
//! What "run" means here, stated plainly: this crate does not jump into
//! machine code (spec §6.3 — no `unsafe`). The linked image is rendered to
//! a JSON file in the runner's staging directory, and the *executor* the
//! caller names is spawned by [`Runner::run`] in the granted namespaces
//! with a scrubbed environment. So the process that maps and executes the
//! image is sandboxed and its input is a file the loader wrote, while the
//! call gate above is what makes a forbidden call impossible to reach.

use crate::cell_builder::{CellError, CellView};
use crate::linker::{link_cells, LinkedImage, Provider};
use crate::runner::{Runner, RunnerError, Sandbox};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

/// Loader failures. `Denied` is the capability refusal and it *is* an
/// `EPERM` (`as_io_error` says so), exactly like the runner's own denial.
#[derive(Debug)]
pub enum LoadError {
    Cell(CellError),
    /// The policy set or the cell's capability table would not parse.
    Cedar(String),
    /// Cedar refused `action` for this cell's table.
    Denied(String),
    Runner(RunnerError),
    Io(std::io::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Cell(e) => write!(f, "{e}"),
            LoadError::Cedar(m) => write!(f, "cedar: {m}"),
            LoadError::Denied(a) => write!(f, "EPERM: capability denied for action {a:?}"),
            LoadError::Runner(e) => write!(f, "{e}"),
            LoadError::Io(e) => write!(f, "loader i/o: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<CellError> for LoadError {
    fn from(e: CellError) -> Self {
        LoadError::Cell(e)
    }
}

impl From<RunnerError> for LoadError {
    fn from(e: RunnerError) -> Self {
        LoadError::Runner(e)
    }
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

/// A provider cell in a multi-cell image: its own relocated text and the
/// base it was linked for.
#[derive(Debug)]
pub struct LoadedProvider {
    pub base: u64,
    pub image: LinkedImage,
}

/// A cell that passed the call gate and has been relocated.
#[derive(Debug)]
pub struct LoadedImage {
    /// The distinct symbols the cell declares calls to, sorted.
    pub calls: Vec<String>,
    /// The relocated image, as if mapped at the requested base.
    pub image: LinkedImage,
    pub base: u64,
    pub drv_hash: [u8; 32],
    /// Provider cells, in the order given, each relocated against the same
    /// symbol table and gated by its own capability table.
    pub providers: Vec<LoadedProvider>,
    /// The cell's own capability table, kept so the run step judges with the
    /// same table the load step did — never a second copy read from disk.
    pub capability_table: serde_json::Value,
}

/// Gate one cell's `Execute` and each declared call against its own table.
/// Returns the sorted, de-duplicated call list.
fn gate(view: &CellView<'_>, policy_src: &str) -> Result<Vec<String>, LoadError> {
    let sandbox =
        Sandbox::new(view.capability_table.clone(), policy_src).map_err(LoadError::Cedar)?;

    // Execute first: a cell that may not run has no calls to judge, and the
    // refusal should name the thing that was actually asked for.
    if !sandbox.check("Execute") {
        return Err(LoadError::Denied("Execute".to_string()));
    }

    let mut calls: Vec<String> = view.imports.clone();
    calls.sort();
    calls.dedup();
    for call in &calls {
        let action = format!("Call:{call}");
        if !sandbox.check(&action) {
            return Err(LoadError::Denied(action));
        }
    }
    Ok(calls)
}

/// Gate a cell's declared calls, then relocate it.
///
/// `externs` is the runtime symbol table the image is linked against — for
/// a kernel-calling cell, the addresses the JIT registered its `uk_*`
/// symbols at. An import with no reloc is still a call and is still gated.
pub fn load(
    cell_bytes: &[u8],
    policy_src: &str,
    base: u64,
    externs: &BTreeMap<String, u64>,
) -> Result<LoadedImage, LoadError> {
    load_with(cell_bytes, &[], policy_src, base, externs)
}

/// [`load`] for a multi-cell image: `providers` are `(cell bytes, base)`
/// pairs, already mapped-at-their-real-addresses in the executor's process.
///
/// Each provider is gated by *its own* capability table — a provider runs in
/// the same process, so its calls are as reachable as the owner's — and its
/// text is relocated against the shared table (a provider may call the owner
/// or the runtime). All addresses are assigned by the caller; nothing here
/// allocates or maps, so the executor can reproduce the layout exactly from
/// the rendered image.
pub fn load_with(
    cell_bytes: &[u8],
    providers: &[(&[u8], u64)],
    policy_src: &str,
    base: u64,
    externs: &BTreeMap<String, u64>,
) -> Result<LoadedImage, LoadError> {
    let view = CellView::parse(cell_bytes)?;
    let calls = gate(&view, policy_src)?;

    let mut provider_views = Vec::with_capacity(providers.len());
    for (bytes, provider_base) in providers {
        let pview = CellView::parse(bytes)?;
        gate(&pview, policy_src)?;
        provider_views.push((pview, *provider_base));
    }
    let refs: Vec<Provider<'_, '_>> = provider_views
        .iter()
        .map(|(v, b)| Provider { view: v, base: *b })
        .collect();

    let linked = link_cells(&view, base, &refs, externs)?;
    let loaded_providers = linked
        .providers
        .into_iter()
        .zip(providers.iter().map(|(_, b)| *b))
        .map(|(image, base)| LoadedProvider { base, image })
        .collect();

    Ok(LoadedImage {
        calls,
        image: linked.owner,
        base,
        drv_hash: view.drv_hash,
        providers: loaded_providers,
        capability_table: view.capability_table.clone(),
    })
}

/// The image file the executor reads — the linked text plus the addresses
/// it was linked at, so the executor never has to guess either.
pub fn render_image(loaded: &LoadedImage) -> Vec<u8> {
    let hex = |b: &[u8]| -> String {
        let mut s = String::with_capacity(b.len() * 2);
        for x in b {
            s.push_str(&format!("{x:02x}"));
        }
        s
    };
    let symbols: BTreeMap<&String, String> = loaded
        .image
        .symbols
        .iter()
        .map(|(k, v)| (k, format!("{v:#x}")))
        .collect();
    let providers: Vec<serde_json::Value> = loaded
        .providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "base": format!("{:#x}", p.base),
                "text": hex(&p.image.text),
            })
        })
        .collect();
    let doc = serde_json::json!({
        "drv_hash": hex(&loaded.drv_hash),
        "base": format!("{:#x}", loaded.base),
        "text": hex(&loaded.image.text),
        "symbols": symbols,
        "calls": loaded.calls,
        "providers": providers,
    });
    let mut out = serde_json::to_vec_pretty(&doc).expect("image JSON is serializable");
    out.push(b'\n');
    out
}

/// Hand the linked image to `exec` inside the runner's sandbox.
///
/// The runner is built from `loaded`'s own capability table and the same
/// policy text the load step used, and authorizes `Execute` again before
/// spawning — the check is cheap and the second read is what makes "the
/// refusal happened before the process existed" true at the spawn site, not
/// only at the entry point.
///
/// The image is staged as `cell.json` in a fresh directory that becomes the
/// child's cwd (so `exec` must be an absolute path — the same constraint the
/// gated build has), and the child's environment is scrubbed except for
/// PATH/HOME/TMPDIR and the image's derivation hash.
pub fn run_image(
    exec: &Path,
    policy_src: &str,
    loaded: &LoadedImage,
) -> Result<std::process::Output, LoadError> {
    let sandbox =
        Sandbox::new(loaded.capability_table.clone(), policy_src).map_err(LoadError::Cedar)?;
    let runner = Runner::new(sandbox);
    let image = render_image(loaded);
    let drv = loaded
        .drv_hash
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let args: Vec<OsString> = vec!["--image".into(), "cell.json".into()];
    let out = runner.run_with_env(
        "Execute",
        exec,
        &args,
        &[("cell.json", &image)],
        &[("AVM_IMAGE_DRV".to_string(), drv)],
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_builder::{build_cell, AvceDoc, AvceFn, AvceReloc, BuildCell};

    fn cell(imports: &[&str], relocs: Vec<AvceReloc>) -> Vec<u8> {
        let doc = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "run".into(),
                // `mov eax, 7; ret` and padding, so a reloc at offset 1 has a
                // full 8-byte slot to write into.
                body: vec![0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3, 0x90, 0x90, 0x90],
                relocs,
            }],
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        let caps = serde_json::json!([
            { "uid": { "type": "ExecCap", "id": "exec" }, "attrs": {}, "parents": [] }
        ]);
        build_cell(&BuildCell {
            drv_hash: [0x11u8; 32],
            capability_table: &caps,
            avce: &doc,
            signing_seed: &[5u8; 32],
        })
        .unwrap()
    }

    fn externs(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    /// The first question: a cell that may not run is refused before its
    /// calls are even looked at, and the refusal is an EPERM naming Execute.
    #[test]
    fn a_cell_that_may_not_run_is_refused_with_eperm() {
        let bytes = cell(&["uk_version"], vec![]);
        let err = load(
            &bytes,
            "forbid(principal, action, resource);",
            0,
            &externs(&[]),
        )
        .expect_err("a forbid-all policy must refuse the load");
        assert_eq!(
            err.to_string(),
            "EPERM: capability denied for action \"Execute\""
        );
        let io = match err {
            LoadError::Denied(_) => std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            other => panic!("want Denied, got {other:?}"),
        };
        assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
    }

    /// The second question, per call: running is permitted, one symbol is
    /// not, and the refusal names that symbol rather than the cell.
    #[test]
    fn a_denied_call_is_refused_by_name() {
        let bytes = cell(&["uk_version", "uk_net_connect"], vec![]);
        let policy = "permit(principal, action, resource);\n\
                      forbid(principal, action == Action::\"Call:uk_net_connect\", resource);";
        let err = load(&bytes, policy, 0, &externs(&[])).expect_err("the call must be refused");
        assert_eq!(
            err.to_string(),
            "EPERM: capability denied for action \"Call:uk_net_connect\""
        );
    }

    /// Permitted calls are judged individually — allowing one symbol is not
    /// allowing the rest — and the image links against the runtime table.
    #[test]
    fn permitted_calls_are_gated_one_by_one_and_linked() {
        let bytes = cell(
            &["uk_version"],
            vec![AvceReloc {
                offset: 1,
                addend: 0,
                kind: "Abs8".into(),
                symbol: "uk_version".into(),
            }],
        );
        let only_version = "permit(principal, action, resource);\n\
                            forbid(principal, action == Action::\"Call:uk_net_connect\", resource);";
        let loaded = load(
            &bytes,
            only_version,
            0x1000,
            &externs(&[("uk_version", 0xdead_0000)]),
        )
        .expect("Execute and Call:uk_version are permitted");
        assert_eq!(loaded.calls, vec!["uk_version".to_string()]);
        assert_eq!(loaded.base, 0x1000);
        // The Abs8 site now carries the runtime address, little-endian.
        assert_eq!(
            &loaded.image.text[1..9],
            &0xdead_0000u64.to_le_bytes(),
            "the import resolved to the JIT's registered address"
        );

        // With no runtime entry the same load is a hard link error — never a
        // silent zero, and never a "close enough" refusal.
        let err = load(&bytes, only_version, 0, &externs(&[])).expect_err("unresolved import");
        match err {
            LoadError::Cell(CellError::Link(m)) => {
                assert!(m.contains("unresolved symbol"), "got: {m}");
                assert!(m.contains("uk_version"), "got: {m}");
            }
            other => panic!("want a link error, got {other:?}"),
        }
    }

    /// A provider cell supplies an import the owner's own text calls, and
    /// both images reach the executor through `cell.json`.
    #[test]
    fn a_provider_cell_resolves_an_import_and_is_rendered() {
        let owner = cell(
            &["give"],
            vec![AvceReloc {
                offset: 1,
                addend: 0,
                kind: "Abs8".into(),
                symbol: "give".into(),
            }],
        );
        // Provider: `mov eax, 7; ret` under the imported name.
        let provider_doc = AvceDoc {
            arch: "x86_64".into(),
            functions: vec![AvceFn {
                name: "give".into(),
                body: vec![0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3],
                relocs: vec![],
            }],
            imports: vec![],
        };
        let provider = build_cell(&BuildCell {
            drv_hash: [0x22u8; 32],
            capability_table: &serde_json::json!([]),
            avce: &provider_doc,
            signing_seed: &[7u8; 32],
        })
        .unwrap();

        let loaded = load_with(
            &owner,
            &[(&provider, 0x2000)],
            "permit(principal, action, resource);",
            0x1000,
            &externs(&[]),
        )
        .expect("the provider defines the import");
        assert_eq!(&loaded.image.text[1..9], &0x2000u64.to_le_bytes());
        assert_eq!(loaded.providers.len(), 1);
        assert_eq!(loaded.providers[0].base, 0x2000);
        assert_eq!(loaded.providers[0].image.text.len(), 6);

        let doc = String::from_utf8(render_image(&loaded)).unwrap();
        assert!(doc.contains("\"providers\""), "{doc}");
        assert!(doc.contains("\"base\": \"0x2000\""), "{doc}");
        // The provider's own text is in the document, not just its symbols.
        assert!(doc.contains("b807000000c3"), "{doc}");

        // Without the provider the same load is a hard link error.
        let err = load(
            &owner,
            "permit(principal, action, resource);",
            0x1000,
            &externs(&[]),
        )
        .expect_err("give is unresolved without its provider");
        assert!(err.to_string().contains("give"), "{err}");
    }

    /// A provider is gated by its own table and call list: it runs in the
    /// same process, so a denied call there is refused by name too.
    #[test]
    fn a_provider_call_is_gated_by_its_own_table() {
        let owner = cell(&[], vec![]);
        let provider = cell(&["uk_net_connect"], vec![]);
        let policy = "permit(principal, action, resource);\n\
                      forbid(principal, action == Action::\"Call:uk_net_connect\", resource);";
        let err = load_with(
            &owner,
            &[(&provider, 0x2000)],
            policy,
            0x1000,
            &externs(&[]),
        )
        .expect_err("the provider's call must be refused");
        assert_eq!(
            err.to_string(),
            "EPERM: capability denied for action \"Call:uk_net_connect\""
        );
    }

    /// The image reaches the executor through the runner: staged file, cwd
    /// in the staging dir, scrubbed environment, derivation hash supplied.
    #[test]
    fn the_executor_sees_the_image_through_the_runner() {
        let bytes = cell(&[], vec![]);
        let policy = "permit(principal, action, resource);";
        let loaded = load(&bytes, policy, 0, &externs(&[])).expect("no calls to refuse");
        assert!(loaded.calls.is_empty());

        let dir = tempfile::tempdir().unwrap();
        let exec = dir.path().join("executor.sh");
        std::fs::write(
            &exec,
            "#!/bin/sh\n\
             echo \"argv=$*\"\n\
             echo \"drv=$AVM_IMAGE_DRV\"\n\
             echo \"host_env=${HOME:-none}\"\n\
             grep -o '\"drv_hash\": \"[0-9a-f]*\"' cell.json\n",
        )
        .unwrap();
        std::fs::set_permissions(&exec, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let out = run_image(&exec, policy, &loaded).expect("the executor runs");
        assert!(out.status.success(), "executor failed: {out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("argv=--image cell.json"), "got: {stdout}");
        assert!(
            stdout.contains(&format!("drv={}", "11".repeat(32))),
            "the derivation hash reaches the child: {stdout}"
        );
        assert!(
            stdout.contains(&format!("\"drv_hash\": \"{}\"", "11".repeat(32))),
            "the image is staged where the loader said: {stdout}"
        );
        // HOME is the staging dir, not the host's — the env was scrubbed.
        assert!(
            !stdout.contains("host_env=/home"),
            "host env leaked: {stdout}"
        );
    }
}
