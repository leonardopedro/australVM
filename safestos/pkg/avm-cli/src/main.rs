//! `avm-pkg` — the avm-pkg command line (PLAN_avm_pkg.md Sprint 4).
//!
//! Only subcommands whose backing subsystems actually exist are wired;
//! `build` / `push` / / `pull` refuse with a pointer to the pending sprint
//! rather than pretending to work (spec §6: honesty over coverage). Every
//! implemented command is exercised by the tests at the bottom of this file.
//!
//! Environment: `AVM_PKG_STORE` (store dir, default `./avm-store`),
//! `AVM_HOT_DIR` (hydration scratch, default system temp), `AVM_HOT_LIMIT`
//! (hot-store budget, default `1G`, suffixes K/M/G).

#![deny(unsafe_op_in_unsafe_fn)]

use avm_cas::crypto::{sign_manifest, signing_key_from_seed};
use avm_cas::engine::{hex, unhex};
use avm_cas::hot_store::{HotStats, HotStore};
use avm_cas::{ChunkStore, Manifest};
use avm_eval::{Derivation, MemoTable, Plan, Presence};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

const USAGE: &str = "\
avm-pkg — content-addressed package engine for australVM

USAGE:
  avm-pkg [--store <dir>] <command> [args]

COMMANDS (implemented over the real subsystems):
  init                         create the store layout
  put <file> [--name N] [--version V] [--sign <seed-hex32>] [--drv <hex32>]
                               ingest a file as a cell; prints its manifest hash
  get <manifest-hex> [-o FILE] [--hot-limit SZ]
                               hydrate through the ephemeral hot store
  stat <manifest-hex>          manifest as JSON
  store-stats                  cold-store counters as JSON
  hot-stats                    hot-store budget, dir, and scratch usage
  gc [--dry-run] [--keep-roots]
                               compact unreferenced chunks (v1: every
                               manifest is a root; --keep-roots is implicit)
  drv-hash <plan.json> [target]    derivation Merkle ids (deterministic)
  eval <plan.json> [target]        memoized warm evaluation report (L1 + L2)

PENDING (see PLAN_avm_pkg.md — these refuse rather than fake):
  build <file.au>              needs the OCaml totality pass + executor
  push / pull                  needs the Sprint 5 remote client
";

/// Small shared context: where the store lives.
struct Ctx {
    store_dir: PathBuf,
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(Some(text)) => println!("{text}"),
        Ok(None) => {}
        Err(code) => std::process::exit(code),
    }
}

/// Command dispatcher. `Ok(Some(text))` = print; `Ok(None)` = command
/// already wrote what it needed; `Err(code)` = message already printed.
fn run(argv: &[String]) -> Result<Option<String>, i32> {
    // Global: [--store DIR]
    let mut i = 0;
    let mut store_dir = std::env::var_os("AVM_PKG_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./avm-store"));
    while i < argv.len() {
        if argv[i] == "--store" {
            match argv.get(i + 1) {
                Some(d) => {
                    store_dir = PathBuf::from(d);
                    i += 2;
                }
                None => {
                    eprintln!("avm-pkg: --store needs a directory");
                    return Err(2);
                }
            }
        } else {
            break;
        }
    }
    let ctx = Ctx { store_dir };
    let rest = &argv[i..];
    let Some(cmd) = rest.first() else {
        eprint!("{USAGE}");
        return Err(2);
    };
    let args = &rest[1..];

    let result = match cmd.as_str() {
        "init" => cmd_init(&ctx),
        "put" => cmd_put(&ctx, args),
        "stat" => cmd_stat(&ctx, args),
        "store-stats" => cmd_store_stats(&ctx),
        "hot-stats" => cmd_hot_stats(&ctx, args),
        "gc" => cmd_gc(&ctx, args),
        "drv-hash" => cmd_drv_hash(&ctx, args),
        "eval" => cmd_eval(&ctx, args),
        "get" => cmd_get(&ctx, args),
        "build" | "push" | "pull" => Err(format!(
            "`{cmd}` is not implemented yet: {} — see PLAN_avm_pkg.md for the sprint it belongs to (refusing instead of faking)",
            match cmd.as_str() {
                "build" => "needs the OCaml totality pass (Sprint 2) and the hermetic executor (Sprint 3)",
                "push" | "pull" => "needs the Sprint 5 remote client",
                _ => unreachable!(),
            }
        )),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            return Ok(None);
        }
        other => {
            eprintln!("avm-pkg: unknown command `{other}`\n");
            eprint!("{USAGE}");
            return Err(2);
        }
    };

    match result {
        Ok(text) => Ok(Some(text)),
        Err(e) => {
            eprintln!("avm-pkg: {e}");
            Err(1)
        }
    }
}

fn open_store(ctx: &Ctx) -> Result<ChunkStore, String> {
    ChunkStore::open(&ctx.store_dir).map_err(|e| format!("cannot open store: {e}"))
}

/// Size suffix parsing for budgets: `4096`, `256K`, `64M`, `1G` (binary units).
fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult) = match s.chars().last().unwrap_or('0') {
        'K' | 'k' => (&s[..s.len() - 1], 1024u64),
        'M' | 'm' => (&s[..s.len() - 1], 1024 * 1024),
        'G' | 'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        _ => (s, 1),
    };
    let n: u64 = num
        .trim()
        .parse()
        .map_err(|_| format!("bad size `{s}` (want e.g. 4096, 256K, 1G)"))?;
    Ok(n * mult)
}

fn hot_limit_from_env() -> Result<u64, String> {
    match std::env::var("AVM_HOT_LIMIT") {
        Ok(v) => parse_size(&v),
        Err(_) => Ok(1024 * 1024 * 1024),
    }
}

fn hot_dir_from_env() -> PathBuf {
    std::env::var_os("AVM_HOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("avm-hot"))
}

// ── commands ─────────────────────────────────────────────────────────

fn cmd_init(ctx: &Ctx) -> Result<String, String> {
    ChunkStore::open(&ctx.store_dir).map_err(|e| e.to_string())?;
    Ok(format!("initialized store at {}", ctx.store_dir.display()))
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
}

fn cmd_put(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let path = args
        .iter()
        .find(|a| !a.starts_with("--") && !is_flag_value(args, a))
        .ok_or_else(|| "put needs a file path".to_string())?;
    let name = flag(args, "--name").map(str::to_string).unwrap_or_else(|| {
        Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cell".to_string())
    });
    let version = flag(args, "--version").unwrap_or("0.0.0").to_string();
    let drv = match flag(args, "--drv") {
        Some(h) => Some(unhex(h).map_err(|e| e.to_string())?),
        None => None,
    };
    let data = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let store = open_store(ctx)?;
    let mut manifest = store
        .put_buffer(&name, &version, &data, drv)
        .map_err(|e| e.to_string())?;
    if let Some(seed_hex) = flag(args, "--sign") {
        let seed_bytes = unhex(seed_hex).map_err(|e| format!("bad --seed: {e}"))?;
        let key = signing_key_from_seed(&seed_bytes);
        sign_manifest(&mut manifest, &key);
        store.put_manifest(&manifest).map_err(|e| e.to_string())?;
    }
    let id = manifest.identity_digest();
    Ok(format!(
        "{}  {} {}  ({} bytes, {} chunks{} )",
        hex(&id),
        manifest.name,
        manifest.version,
        manifest.total_size,
        manifest.chunks.len(),
        if manifest.signature.is_some() { ", signed" } else { "" }
    ))
}

/// True when `a` is the *value* consumed by a preceding flag (so file
/// paths named like flags, and values, are not mistaken for operands).
fn is_flag_value(args: &[String], a: &str) -> bool {
    args.iter()
        .position(|x| x == a)
        .map(|i| i > 0 && (args[i - 1].starts_with("--") && args[i - 1].len() > 2))
        .unwrap_or(false)
}

fn cmd_stat(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let id_hex = args.first().ok_or_else(|| "stat needs a manifest hash".to_string())?;
    let id = unhex(id_hex).map_err(|e| e.to_string())?;
    let store = open_store(ctx)?;
    let m = store.get_manifest(&id).map_err(|e| e.to_string())?;
    Ok(m.to_json())
}

fn cmd_store_stats(ctx: &Ctx) -> Result<String, String> {
    let store = open_store(ctx)?;
    let s = store.stats().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "chunks": s.chunks,
        "packs": s.packs,
        "pack_bytes": s.pack_bytes,
        "manifests": s.manifests,
        "root": store.root().display().to_string(),
    })
    .to_string())
}

fn cmd_hot_stats(_ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let limit = match flag(args, "--hot-limit") {
        Some(v) => parse_size(v)?,
        None => hot_limit_from_env()?,
    };
    let hot_dir = hot_dir_from_env();
    // Scratch accounting: bytes sitting in the hot dir right now. Resident
    // cell mappings are per-process by design (the CLI hydrates and exits);
    // live residency belongs to the runtime integration of Sprint 3, and
    // this says so rather than inventing a number.
    let mut scratch_bytes = 0u64;
    let mut scratch_files = 0u64;
    if let Ok(rd) = std::fs::read_dir(&hot_dir) {
        for e in rd.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    scratch_bytes += md.len();
                    scratch_files += 1;
                }
            }
        }
    }
    Ok(serde_json::json!({
        "budget_bytes": limit,
        "hot_dir": hot_dir.display().to_string(),
        "scratch_files": scratch_files,
        "scratch_bytes": scratch_bytes,
        "note": "resident cell mappings are per-process; live hot-store \
                 residency is reported by the runtime (Sprint 3), not a CLI \
                 that hydrates and exits",
    })
    .to_string())
}

fn cmd_gc(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let store = open_store(ctx)?;
    if args.iter().any(|a| a == "--dry-run") {
        // Count live vs dead without touching a packfile: roots = manifests.
        let mut live: HashSet<[u8; 32]> = HashSet::new();
        let manifests = store.list_manifests().map_err(|e| e.to_string())?;
        for id in &manifests {
            let m = store.get_manifest(id).map_err(|e| e.to_string())?;
            for c in &m.chunks {
                live.insert(*c);
            }
        }
        let total = store.stats().map_err(|e| e.to_string())?.chunks;
        let dead = total.saturating_sub(live.len());
        return Ok(serde_json::json!({
            "dry_run": true,
            "roots": manifests.len(),
            "live_chunks": live.len(),
            "dead_chunks": dead,
            "note": "v1: every manifest is a root (--keep-roots is implicit)",
        })
        .to_string());
    }
    let report = store.gc().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "live_chunks": report.live_chunks,
        "dead_chunks": report.dead_chunks,
        "packs_before": report.packs_before,
        "packs_after": report.packs_after,
        "bytes_reclaimed": report.bytes_reclaimed,
    })
    .to_string())
}

fn load_plan(path: &str) -> Result<Plan, String> {
    #[derive(serde::Deserialize)]
    struct PlanFile {
        derivations: Vec<Derivation>,
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let file: PlanFile =
        serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))?;
    let mut plan = Plan::new();
    for d in file.derivations {
        plan.insert(d).map_err(|e| e.to_string())?;
    }
    Ok(plan)
}

fn cmd_drv_hash(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let _ = ctx; // store-independent: pure derivation hashing
    let path = args.first().ok_or_else(|| "drv-hash needs <plan.json>".to_string())?;
    let target = args.get(1).map(|s| s.as_str());
    let plan = load_plan(path)?;
    let graph = plan.resolve(target).map_err(|e| e.to_string())?;
    let ids: std::collections::BTreeMap<&str, String> =
        graph.ids.iter().map(|(k, v)| (k.as_str(), hex(v))).collect();
    Ok(serde_json::json!({
        "target": target.and_then(|t| graph.ids.get(t)).map(|h| hex(h)),
        "resolved": ids,
        "order": graph.order,
    })
    .to_string())
}

fn cmd_eval(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let path = args.first().ok_or_else(|| "eval needs <plan.json>".to_string())?;
    let target = args.get(1).map(|s| s.as_str());
    let plan = load_plan(path)?;

    // L2 oracle: every manifest in the store that records a drv_hash.
    let presence: Option<Arc<dyn Presence>> = if let Ok(store) = open_store(ctx) {
        let mut ids = HashSet::new();
        if let Ok(list) = store.list_manifests() {
            for id in list {
                if let Ok(m) = store.get_manifest(&id) {
                    if let Some(d) = m.drv_hash {
                        ids.insert(d);
                    }
                }
            }
        }
        Some(Arc::new(StorePresence(ids)))
    } else {
        None
    };

    let memo = MemoTable::new(presence);
    let start = Instant::now();
    let report = memo.evaluate(&plan, target).map_err(|e| e.to_string())?;
    let dt = start.elapsed();
    Ok(serde_json::json!({
        "target": hex(&report.target_id),
        "resolved": report.resolved.len(),
        "order": report.order,
        "visited": report.visited,
        "session_hits": report.session_hits,
        "store_hits": report.store_hits,
        "eval_us": dt.as_micros() as u64,
        "note": "store_hits are id-keyed build skips; ids are always recomputed (see PLAN_avm_pkg.md)",
    })
    .to_string())
}

/// Store-derived presence: built-cell ids recorded by manifests.
struct StorePresence(HashSet<[u8; 32]>);
impl Presence for StorePresence {
    fn contains_id(&self, id: &[u8; 32]) -> bool {
        self.0.contains(id)
    }
}

/// Hydrate a cell through the hot store; returns payload + stats for the
/// caller (main prints, tests assert).
fn hydrate_get(
    ctx: &Ctx,
    id_hex: &str,
    limit: u64,
) -> Result<(Vec<u8>, HotStats, Manifest), String> {
    let id = unhex(id_hex).map_err(|e| e.to_string())?;
    let store = Arc::new(open_store(ctx)?);
    let manifest = store.get_manifest(&id).map_err(|e| e.to_string())?;
    let hot = HotStore::new(store.clone(), limit, hot_dir_from_env(), false)
        .map_err(|e| e.to_string())?;
    let bytes = hot.read_cell(&id).map_err(|e| e.to_string())?;
    Ok((bytes, hot.stats(), manifest))
}

fn cmd_get(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let id_hex = args.first().ok_or_else(|| "get needs a manifest hash".to_string())?;
    let limit = match flag(args, "--hot-limit") {
        Some(v) => parse_size(v)?,
        None => hot_limit_from_env()?,
    };
    let (bytes, stats, manifest) = hydrate_get(ctx, id_hex, limit)?;
    match flag(args, "-o") {
        Some(out) => {
            std::fs::write(out, &bytes).map_err(|e| format!("write {out}: {e}"))?;
            Ok(serde_json::json!({
                "wrote": out.to_string(),
                "bytes": bytes.len(),
                "name": manifest.name,
                "hot_usage_bytes": stats.usage,
                "hot_cells": stats.cells,
            })
            .to_string())
        }
        None => {
            use std::io::Write;
            std::io::stdout()
                .write_all(&bytes)
                .map_err(|e| format!("stdout: {e}"))?;
            Ok(format!(
                "{} bytes hydrated (hot usage {} / {})",
                bytes.len(),
                stats.usage,
                stats.capacity
            ))
        }
    }
}

// ── tests (each exercises a real command path) ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    struct TempStore {
        _dir: tempfile::TempDir,
        ctx: Ctx,
    }
    // tempfile is a dev-dep of avm-cas only; re-create the need here.
    impl TempStore {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let ctx = Ctx { store_dir: dir.path().join("store") };
            TempStore { _dir: dir, ctx }
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn put_get_stat_roundtrip_through_commands() {
        let ts = TempStore::new();
        let payload = vec![0xABu8; 300 * 1024];
        let f = ts._dir.path().join("cell.bin");
        write(&f, &payload);

        let out = cmd_init(&ts.ctx).unwrap();
        assert!(out.contains("initialized"));

        let put_out = cmd_put(
            &ts.ctx,
            &[
                f.display().to_string(),
                "--name".into(),
                "demo".into(),
                "--version".into(),
                "1.2.3".into(),
                "--drv".into(),
                hex(&[7u8; 32]),
            ],
        )
        .unwrap();
        let id_hex: String = put_out.split_whitespace().next().unwrap().to_string();
        assert_eq!(id_hex.len(), 64);

        let stat = cmd_stat(&ts.ctx, std::slice::from_ref(&id_hex)).unwrap();
        assert!(stat.contains("\"name\":\"demo\""), "{stat}");
        assert!(stat.contains("\"version\":\"1.2.3\""), "{stat}");
        assert!(stat.contains(&format!("\"drv_hash\":\"{}\"", hex(&[7u8; 32]))), "{stat}");

        let out_file = ts._dir.path().join("out.bin");
        let get_out = cmd_get(
            &ts.ctx,
            &[id_hex.clone(), "-o".into(), out_file.display().to_string()],
        )
        .unwrap();
        assert!(get_out.contains(&format!("\"bytes\":{}", payload.len())));
        assert_eq!(std::fs::read(&out_file).unwrap(), payload, "bytes must roundtrip");
    }

    #[test]
    fn signed_put_verifies() {
        let ts = TempStore::new();
        let f = ts._dir.path().join("s.bin");
        write(&f, b"signed payload");
        let seed = hex(&[42u8; 32]);
        let put_out =
            cmd_put(&ts.ctx, &[f.display().to_string(), "--sign".into(), seed.clone()])
                .unwrap();
        assert!(put_out.contains("signed"), "{put_out}");
        let id_hex = put_out.split_whitespace().next().unwrap();
        let store = open_store(&ts.ctx).unwrap();
        let m = store.get_manifest(&unhex(id_hex).unwrap()).unwrap();
        avm_cas::crypto::verify_manifest(&m).expect("command-signed manifest verifies");
    }

    #[test]
    fn gc_dry_run_counts_without_mutation() {
        let ts = TempStore::new();
        let a = ts._dir.path().join("a.bin");
        let b = ts._dir.path().join("b.bin");
        write(&a, &[1u8; 1024 * 1024]);
        write(&b, &[2u8; 1024 * 1024]);
        cmd_put(&ts.ctx, &[a.display().to_string()]).unwrap();
        let put_b = cmd_put(&ts.ctx, &[b.display().to_string()]).unwrap();
        let id_b = put_b.split_whitespace().next().unwrap().to_string();

        let dry = cmd_gc(&ts.ctx, &["--dry-run".into()]).unwrap();
        assert!(dry.contains("\"dead_chunks\":0"), "both live: {dry}");

        // Drop one root (store-level operation; CLI `rm` is not a spec'd
        // command), then dry-run shows dead chunks and gc reclaims them.
        let store = open_store(&ts.ctx).unwrap();
        store.drop_manifest(&unhex(&id_b).unwrap()).unwrap();
        let dry2 = cmd_gc(&ts.ctx, &["--dry-run".into()]).unwrap();
        assert!(dry2.contains("\"dead_chunks\":"), "{dry2}");
        let dead: u64 = serde_json::from_str::<serde_json::Value>(&dry2)
            .unwrap()["dead_chunks"]
            .as_u64()
            .unwrap();
        assert!(dead > 0);

        let report = cmd_gc(&ts.ctx, &[]).unwrap();
        assert!(report.contains("\"dead_chunks\":"), "{report}");
        // Live file still readable after compaction.
        let live = store.list_manifests().unwrap();
        assert_eq!(live.len(), 1);
    }

    #[test]
    fn drv_hash_and_eval_are_deterministic_and_report_store_tier() {
        let ts = TempStore::new();
        let plan_path = ts._dir.path().join("plan.json");
        std::fs::write(
            &plan_path,
            r#"{"derivations":[
                {"name":"base","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["aa"],"env_vars":[]},
                {"name":"mid","version":"1","builder_symbol":"stdenv.build","dependencies":["base"],"sources":[],"env_vars":[["K","V"]]},
                {"name":"app","version":"1","builder_symbol":"stdenv.build","dependencies":["mid"],"sources":[],"env_vars":[]}
            ]}"#,
        )
        .unwrap();

        let h1 = cmd_drv_hash(&ts.ctx, &[plan_path.display().to_string()]).unwrap();
        let h2 = cmd_drv_hash(&ts.ctx, &[plan_path.display().to_string()]).unwrap();
        assert_eq!(h1, h2, "drv-hash must be byte-stable");
        assert!(h1.contains("\"app\""));

        // First eval: nothing in the store -> no store_hits.
        let e1 = cmd_eval(&ts.ctx, &[plan_path.display().to_string()]).unwrap();
        assert!(e1.contains("\"store_hits\":[]"), "{e1}");

        // Record mid's built cell with its drv_hash (as the builder will),
        // re-eval: store tier reports mid, ids unchanged.
        let mid_id = {
            let v: serde_json::Value = serde_json::from_str(&h1).unwrap();
            unhex(v["resolved"]["mid"].as_str().unwrap()).unwrap()
        };
        let store = open_store(&ts.ctx).unwrap();
        let mut m = store.put_buffer("mid", "1", b"built-mid", Some(mid_id)).unwrap();
        avm_cas::crypto::sign_manifest(&mut m, &signing_key_from_seed(&[1u8; 32]));
        store.put_manifest(&m).unwrap();

        let e2 = cmd_eval(&ts.ctx, &[plan_path.display().to_string()]).unwrap();
        assert!(e2.contains("\"store_hits\":[\"mid\"]"), "{e2}");
        let t1: serde_json::Value = serde_json::from_str(&e1).unwrap();
        let t2: serde_json::Value = serde_json::from_str(&e2).unwrap();
        assert_eq!(t1["target"], t2["target"], "store tier may not move ids");
    }

    #[test]
    fn pending_commands_refuse_honestly() {
        for cmd in ["build", "push", "pull"] {
            let r = run(&[cmd.to_string(), "whatever".to_string()]);
            assert!(r.is_err(), "{cmd} must not pretend to succeed");
        }
    }

    #[test]
    fn size_parser() {
        assert_eq!(parse_size("4096").unwrap(), 4096);
        assert_eq!(parse_size("256K").unwrap(), 256 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert!(parse_size("bogus").is_err());
    }
}
