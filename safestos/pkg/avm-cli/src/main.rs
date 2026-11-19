//! `avm-pkg` — the avm-pkg command line (PLAN_avm_pkg.md Sprint 4).
//!
//! Every spec subcommand is wired to its backing subsystem; a command whose
//! input cannot be produced honestly (an uncompilable CPS file, a remote
//! that does not answer) fails loudly instead of faking a result (spec §6:
//! honesty over coverage). Every command is exercised by the tests at the
//! bottom of this file.
//!
//! Environment: `AVM_PKG_STORE` (store dir, default `./avm-store`),
//! `AVM_HOT_DIR` (hydration scratch, default system temp), `AVM_HOT_LIMIT`
//! (hot-store budget, default `1G`, suffixes K/M/G), `AVM_CPS2CELL`
//! (bridge helper used by `build`, default `cps2cell` on `PATH`),
//! `AVM_AUSTRAL` (compiler used by `build` for Austral sources, default
//! `austral` on `PATH`).

#![deny(unsafe_op_in_unsafe_fn)]

use avm_cas::crypto::{sign_manifest, signing_key_from_seed};
use avm_cas::engine::{hex, unhex};
use avm_cas::hot_store::{HotStats, HotStore};
use avm_cas::remote::{block_on, Remote};
use avm_cas::{ChunkStore, Manifest};
use avm_eval::{ClosureIndex, Derivation, DerivationId, MemoTable, Plan, Presence};
use avm_exec::{build_cell, parse_avce, BuildCell, Runner};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
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
  hot-bench [<manifest-hex>] [--size SZ] [--acquire N] [--fault-ins R]
            [--require]
                               time the two §3 hot-store rows through the
                               real HotStore API: resident acquire (mean ns,
                               target < 1 µs) and evicted-cell fault-in
                               (median ms, target < 15 ms for 50 MiB). With
                               a manifest it measures that cell; without,
                               it synthesizes an incompressible cell in a
                               throwaway store (default 50 MiB). --require
                               makes a regressed row a failure (non-zero
                               exit, report attached) — the CI smoke.
  gc [--dry-run] [--keep-roots]
                               compact unreferenced chunks (v1: every
                               manifest is a root; --keep-roots is implicit)
  drv-hash <plan.json> [target]    derivation Merkle ids (deterministic)
  eval <plan.json> [target] [--remote URL]
                               memoized warm evaluation report (L1 + L2,
                               plus L3 when a remote cache is named)
  build <source...> --sign <seed-hex32>
        [--drv <hex32> | --plan <plan.json> --for <name>]
        [--caps <cedar.json>] [--policy <cedar.txt>] [--name N] [--version V]
        [--austral <compiler>] [--cps2cell <helper>] [--no-totality]
        [-o cell.avm]
                               Austral sources (`mod.aum` or
                               `mod.aui,mod.aum`; the compiler runs with
                               --use-cps-jit --emit-cps --emit-derivations
                               --pkg-totality), or `.cps` IR, or an `.avce`
                               container -> signed `.avm` cell -> cold store.
                               The derivation identity (and closure, §4.2)
                               comes from --drv or from the plan's `--for`
                               derivation (default: --name / the source stem).
                               --policy names a Cedar policy set: the cell's
                               --caps table is then judged before anything
                               is spawned, and the compiler/cps2cell run
                               through avm-exec's runner (namespace
                               isolation as the host grants). A denied
                               build exits 1 with `EPERM: capability
                               denied for action Execute` and no tool
                               process ever existed.
  load <cell.avm> --policy <cedar.txt> [--base <hex>] [--key <hex32>]
       [--provider <cell.avm>]... [--plan <plan.json> --for NAME --cells <dir>]
       [--resolve SYM=HEX]... [--exec <program>]
                               gate the cell's own calls: `Execute`, then one
                               `Call:<symbol>` request per import it
                               declares, judged against the cell's *own*
                               capability table. A refusal exits 1 with
                               `EPERM: capability denied for action
                               \"Call:uk_…\"` before any relocation or
                               process. With --exec the linked image is
                               staged as cell.json and the program runs it
                               through avm-exec's runner (scrubbed env,
                               granted namespaces). --resolve supplies the
                               runtime symbol table the image links against
                               (the JIT's registered `uk_*` addresses).
                               --provider adds a cell whose exported symbols
                               the image may call; repeat for more.
                               --plan/--for/--cells adds the owner's plan
                               closure: every derivation NAME depends on,
                               dependency-first, as `<dir>/NAME.avm`.
                               --key is the builder's Ed25519 public key;
                               providers are signature-checked against it
                               (and so is the owner), and --provider without
                               --key is refused. Providers
                               are gated by their own capability tables and
                               their text is relocated too, then each is
                               mapped at base + 0x01000000, +0x02000000, …
                               by the executor.
  push <manifest-hex> --remote URL  upload only the chunks the cache lacks,
                               then the manifest
  pull <manifest-hex> --remote URL [-o FILE]
                               fetch only the chunks the store lacks, then
                               the manifest (fail-closed order)
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
        "hot-bench" => cmd_hot_bench(&ctx, args),
        "gc" => cmd_gc(&ctx, args),
        "drv-hash" => cmd_drv_hash(&ctx, args),
        "eval" => cmd_eval(&ctx, args),
        "get" => cmd_get(&ctx, args),
        "build" => cmd_build(&ctx, args),
        "load" => cmd_load(&ctx, args),
        "push" => cmd_push(&ctx, args),
        "pull" => cmd_pull(&ctx, args),
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

/// Flags that consume the following argument. `--flag=value` spellings are
/// not used by this CLI, so a value is always the next argv element.
const VALUE_FLAGS: &[&str] = &[
    "--store",
    "--name",
    "--version",
    "--sign",
    "--drv",
    "--plan",
    "--for",
    "--caps",
    "--austral",
    "--cps2cell",
    "--policy",
    "--base",
    "--key",
    "--resolve",
    "--provider",
    "--cells",
    "--exec",
    "--remote",
    "--hot-limit",
    "-o",
];

/// Positional (operand) arguments, with flag *values* excluded — including
/// `-o`'s, which the old `--`-prefix heuristic mistook for an operand.
fn positionals<'a>(args: &'a [String]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if VALUE_FLAGS.contains(&a) {
            i += 2;
            continue;
        }
        if a.starts_with('-') && a.len() > 1 {
            i += 1;
            continue;
        }
        out.push(a);
        i += 1;
    }
    out
}

/// The `cps2cell` helper: flag, env, or `PATH`.
fn cps_helper(args: &[String]) -> PathBuf {
    flag(args, "--cps2cell")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("AVM_CPS2CELL").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("cps2cell"))
}

fn cmd_put(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let path = *positionals(args)
        .first()
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
        if manifest.signature.is_some() {
            ", signed"
        } else {
            ""
        }
    ))
}

fn cmd_stat(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let id_hex = args
        .first()
        .ok_or_else(|| "stat needs a manifest hash".to_string())?;
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

/// Incompressible bytes for the synthetic probe cell: splitmix64, so the
/// cold store takes the STORE codec path on a real payload shape instead of
/// timing LZ4 against zeros.
fn incompressible(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 8);
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    while out.len() < len {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        out.extend_from_slice(&z.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// The two hot-store latency rows of the plan's §3 acceptance table,
/// measured through the API the runtime actually uses. A named manifest is
/// measured in place; with no argument the probe builds an incompressible
/// `--size` cell (default 50 MiB) in a throwaway store, so it is runnable on
/// a fresh machine and never writes into the configured store.
fn cmd_hot_bench(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let acquire_n: u32 = match flag(args, "--acquire") {
        Some(v) => v
            .parse()
            .map_err(|_| format!("bad --acquire `{v}` (want a count)"))?,
        None => 100_000,
    };
    let fault_ins: usize = match flag(args, "--fault-ins") {
        Some(v) => v
            .parse()
            .map_err(|_| format!("bad --fault-ins `{v}` (want a count)"))?,
        None => 5,
    };
    if acquire_n == 0 || fault_ins == 0 {
        return Err("--acquire and --fault-ins must be at least 1".into());
    }
    let size = match flag(args, "--size") {
        Some(v) => parse_size(v)? as usize,
        None => 50 * 1024 * 1024,
    };

    // `scratch` keeps the throwaway store's directory alive for the whole
    // measurement; a named cell lives in the configured store instead.
    let _scratch;
    let (store, id, size, mode) = match args.first() {
        Some(id_hex) if !id_hex.starts_with('-') => {
            let id = unhex(id_hex).map_err(|e| e.to_string())?;
            let store = Arc::new(open_store(ctx)?);
            let manifest = store.get_manifest(&id).map_err(|e| e.to_string())?;
            (store, id, manifest.total_size as usize, "cell")
        }
        _ => {
            let dir = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
            let store =
                Arc::new(ChunkStore::open(dir.path().join("store")).map_err(|e| e.to_string())?);
            let data = incompressible(size);
            let m = store
                .put_buffer("hot-bench", "1", &data, None)
                .map_err(|e| e.to_string())?;
            let id = m.identity_digest();
            _scratch = dir;
            (store, id, size, "synthetic")
        }
    };
    let capacity = (size as u64).saturating_mul(2).max(64 * 1024 * 1024);
    let hot = HotStore::new(store.clone(), capacity, hot_dir_from_env(), false)
        .map_err(|e| e.to_string())?;

    // Prime once: the timed acquire loop must hit the resident fast path.
    let primed = hot.acquire(&id).map_err(|e| e.to_string())?;
    if primed.len() != size {
        return Err(format!(
            "cell hydrated to {} bytes, manifest says {size}",
            primed.len()
        ));
    }
    drop(primed);
    let evictions_before = hot.stats().evictions;
    let t0 = Instant::now();
    for _ in 0..acquire_n {
        let map = hot.acquire(&id).map_err(|e| e.to_string())?;
        std::hint::black_box(&map);
    }
    let acquire_mean_ns = t0.elapsed().as_nanos() as f64 / f64::from(acquire_n);
    let acquire_evictions = hot.stats().evictions - evictions_before;

    // Fault-in: evict explicitly, then time the hydrate (cold pages stay
    // warm — the row measures hydration, not disk first-touch).
    let mut samples = Vec::with_capacity(fault_ins);
    for _ in 0..fault_ins {
        hot.release(&id);
        let t = Instant::now();
        let map = hot.acquire(&id).map_err(|e| e.to_string())?;
        samples.push(t.elapsed());
        if map.len() != size {
            return Err(format!(
                "fault-in hydrated {} bytes, manifest says {size}",
                map.len()
            ));
        }
    }
    samples.sort();
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
    let median_ms = ms(samples[samples.len() / 2]);
    let fifty = size == 50 * 1024 * 1024;

    let report = serde_json::json!({
        "mode": mode,
        "manifest": hex(&id),
        "size_bytes": size,
        "acquire_n": acquire_n,
        "acquire_mean_ns": acquire_mean_ns,
        "acquire_evictions": acquire_evictions,
        "acquire_under_1us": acquire_mean_ns < 1_000.0,
        "fault_in_n": fault_ins,
        "fault_in_median_ms": median_ms,
        "fault_in_min_ms": ms(samples[0]),
        "fault_in_max_ms": ms(samples[samples.len() - 1]),
        "fault_in_row_applies": fifty,
        "fault_in_under_15ms": if fifty { Some(median_ms < 15.0) } else { None },
    });
    if args.iter().any(|a| a == "--require") {
        if let Some(why) = latency_regression(&report) {
            return Err(format!("{why}; report: {report}"));
        }
    }
    Ok(report.to_string())
}

/// The `--require` predicate: which §3 latency row regressed, if any. Split
/// out from the timing loop so the failure branch is testable without
/// needing a machine slow enough to miss a row.
fn latency_regression(report: &serde_json::Value) -> Option<String> {
    if report["acquire_under_1us"] != serde_json::json!(true) {
        return Some(format!(
            "hot-store acquire regressed: mean {} ns (row: < 1 µs)",
            report["acquire_mean_ns"]
        ));
    }
    if report["fault_in_row_applies"] == serde_json::json!(true)
        && report["fault_in_under_15ms"] != serde_json::json!(true)
    {
        return Some(format!(
            "50 MiB fault-in regressed: median {} ms (row: < 15 ms)",
            report["fault_in_median_ms"]
        ));
    }
    None
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
    let file: PlanFile = serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))?;
    let mut plan = Plan::new();
    for d in file.derivations {
        plan.insert(d).map_err(|e| e.to_string())?;
    }
    Ok(plan)
}

fn cmd_drv_hash(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let _ = ctx; // store-independent: pure derivation hashing
    let path = args
        .first()
        .ok_or_else(|| "drv-hash needs <plan.json>".to_string())?;
    let target = args.get(1).map(|s| s.as_str());
    let plan = load_plan(path)?;
    let graph = plan.resolve(target).map_err(|e| e.to_string())?;
    let ids: std::collections::BTreeMap<&str, String> = graph
        .ids
        .iter()
        .map(|(k, v)| (k.as_str(), hex(v)))
        .collect();
    Ok(serde_json::json!({
        "target": target.and_then(|t| graph.ids.get(t)).map(|h| hex(h)),
        "resolved": ids,
        "order": graph.order,
    })
    .to_string())
}

fn cmd_eval(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let pos = positionals(args);
    let path = *pos
        .first()
        .ok_or_else(|| "eval needs <plan.json>".to_string())?;
    let target = pos.get(1).copied();
    let plan = load_plan(path)?;

    // L2 oracle: every manifest in the store that records a drv_hash.
    // L0 oracle: the (name -> drv_hash, closure) records the `build`
    // command writes; a format-1 manifest has no closure and can never
    // serve as an L0 hit. Iterating sorted manifest ids and keeping the
    // first record per name makes the choice deterministic.
    let mut closures: std::collections::HashMap<String, (DerivationId, DerivationId)> =
        std::collections::HashMap::new();
    let presence: Option<Arc<dyn Presence>> = if let Ok(store) = open_store(ctx) {
        let mut ids = HashSet::new();
        if let Ok(list) = store.list_manifests() {
            for id in list {
                if let Ok(m) = store.get_manifest(&id) {
                    if let Some(d) = m.drv_hash {
                        ids.insert(d);
                    }
                    if let (Some(d), Some(c)) = (m.drv_hash, m.closure) {
                        closures.entry(m.name.clone()).or_insert((d, c));
                    }
                }
            }
        }
        Some(Arc::new(StorePresence(ids)))
    } else {
        None
    };

    // L3 oracle: ask the remote cache about every id in the plan (spec
    // §3.3). Ids are content-derived, so one HEAD per node is the whole
    // protocol; resolved ids stay bit-identical whichever tier answers.
    let remote: Option<Arc<dyn Presence>> = match flag(args, "--remote") {
        Some(url) => {
            let r = Remote::new(url);
            let graph = plan.resolve(None).map_err(|e| e.to_string())?;
            let mut ids = HashSet::new();
            for id in graph.ids.values() {
                if block_on(r.has_manifest(id)).unwrap_or(false) {
                    ids.insert(*id);
                }
            }
            Some(Arc::new(StorePresence(ids)))
        }
        None => None,
    };

    let memo = MemoTable::with_remote(presence, remote)
        .with_closure_index(Arc::new(StoreClosures(closures)));
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
        "remote_hits": report.remote_hits,
        "closure_hits": report.closure_hits,
        "eval_us": dt.as_micros() as u64,
        "note": "closure_hits adopt a store build whose recorded input closure matches the plan; store_hits/remote_hits are id-keyed build skips (see PLAN_avm_pkg.md)",
    })
    .to_string())
}

// ── Sprint 3/4: `.avm` cell build ────────────────────────────────────

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cell".to_string())
}

/// Run the Cranelift bridge helper (`cps2cell`, a separate process by
/// spec §6.2 workspace separation) to turn CPS binary IR into an AVCE
/// container, and return its bytes.
/// Resolve a path against the caller's cwd. Build tools run with the
/// *staging* directory as their cwd once a runner is in play (and the
/// runner's working directory is not the caller's), so every file the
/// compiler sees has to be named absolutely.
fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Spawn one build tool. With no policy the command runs exactly as it
/// always has (inherited environment, caller's cwd). Under a policy it
/// goes through `avm-exec`'s runner: Cedar authorizes first, then the
/// child runs in whichever namespaces the host actually grants, with a
/// scrubbed environment — `LD_LIBRARY_PATH` is the one thing carried
/// through, because the compiler's shared library sits outside its
/// loader's default search path.
fn run_tool(
    runner: Option<&Runner>,
    program: &Path,
    args: &[std::ffi::OsString],
    hint: &str,
) -> Result<std::process::Output, String> {
    match runner {
        None => Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run `{}` ({e}); {hint}", program.display())),
        Some(runner) => {
            let env: Vec<(String, String)> = std::env::var("LD_LIBRARY_PATH")
                .map(|v| vec![("LD_LIBRARY_PATH".to_string(), v)])
                .unwrap_or_default();
            runner
                .run_with_env("Execute", program, args, &[], &env)
                .map_err(|e| match e {
                    // A refusal keeps its own `EPERM: …` text: wrapping it as
                    // "cannot run …" would bury the errno class.
                    avm_exec::RunnerError::Denied(_) => e.to_string(),
                    _ => format!("cannot run `{}` ({e}); {hint}", program.display()),
                })
        }
    }
}

fn cps_to_avce(input: &Path, helper: &Path, runner: Option<&Runner>) -> Result<Vec<u8>, String> {
    // A private temp file per invocation: concurrent builds in one process
    // must not share an output path, and the helper is free to overwrite
    // the empty file the tempfile creates.
    let out = tempfile::Builder::new()
        .prefix("avm-pkg-avce-")
        .suffix(".avce")
        .tempfile()
        .map_err(|e| format!("tempfile: {e}"))?;
    let out_path = out.path().to_path_buf();
    let status = run_tool(
        runner,
        helper,
        &[absolute(input).into(), out_path.clone().into_os_string()],
        "build the bridge (`make bridge`) or pass --cps2cell",
    )?
    .status;
    if !status.success() {
        return Err(format!(
            "cps2cell `{}` failed on {} ({status})",
            helper.display(),
            input.display()
        ));
    }
    let bytes =
        std::fs::read(&out_path).map_err(|e| format!("read {}: {e}", out_path.display()))?;
    drop(out);
    Ok(bytes)
}

// ── Sprint 3: cell loader (a cell's own calls) ──────────────────────

/// `load <cell.avm> --policy <cedar.txt> [--base HEX] [--key HEX32]
/// [--provider CELL]… [--plan PLAN --for NAME --cells DIR] [--resolve SYM=HEX]…
/// [--exec PROG]` — the load-time half of the capability row.
///
/// Unlike `build`, there is no ungated mode: loading a cell *is* the moment
/// its calls become reachable, so a missing policy is a usage error rather
/// than an implicit allow.
fn cmd_load(_ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let pos = positionals(args);
    let input = *pos
        .first()
        .ok_or_else(|| "load needs a `.avm` cell".to_string())?;
    if pos.len() > 1 {
        return Err(format!("load takes one cell, got {}", pos.len()));
    }
    let policy_path = flag(args, "--policy").ok_or_else(|| {
        "load needs --policy <cedar.txt>: a cell's calls are judged against a policy set, \
         and there is no default grant"
            .to_string()
    })?;
    let policy =
        std::fs::read_to_string(policy_path).map_err(|e| format!("read {policy_path}: {e}"))?;

    let base = match flag(args, "--base") {
        Some(h) => u64::from_str_radix(h.trim_start_matches("0x"), 16)
            .map_err(|e| format!("bad --base {h:?}: {e}"))?,
        None => 0,
    };

    // The runtime symbol table (`--resolve uk_version=0x7f…`), repeated. It
    // is what a JIT loader hands the linker: addresses, not names.
    let mut externs: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--resolve" {
            let spec = args.get(i + 1).ok_or_else(|| {
                "--resolve needs SYM=HEX (e.g. uk_version=0x7f00_0000)".to_string()
            })?;
            let (sym, addr) = spec
                .split_once('=')
                .ok_or_else(|| format!("--resolve {spec:?} is not SYM=HEX"))?;
            let addr = u64::from_str_radix(addr.trim_start_matches("0x"), 16)
                .map_err(|e| format!("bad --resolve address in {spec:?}: {e}"))?;
            if sym.is_empty() {
                return Err(format!("--resolve {spec:?} names no symbol"));
            }
            externs.insert(sym.to_string(), addr);
            i += 2;
        } else {
            i += 1;
        }
    }

    // Provider cells: their exported symbols resolve the owner's imports, and
    // their own text is relocated against the same table. Bases are assigned
    // deterministically so the executor can map each image at the address it
    // was linked for: one 16 MiB slot per provider above the owner.
    //
    // Explicit `--provider` cells come first (their order fixes their bases),
    // then the plan closure: `--plan … --for NAME --cells DIR` adds every
    // derivation NAME depends on, dependency-first, as `DIR/<name>.avm`. An
    // explicit cell whose file stem names the same derivation wins, so the
    // caller can override one provider without dropping the rest.
    const PROVIDER_STRIDE: u64 = 0x0100_0000;
    let mut provider_paths: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--provider" {
            let p = args
                .get(i + 1)
                .ok_or_else(|| "--provider needs a `.avm` cell path".to_string())?;
            if !provider_paths.iter().any(|q| q == p) {
                provider_paths.push(p.clone());
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    match (flag(args, "--plan"), flag(args, "--for"), flag(args, "--cells")) {
        (None, None, None) => {}
        (Some(plan_path), Some(target), Some(cells)) => {
            let plan = load_plan(plan_path)?;
            let graph = plan.resolve(Some(target)).map_err(|e| e.to_string())?;
            for name in &graph.order {
                // The owner is the cell being loaded, not one of its providers.
                if name == target {
                    continue;
                }
                let supplied = provider_paths.iter().any(|p| {
                    Path::new(p).file_stem().and_then(|s| s.to_str()) == Some(name.as_str())
                });
                if !supplied {
                    provider_paths.push(
                        Path::new(cells)
                            .join(format!("{name}.avm"))
                            .display()
                            .to_string(),
                    );
                }
            }
        }
        _ => {
            return Err(
                "--plan, --for and --cells go together: the plan names the closure, \
                 --for its root, and --cells is the directory holding `NAME.avm`"
                    .to_string(),
            )
        }
    }

    // A provider contributes executable text, so its layout is not enough:
    // every provider is checked against its builder's Ed25519 public key
    // (`--key`), and so is the owner when a key is supplied. A provider with
    // no key is refused rather than trusted by structure alone.
    let key: Option<[u8; 32]> = match flag(args, "--key") {
        Some(h) => {
            let raw = unhex(h).map_err(|e| format!("bad --key: {e}"))?;
            let k: [u8; 32] = raw.try_into().map_err(|_| {
                format!(
                    "--key must be 32 bytes of hex (64 hex chars), got {}",
                    h.len()
                )
            })?;
            Some(k)
        }
        None => None,
    };
    if !provider_paths.is_empty() && key.is_none() {
        return Err("--provider needs --key <pubkey-hex32>: a provider contributes \
             executable text, so it is signature-checked, never trusted by layout alone"
            .to_string());
    }

    let mut providers: Vec<(Vec<u8>, u64)> = Vec::with_capacity(provider_paths.len());
    for (n, path) in provider_paths.iter().enumerate() {
        let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        if let Some(k) = &key {
            let view =
                avm_exec::CellView::parse(&bytes).map_err(|e| format!("provider {path}: {e}"))?;
            view.verify_signature(k)
                .map_err(|e| format!("provider {path}: {e}"))?;
        }
        let pbase = base
            .checked_add((n as u64 + 1).saturating_mul(PROVIDER_STRIDE))
            .ok_or_else(|| format!("provider base for {path} overflows"))?;
        providers.push((bytes, pbase));
    }

    let bytes = std::fs::read(input).map_err(|e| format!("read {input}: {e}"))?;
    if let Some(k) = &key {
        avm_exec::CellView::parse(&bytes)
            .map_err(|e| format!("{input}: {e}"))?
            .verify_signature(k)
            .map_err(|e| format!("{input}: {e}"))?;
    }
    let refs: Vec<(&[u8], u64)> = providers
        .iter()
        .map(|(b, base)| (b.as_slice(), *base))
        .collect();
    let loaded =
        avm_exec::load_with(&bytes, &refs, &policy, base, &externs).map_err(|e| e.to_string())?;
    let drv = hex(&loaded.drv_hash);
    let exec = flag(args, "--exec");
    let mut report = serde_json::json!({
        "cell": input,
        "drv_hash": drv,
        "base": format!("{:#x}", loaded.base),
        "calls": loaded.calls,
        "symbols": loaded.image.symbols.len(),
        "text_bytes": loaded.image.text.len(),
        "providers": provider_paths
            .iter()
            .zip(loaded.providers.iter())
            .map(|(p, loaded)| serde_json::json!({ "path": p, "base": format!("{:#x}", loaded.base) }))
            .collect::<Vec<_>>(),
        "spawned": exec.is_some(),
    });

    if let Some(prog) = exec {
        // The runner stages the image and runs with the staging dir as cwd,
        // so the executor is named absolutely for the same reason the
        // gated build's tools are.
        let out = avm_exec::run_image(&absolute(Path::new(prog)), &policy, &loaded)
            .map_err(|e| e.to_string())?;
        let obj = report.as_object_mut().expect("report is an object");
        obj.insert("exit".into(), serde_json::json!(out.status.code()));
        obj.insert(
            "stdout".into(),
            serde_json::json!(String::from_utf8_lossy(&out.stdout)),
        );
        obj.insert(
            "stderr".into(),
            serde_json::json!(String::from_utf8_lossy(&out.stderr)),
        );
        if !out.status.success() {
            return Err(format!("executor `{prog}` failed ({})", out.status));
        }
    }

    Ok(report.to_string())
}

fn cmd_build(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let pos = positionals(args);
    let input = *pos
        .first()
        .ok_or_else(|| "build needs a source (.aum, .aui,.aum, .cps, or .avce)".to_string())?;
    let input_path = Path::new(input);
    let name = flag(args, "--name")
        .map(str::to_string)
        .unwrap_or_else(|| file_stem(input_path));
    let version = flag(args, "--version").unwrap_or("0.0.0").to_string();
    let seed_hex = flag(args, "--sign").ok_or_else(|| {
        "build needs --sign <seed-hex32>: a cell carries a mandatory Ed25519 signature".to_string()
    })?;
    let seed = unhex(seed_hex).map_err(|e| format!("bad --sign: {e}"))?;
    let no_totality = args.iter().any(|a| a == "--no-totality");

    let caps: serde_json::Value = match flag(args, "--caps") {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("read {p}: {e}"))?;
            serde_json::from_str(&text).map_err(|e| format!("parse caps {p}: {e}"))?
        }
        None => serde_json::json!([]),
    };
    if !(caps.is_array() || caps.is_object()) {
        return Err("capability table must serialize to a JSON array or object".into());
    }

    // The live capability gate (spec §3.1's runtime half). With a policy
    // named, the build's own capability table is judged by Cedar *before*
    // any tool is spawned, and the tools then run through `avm-exec`'s
    // runner — so the refusal is the same `EPERM` class the OS raises, and
    // it happens while no builder process exists. Without a policy nothing
    // is declared and no gate applies; that is stated in PLAN_avm_pkg.md,
    // never assumed.
    let runner = match flag(args, "--policy") {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("read {p}: {e}"))?;
            let sandbox =
                avm_exec::Sandbox::new(caps.clone(), &text).map_err(|e| format!("cedar: {e}"))?;
            let runner = Runner::new(sandbox);
            // "No capability ⇒ no phase": refuse before reading a source
            // or spawning anything.
            runner.authorize("Execute").map_err(|e| e.to_string())?;
            Some(runner)
        }
        None => None,
    };

    // Produce the AVCE container. An Austral source goes through the
    // compiler (`--use-cps-jit --emit-cps --emit-derivations`, and the
    // totality pass unless disabled), which is also where the plan comes
    // from; `.cps` is the compiler's IR and `.avce` the bridge container.
    let helper = cps_helper(args);
    let (avce_bytes, emitted_plan, totality) = if input.ends_with(".avce") {
        if pos.len() > 1 {
            return Err(format!("build takes one .avce file, got {}", pos.len()));
        }
        (
            std::fs::read(input_path).map_err(|e| format!("read {input}: {e}"))?,
            None,
            None,
        )
    } else if input.ends_with(".cps") {
        if pos.len() > 1 {
            return Err(format!("build takes one .cps file, got {}", pos.len()));
        }
        (
            cps_to_avce(input_path, &helper, runner.as_ref())?,
            None,
            None,
        )
    } else {
        let austral = flag(args, "--austral")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("AVM_AUSTRAL").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("austral"));
        let (avce, plan) = compile_austral(&pos, &austral, &helper, !no_totality, runner.as_ref())?;
        (
            avce,
            plan,
            Some(if no_totality { "unproven" } else { "proven" }),
        )
    };

    // Derivation identity: explicit, or recomputed from a plan (the same
    // Merkle calculus `drv-hash` prints), never fabricated. A plan also
    // supplies the freshness-carrying input closure, which is what a later
    // warm evaluation uses to adopt this build under its name.
    let explicit_plan: Option<Plan> = match flag(args, "--plan") {
        Some(p) => Some(load_plan(p)?),
        None => None,
    };
    let target = flag(args, "--for").unwrap_or(&name).to_string();
    let plan_for_identity: Option<Plan> = match explicit_plan {
        Some(p) => Some(p),
        // An emitted plan only applies when it actually describes the
        // derivation being built; a module with no `Derivation` constants
        // must not turn `--drv` into an error.
        None => match emitted_plan {
            Some(p) if p.get(&target).is_some() => Some(p),
            Some(p) if flag(args, "--drv").is_none() => {
                return Err(format!(
                    "the compiled plan has no derivation `{target}` (available: {}); \
                     pass --for <name> or --drv <hex32>",
                    p.all_names().join(", ")
                ))
            }
            _ => None,
        },
    };
    let plan_id = match &plan_for_identity {
        Some(plan) => {
            let graph = plan.resolve(Some(&target)).map_err(|e| e.to_string())?;
            Some(
                *graph
                    .ids
                    .get(&target)
                    .ok_or_else(|| format!("plan has no derivation `{target}`"))?,
            )
        }
        None => None,
    };
    let drv = match flag(args, "--drv") {
        Some(h) => {
            let explicit = unhex(h).map_err(|e| format!("bad --drv: {e}"))?;
            if let Some(resolved) = plan_id {
                if resolved != explicit {
                    return Err(format!(
                        "--drv {} disagrees with the plan's `{target}` ({}) — refusing \
                         to record a closure for a different derivation",
                        hex(&explicit),
                        hex(&resolved)
                    ));
                }
            }
            explicit
        }
        None => plan_id.ok_or_else(|| {
            format!(
                "build needs --drv <hex32> or a plan with derivation `{target}` \
                 (--plan … --for <name>, or an Austral module that declares one)"
            )
        })?,
    };
    let closure = match &plan_for_identity {
        Some(plan) => Some(plan.closure_digest(&target).map_err(|e| e.to_string())?),
        None => None,
    };

    let doc = parse_avce(&avce_bytes).map_err(|e| e.to_string())?;
    let cell = build_cell(&BuildCell {
        drv_hash: drv,
        capability_table: &caps,
        avce: &doc,
        signing_seed: &seed,
    })
    .map_err(|e| e.to_string())?;

    let store = open_store(ctx)?;
    let mut manifest = store
        .put_buffer_with_closure(&name, &version, &cell, Some(drv), closure)
        .map_err(|e| e.to_string())?;
    let signing = signing_key_from_seed(&seed);
    sign_manifest(&mut manifest, &signing);
    let id = store.put_manifest(&manifest).map_err(|e| e.to_string())?;

    let wrote = match flag(args, "-o") {
        Some(out) => {
            std::fs::write(out, &cell).map_err(|e| format!("write {out}: {e}"))?;
            Some(out.to_string())
        }
        None => None,
    };
    let relocs: usize = doc.functions.iter().map(|f| f.relocs.len()).sum();
    Ok(serde_json::json!({
        "identity": hex(&id),
        "drv_hash": hex(&drv),
        "closure": closure.map(|c| hex(&c)),
        "name": name,
        "version": version,
        "sources": pos,
        "totality": totality,
        "cell_bytes": cell.len(),
        "pubkey": hex(&signing.verifying_key().to_bytes()),
        "arch": doc.arch,
        "functions": doc.functions.len(),
        "imports": doc.imports.len(),
        "relocations": relocs,
        "wrote": wrote,
    })
    .to_string())
}

/// Compile Austral source modules to an AVCE container, returning the
/// container bytes and the plan the compiler exported (if any). Runs in a
/// private temp dir, so a failed build leaves nothing behind.
fn compile_austral(
    sources: &[&str],
    austral: &Path,
    helper: &Path,
    totality: bool,
    runner: Option<&Runner>,
) -> Result<(Vec<u8>, Option<Plan>), String> {
    let dir = tempfile::Builder::new()
        .prefix("avm-pkg-build-")
        .tempdir()
        .map_err(|e| format!("tempdir: {e}"))?;
    let cps = dir.path().join("cell.cps");
    let plan_path = dir.path().join("plan.json");
    let c_out = dir.path().join("cell.c");

    let mut argv: Vec<std::ffi::OsString> = vec!["compile".into()];
    for s in sources {
        // One positional may name an interface/body pair (`mod.aui,mod.aum`)
        // — the compiler splits on the comma, so each half has to be made
        // absolute separately.
        let parts: Vec<String> = s
            .split(',')
            .map(|p| absolute(Path::new(p)).display().to_string())
            .collect();
        argv.push(parts.join(",").into());
    }
    argv.push("--target-type=c".into());
    argv.push("--no-entrypoint".into());
    argv.push(format!("--output={}", c_out.display()).into());
    argv.push("--use-cps-jit".into());
    argv.push("--allow-all".into());
    argv.push(format!("--emit-cps={}", cps.display()).into());
    argv.push(format!("--emit-derivations={}", plan_path.display()).into());
    if totality {
        argv.push("--pkg-totality=true".into());
    }
    let out = run_tool(
        runner,
        austral,
        &argv,
        "build the compiler (`make all`) or pass --austral <path>",
    )?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!(
            "austral compile failed ({}):\n{}",
            out.status,
            tail.trim_end()
        ));
    }
    if !cps.is_file() {
        return Err(
            "austral compile produced no CPS IR: the module defines no functions \
             (the CPS path emits function bodies only)"
                .into(),
        );
    }
    let avce = cps_to_avce(&cps, helper, runner)?;
    let plan = if plan_path.is_file() {
        Some(load_plan(&plan_path.display().to_string())?)
    } else {
        None
    };
    Ok((avce, plan))
}

// ── Sprint 5.1: remote binary cache ──────────────────────────────────

fn remote_from(args: &[String]) -> Result<Remote, String> {
    let url =
        flag(args, "--remote").ok_or_else(|| "a --remote <base-url> is required".to_string())?;
    Ok(Remote::new(url))
}

fn cmd_push(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let id_hex = positionals(args)
        .first()
        .copied()
        .ok_or_else(|| "push needs a manifest hash".to_string())?;
    let id = unhex(id_hex).map_err(|e| e.to_string())?;
    let remote = remote_from(args)?;
    let store = open_store(ctx)?;
    let manifest = store.get_manifest(&id).map_err(|e| e.to_string())?;
    let sent = block_on(remote.push_chunks(&store, &manifest.chunks)).map_err(|e| e.to_string())?;
    block_on(remote.push_manifest(&store, &id)).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "manifest": hex(&id),
        "name": manifest.name,
        "version": manifest.version,
        "bytes": manifest.total_size,
        "chunks_total": manifest.chunks.len(),
        "chunks_sent": sent,
        "remote": remote.base(),
        "note": "only chunks the remote lacked travelled (one missing-list round-trip)",
    })
    .to_string())
}

fn cmd_pull(ctx: &Ctx, args: &[String]) -> Result<String, String> {
    let id_hex = positionals(args)
        .first()
        .copied()
        .ok_or_else(|| "pull needs a manifest hash".to_string())?;
    let id = unhex(id_hex).map_err(|e| e.to_string())?;
    let remote = remote_from(args)?;
    let store = open_store(ctx)?;
    // The manifest is fetched (not persisted) first: its chunk list is what
    // the missing-only fetch asks for. `put_manifest` then refuses if any
    // referenced chunk is still absent — fail-closed, chunks first.
    let manifest = block_on(remote.fetch_manifest(&id)).map_err(|e| e.to_string())?;
    let fetched =
        block_on(remote.pull_chunks(&store, &manifest.chunks)).map_err(|e| e.to_string())?;
    store.put_manifest(&manifest).map_err(|e| e.to_string())?;
    let wrote = match flag(args, "-o") {
        Some(out) => {
            let bytes = store.read_assembled(&manifest).map_err(|e| e.to_string())?;
            std::fs::write(out, &bytes).map_err(|e| format!("write {out}: {e}"))?;
            Some(out.to_string())
        }
        None => None,
    };
    Ok(serde_json::json!({
        "manifest": hex(&id),
        "name": manifest.name,
        "version": manifest.version,
        "bytes": manifest.total_size,
        "chunks_total": manifest.chunks.len(),
        "chunks_fetched": fetched,
        "remote": remote.base(),
        "wrote": wrote,
        "note": "only chunks the local store lacked were requested",
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

/// Store-derived freshness records: `name -> (drv_hash, closure)` from
/// manifests that carry both. The memo adopts a record only when the
/// plan-side closure matches, so a stale record is inert.
struct StoreClosures(std::collections::HashMap<String, (DerivationId, DerivationId)>);
impl ClosureIndex for StoreClosures {
    fn lookup(&self, name: &str) -> Option<(DerivationId, DerivationId)> {
        self.0.get(name).copied()
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
    let id_hex = args
        .first()
        .ok_or_else(|| "get needs a manifest hash".to_string())?;
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
            let ctx = Ctx {
                store_dir: dir.path().join("store"),
            };
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
        assert!(
            stat.contains(&format!("\"drv_hash\":\"{}\"", hex(&[7u8; 32]))),
            "{stat}"
        );

        let out_file = ts._dir.path().join("out.bin");
        let get_out = cmd_get(
            &ts.ctx,
            &[id_hex.clone(), "-o".into(), out_file.display().to_string()],
        )
        .unwrap();
        assert!(get_out.contains(&format!("\"bytes\":{}", payload.len())));
        assert_eq!(
            std::fs::read(&out_file).unwrap(),
            payload,
            "bytes must roundtrip"
        );
    }

    #[test]
    fn signed_put_verifies() {
        let ts = TempStore::new();
        let f = ts._dir.path().join("s.bin");
        write(&f, b"signed payload");
        let seed = hex(&[42u8; 32]);
        let put_out = cmd_put(
            &ts.ctx,
            &[f.display().to_string(), "--sign".into(), seed.clone()],
        )
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
        let dead: u64 = serde_json::from_str::<serde_json::Value>(&dry2).unwrap()["dead_chunks"]
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
        let mut m = store
            .put_buffer("mid", "1", b"built-mid", Some(mid_id))
            .unwrap();
        avm_cas::crypto::sign_manifest(&mut m, &signing_key_from_seed(&[1u8; 32]));
        store.put_manifest(&m).unwrap();

        let e2 = cmd_eval(&ts.ctx, &[plan_path.display().to_string()]).unwrap();
        assert!(e2.contains("\"store_hits\":[\"mid\"]"), "{e2}");
        let t1: serde_json::Value = serde_json::from_str(&e1).unwrap();
        let t2: serde_json::Value = serde_json::from_str(&e2).unwrap();
        assert_eq!(t1["target"], t2["target"], "store tier may not move ids");
    }

    // ── build / push / pull ──────────────────────────────────────────

    /// Minimal `AVCE` container: one function, no relocs, no imports.
    fn avce_bytes() -> Vec<u8> {
        fn put_str(b: &mut Vec<u8>, s: &str) {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        }
        let body = [0xB8u8, 0x07, 0x00, 0x00, 0x00, 0xC3];
        let mut b = Vec::new();
        b.extend_from_slice(&0x4156_4345u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        put_str(&mut b, "x86_64");
        b.extend_from_slice(&1u32.to_le_bytes());
        put_str(&mut b, "run");
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(&body);
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b
    }

    #[test]
    fn build_serializes_a_signed_cell_and_records_its_derivation() {
        let ts = TempStore::new();
        let avce = ts._dir.path().join("cell.avce");
        write(&avce, &avce_bytes());
        let out = ts._dir.path().join("cell.avm");
        let seed = [9u8; 32];
        let drv = [5u8; 32];

        let json = cmd_build(
            &ts.ctx,
            &[
                avce.display().to_string(),
                "--name".into(),
                "demo".into(),
                "--version".into(),
                "2.0.0".into(),
                "--drv".into(),
                hex(&drv),
                "--sign".into(),
                hex(&seed),
                "--caps".into(),
                write_caps(&ts._dir.path().join("caps.json")),
                "-o".into(),
                out.display().to_string(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["functions"], 1);
        assert_eq!(v["arch"], "x86_64");
        assert_eq!(v["drv_hash"], hex(&drv));

        let cell = std::fs::read(&out).unwrap();
        assert_eq!(v["cell_bytes"].as_u64().unwrap() as usize, cell.len());
        let view = avm_exec::CellView::parse(&cell).unwrap();
        assert_eq!(view.drv_hash, drv, "the cell must record its derivation");
        assert_eq!(view.arch_str().unwrap(), "x86_64");
        assert_eq!(view.symdef("run").unwrap().len as usize, 6);
        assert_eq!(view.capability_table, serde_json::json!(["net"]));
        let key = ed25519_dalek::SigningKey::from_bytes(&seed);
        view.verify_signature(&key.verifying_key().to_bytes())
            .expect("cell signature must verify against the builder key");
        assert_eq!(
            v["pubkey"],
            hex(&key.verifying_key().to_bytes()),
            "the report names the public half, so a loader can verify"
        );

        // The same bytes landed in the cold store, under the derivation.
        let id = unhex(v["identity"].as_str().unwrap()).unwrap();
        let m = open_store(&ts.ctx).unwrap().get_manifest(&id).unwrap();
        assert_eq!(m.drv_hash, Some(drv));
        assert_eq!(m.name, "demo");
        assert!(m.signature.is_some(), "manifest is signed too");
        avm_cas::crypto::verify_manifest(&m).unwrap();
    }

    fn write_caps(path: &Path) -> String {
        std::fs::write(path, r#"["net"]"#).unwrap();
        path.display().to_string()
    }

    #[test]
    fn build_can_derive_the_identity_from_a_plan() {
        let ts = TempStore::new();
        let avce = ts._dir.path().join("p.avce");
        write(&avce, &avce_bytes());
        let plan = ts._dir.path().join("plan.json");
        std::fs::write(
            &plan,
            r#"{"derivations":[{"name":"base","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["aa"],"env_vars":[]}]}"#,
        )
        .unwrap();

        let json = cmd_build(
            &ts.ctx,
            &[
                avce.display().to_string(),
                "--plan".into(),
                plan.display().to_string(),
                "--for".into(),
                "base".into(),
                "--sign".into(),
                hex(&[3u8; 32]),
            ],
        )
        .unwrap();
        let built: serde_json::Value = serde_json::from_str(&json).unwrap();
        let resolved: serde_json::Value =
            serde_json::from_str(&cmd_drv_hash(&ts.ctx, &[plan.display().to_string()]).unwrap())
                .unwrap();
        assert_eq!(built["drv_hash"], resolved["resolved"]["base"]);

        // The build recorded the plan-side input closure beside its
        // derivation, so a later warm evaluation can guard a name lookup.
        let plan_obj = load_plan(&plan.display().to_string()).unwrap();
        let expected = plan_obj.closure_digest("base").unwrap();
        assert_eq!(built["closure"], hex(&expected));
        let id = unhex(built["identity"].as_str().unwrap()).unwrap();
        let m = open_store(&ts.ctx).unwrap().get_manifest(&id).unwrap();
        assert_eq!(m.closure, Some(expected));
    }

    /// The freshness-carrying manifest pays off end to end: `build` records
    /// the closure, `eval` adopts the build under its name, and a patched
    /// plan falls back to hashing because the recorded closure no longer
    /// matches.
    #[test]
    fn eval_adopts_a_closure_matching_build_and_rejects_a_stale_one() {
        let ts = TempStore::new();
        let avce = ts._dir.path().join("b.avce");
        write(&avce, &avce_bytes());
        let plan = ts._dir.path().join("plan.json");
        std::fs::write(
            &plan,
            r#"{"derivations":[
                {"name":"base","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["aa"],"env_vars":[]},
                {"name":"app","version":"1","builder_symbol":"stdenv.build","dependencies":["base"],"sources":[],"env_vars":[]}
            ]}"#,
        )
        .unwrap();
        cmd_build(
            &ts.ctx,
            &[
                avce.display().to_string(),
                "--name".into(),
                "base".into(),
                "--plan".into(),
                plan.display().to_string(),
                "--for".into(),
                "base".into(),
                "--sign".into(),
                hex(&[4u8; 32]),
            ],
        )
        .unwrap();

        let warm: serde_json::Value =
            serde_json::from_str(&cmd_eval(&ts.ctx, &[plan.display().to_string()]).unwrap())
                .unwrap();
        assert_eq!(warm["closure_hits"], serde_json::json!(["base"]));
        assert_eq!(
            warm["visited"],
            serde_json::json!(["app"]),
            "the adopted subtree must not be re-hashed"
        );
        assert_eq!(warm["resolved"], 2);

        // Patch the source `base` was built from: the record is stale and
        // must not be adopted, and the resolved id must be the new one.
        let cold_before = warm["target"].as_str().unwrap().to_string();
        let patched = ts._dir.path().join("patched.json");
        std::fs::write(
            &patched,
            r#"{"derivations":[
                {"name":"base","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["cc"],"env_vars":[]},
                {"name":"app","version":"1","builder_symbol":"stdenv.build","dependencies":["base"],"sources":[],"env_vars":[]}
            ]}"#,
        )
        .unwrap();
        let stale: serde_json::Value =
            serde_json::from_str(&cmd_eval(&ts.ctx, &[patched.display().to_string()]).unwrap())
                .unwrap();
        assert_eq!(stale["closure_hits"], serde_json::json!([]));
        assert_eq!(stale["visited"], serde_json::json!(["base", "app"]));
        assert_ne!(
            stale["target"].as_str().unwrap(),
            cold_before,
            "the patch must move the derivation id"
        );
    }

    #[test]
    fn build_refuses_without_a_signature_or_derivation() {
        let ts = TempStore::new();
        let avce = ts._dir.path().join("c.avce");
        write(&avce, &avce_bytes());
        let no_sig = cmd_build(&ts.ctx, &[avce.display().to_string()]);
        assert!(no_sig.unwrap_err().contains("--sign"));
        let no_drv = cmd_build(
            &ts.ctx,
            &[avce.display().to_string(), "--sign".into(), hex(&[1u8; 32])],
        );
        let msg = no_drv.unwrap_err();
        assert!(msg.contains("--drv") && msg.contains("--plan"), "{msg}");
    }

    // ── end-to-end `build` from Austral source ───────────────────────

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    /// A real `.avm` cell with its own capability table and declared calls —
    /// the loader's input. Nine bytes of text so an `Abs8` slot at offset 1
    /// fits.
    fn cell_with_imports(imports: &[&str], relocs: Vec<avm_exec::AvceReloc>) -> Vec<u8> {
        let caps = serde_json::json!([
            { "uid": { "type": "ExecCap", "id": "exec" }, "attrs": {}, "parents": [] }
        ]);
        let doc = avm_exec::AvceDoc {
            arch: "x86_64".into(),
            functions: vec![avm_exec::AvceFn {
                name: "run".into(),
                body: vec![0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3, 0x90, 0x90, 0x90],
                relocs,
            }],
            imports: imports.iter().map(|s| s.to_string()).collect(),
        };
        avm_exec::build_cell(&avm_exec::BuildCell {
            drv_hash: [0x33u8; 32],
            capability_table: &caps,
            avce: &doc,
            signing_seed: &[9u8; 32],
        })
        .unwrap()
    }

    /// A provider cell that *defines* `sym` (one function, no imports).
    fn cell_defining(sym: &str) -> Vec<u8> {
        let caps = serde_json::json!([
            { "uid": { "type": "ExecCap", "id": "exec" }, "attrs": {}, "parents": [] }
        ]);
        let doc = avm_exec::AvceDoc {
            arch: "x86_64".into(),
            functions: vec![avm_exec::AvceFn {
                name: sym.into(),
                body: vec![0xB8, 0x07, 0x00, 0x00, 0x00, 0xC3, 0x90, 0x90, 0x90],
                relocs: vec![],
            }],
            imports: vec![],
        };
        avm_exec::build_cell(&avm_exec::BuildCell {
            drv_hash: [0x44u8; 32],
            capability_table: &caps,
            avce: &doc,
            signing_seed: &[9u8; 32],
        })
        .unwrap()
    }

    const FAKE_PLAN: &str = r#"{"derivations":[
        {"name":"demo","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["aa"],"env_vars":[],"totality":"proven"}
    ]}"#;

    /// A stand-in for `austral compile`: it records its argv, then writes a
    /// dummy CPS file, the plan fixture, and the C output at the paths the
    /// real compiler is asked for. The `cps2cell` stand-in copies a fixed
    /// AVCE container. This exercises the *plumbing* — argument shape,
    /// emitted plan, identity + closure, store write — not the OCaml
    /// compiler, which is covered by its own suite.
    #[cfg(unix)]
    fn fake_toolchain(dir: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
        let avce_fixture = dir.join("fixture.avce");
        write(&avce_fixture, &avce_bytes());
        let plan_fixture = dir.join("fixture-plan.json");
        write(&plan_fixture, FAKE_PLAN.as_bytes());
        let argv_out = dir.join("austral-argv.txt");

        let helper = script(
            dir,
            "fake-cps2cell",
            &format!("#!/bin/sh\ncp '{}' \"$2\"\n", avce_fixture.display()),
        );
        let austral = script(
            dir,
            "fake-austral",
            &format!(
                r#"#!/bin/sh
printf '%s\n' "$@" > '{argv}'
CPS=""; PLAN=""; OUT=""; TGT=""; JIT=""; ALLOW=""
for a in "$@"; do
  case "$a" in
    --emit-cps=*) CPS="${{a#--emit-cps=}}" ;;
    --emit-derivations=*) PLAN="${{a#--emit-derivations=}}" ;;
    --output=*) OUT="${{a#--output=}}" ;;
    --target-type=c) TGT=1 ;;
    --use-cps-jit) JIT=1 ;;
    --allow-all) ALLOW=1 ;;
  esac
done
[ -n "$CPS" ] && [ -n "$PLAN" ] && [ -n "$OUT" ] && [ -n "$TGT" ] && [ -n "$JIT" ] && [ -n "$ALLOW" ] || exit 2
printf 'ir' > "$CPS"
cp '{plan}' "$PLAN"
: > "$OUT"
"#,
                argv = argv_out.display(),
                plan = plan_fixture.display()
            ),
        );
        (austral, helper, avce_fixture, plan_fixture, argv_out)
    }

    #[cfg(unix)]
    #[test]
    fn build_compiles_an_austral_source_through_plan_and_cps() {
        let ts = TempStore::new();
        let (austral, helper, _avce, plan_fixture, argv_out) = fake_toolchain(ts._dir.path());
        let src = ts._dir.path().join("demo.aum");
        write(&src, b"module body Demo is end module body.\n");
        let out = ts._dir.path().join("demo.avm");

        let json = cmd_build(
            &ts.ctx,
            &[
                src.display().to_string(),
                "--for".into(),
                "demo".into(),
                "--version".into(),
                "1".into(),
                "--sign".into(),
                hex(&[6u8; 32]),
                "--austral".into(),
                austral.display().to_string(),
                "--cps2cell".into(),
                helper.display().to_string(),
                "-o".into(),
                out.display().to_string(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["sources"], serde_json::json!([src.display().to_string()]));
        assert_eq!(v["totality"], "proven");
        assert_eq!(v["arch"], "x86_64");

        // The compiler was driven with the CPS/plan/totality flags.
        let argv = std::fs::read_to_string(&argv_out).unwrap();
        for flag in [
            "compile",
            "--target-type=c",
            "--no-entrypoint",
            "--use-cps-jit",
            "--allow-all",
            "--pkg-totality=true",
        ] {
            assert!(
                argv.lines().any(|l| l == flag),
                "missing {flag} in:\n{argv}"
            );
        }
        assert!(argv.lines().any(|l| l.starts_with("--emit-cps=")), "{argv}");
        assert!(
            argv.lines().any(|l| l.starts_with("--emit-derivations=")),
            "{argv}"
        );

        // Identity and closure come from the *emitted* plan.
        let plan = load_plan(&plan_fixture.display().to_string()).unwrap();
        let drv = plan.resolve(Some("demo")).unwrap().ids["demo"];
        assert_eq!(v["drv_hash"], hex(&drv));
        assert_eq!(v["closure"], hex(&plan.closure_digest("demo").unwrap()));

        // The signed cell is in the store and on disk.
        let id = unhex(v["identity"].as_str().unwrap()).unwrap();
        let m = open_store(&ts.ctx).unwrap().get_manifest(&id).unwrap();
        assert_eq!(m.drv_hash, Some(drv));
        assert_eq!(m.closure, Some(plan.closure_digest("demo").unwrap()));
        let cell = std::fs::read(&out).unwrap();
        let view = avm_exec::CellView::parse(&cell).unwrap();
        assert_eq!(view.drv_hash, drv);
    }

    #[cfg(unix)]
    #[test]
    fn build_can_skip_the_totality_pass_and_reports_it() {
        let ts = TempStore::new();
        let (austral, helper, _avce, _plan, argv_out) = fake_toolchain(ts._dir.path());
        let src = ts._dir.path().join("demo.aum");
        write(&src, b"module body Demo is end module body.\n");
        let json = cmd_build(
            &ts.ctx,
            &[
                src.display().to_string(),
                "--for".into(),
                "demo".into(),
                "--sign".into(),
                hex(&[7u8; 32]),
                "--austral".into(),
                austral.display().to_string(),
                "--cps2cell".into(),
                helper.display().to_string(),
                "--no-totality".into(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["totality"], "unproven");
        let argv = std::fs::read_to_string(&argv_out).unwrap();
        assert!(
            !argv.lines().any(|l| l == "--pkg-totality=true"),
            "--no-totality must not pass the pass flag:\n{argv}"
        );
    }

    /// The documented fixtures under `safestos/pkg/policies/` are the ones
    /// the plan's deny/allow example copies, so they are pinned here: each
    /// must parse, and each must decide exactly what its comment claims.
    /// A fixture that drifts from its prose turns this test red instead of
    /// sending a reader down a broken example.
    #[test]
    fn shipped_policy_fixtures_parse_and_decide_what_they_say() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../policies");
        let caps: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("caps-exec.json")).unwrap())
                .expect("caps-exec.json is Cedar entities JSON");
        let allows = |file: &str, action: &str| -> bool {
            let text = std::fs::read_to_string(dir.join(file))
                .unwrap_or_else(|e| panic!("read {file}: {e}"));
            avm_exec::Sandbox::new(caps.clone(), &text)
                .unwrap_or_else(|e| panic!("{file} must parse: {e}"))
                .check(action)
        };

        assert!(allows("allow.cedar", "Execute"));
        assert!(allows("allow.cedar", "Call:au_alloc"));
        assert!(!allows("deny-execute.cedar", "Execute"));
        // The per-call fixture permits running and every other call, and
        // refuses exactly one symbol.
        assert!(allows("deny-call-au-alloc.cedar", "Execute"));
        assert!(!allows("deny-call-au-alloc.cedar", "Call:au_alloc"));
        assert!(allows("deny-call-au-alloc.cedar", "Call:uk_version"));
    }

    /// The plan closure is the provider set: `--plan … --for … --cells` turns
    /// every dependency into `<dir>/NAME.avm`, verifies each against the
    /// builder key, and links it; a provider with no key, a wrong key, or an
    /// incomplete closure is refused rather than trusted.
    #[test]
    fn load_resolves_the_plan_closure_as_verified_providers() {
        let ts = TempStore::new();
        let cells = ts._dir.path().join("cells");
        std::fs::create_dir(&cells).unwrap();
        write(&cells.join("lib.avm"), &cell_defining("double"));
        let owner = ts._dir.path().join("app.avm");
        write(
            &owner,
            &cell_with_imports(
                &["double"],
                vec![avm_exec::AvceReloc {
                    offset: 1,
                    addend: 0,
                    kind: "Abs8".into(),
                    symbol: "double".into(),
                }],
            ),
        );
        let plan = ts._dir.path().join("plan.json");
        std::fs::write(
            &plan,
            r#"{"derivations":[
                {"name":"lib","version":"1","builder_symbol":"t","dependencies":[],"sources":[],"env_vars":[]},
                {"name":"app","version":"1","builder_symbol":"t","dependencies":["lib"],"sources":[],"env_vars":[]}
            ]}"#,
        )
        .unwrap();
        let allow = ts._dir.path().join("allow.cedar");
        write(&allow, b"permit(principal, action, resource);\n");
        let key = hex(
            &ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        let base = [
            owner.display().to_string(),
            "--policy".into(),
            allow.display().to_string(),
            "--plan".into(),
            plan.display().to_string(),
            "--for".into(),
            "app".into(),
            "--cells".into(),
            cells.display().to_string(),
            "--key".into(),
            key.clone(),
        ]
        .to_vec();
        let out = cmd_load(&ts.ctx, &base).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["calls"], serde_json::json!(["double"]));
        assert_eq!(v["providers"].as_array().unwrap().len(), 1, "{out}");
        assert_eq!(
            v["providers"][0]["path"],
            cells.join("lib.avm").display().to_string()
        );
        assert_eq!(v["providers"][0]["base"], "0x1000000");

        // No key: a provider is executable text, so it is refused.
        let mut no_key = base.clone();
        no_key.truncate(no_key.len() - 2);
        let err = cmd_load(&ts.ctx, &no_key).unwrap_err();
        assert!(err.contains("--key"), "got: {err}");

        // Wrong key: the signature, not the layout, refuses it.
        let mut wrong = base.clone();
        *wrong.last_mut().unwrap() = hex(
            &ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        let err = cmd_load(&ts.ctx, &wrong).unwrap_err();
        assert!(err.contains("lib.avm") && err.contains("signature"), "{err}");

        // An incomplete closure names the missing cell instead of linking.
        std::fs::remove_file(cells.join("lib.avm")).unwrap();
        let err = cmd_load(&ts.ctx, &base).unwrap_err();
        assert!(err.contains("lib.avm"), "{err}");

        // Half a plan spec is a usage error, not a silent partial closure.
        let err = cmd_load(
            &ts.ctx,
            &[
                owner.display().to_string(),
                "--policy".into(),
                allow.display().to_string(),
                "--plan".into(),
                plan.display().to_string(),
            ],
        )
        .unwrap_err();
        assert!(err.contains("--cells"), "{err}");
    }

    /// A cell that may not call one of its declared symbols is refused *by
    /// that call*, at load, before any relocation or process — and a cell
    /// with no policy supplied is refused as a usage error rather than run
    /// ungated.
    #[cfg(unix)]
    #[test]
    fn load_refuses_a_denied_call_by_name() {
        let ts = TempStore::new();
        let cell = ts._dir.path().join("k.auvm");
        write(
            &cell,
            &cell_with_imports(&["uk_version", "uk_net_connect"], vec![]),
        );
        let deny_call = ts._dir.path().join("deny-call.cedar");
        write(
            &deny_call,
            b"permit(principal, action, resource);\n\
              forbid(principal, action == Action::\"Call:uk_net_connect\", resource);\n",
        );
        let path = cell.display().to_string();
        let err = cmd_load(
            &ts.ctx,
            &[
                path.clone(),
                "--policy".into(),
                deny_call.display().to_string(),
            ],
        )
        .unwrap_err();
        assert_eq!(
            err, "EPERM: capability denied for action \"Call:uk_net_connect\"",
            "the refusal names the call, not the cell"
        );

        // Same cell, policy permitting every call: the load proceeds.
        let allow = ts._dir.path().join("allow.cedar");
        write(&allow, b"permit(principal, action, resource);\n");
        let ok = cmd_load(
            &ts.ctx,
            &[
                path.clone(),
                "--policy".into(),
                allow.display().to_string(),
                "--resolve".into(),
                "uk_version=0x1000".into(),
                "--resolve".into(),
                "uk_net_connect=0x2000".into(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&ok).unwrap();
        assert_eq!(
            v["calls"],
            serde_json::json!(["uk_net_connect", "uk_version"])
        );
        assert_eq!(v["spawned"], false, "no --exec means no process");

        // No policy at all is a usage error: loading is when calls become
        // reachable, so there is no implicit allow.
        let err = cmd_load(&ts.ctx, &[path]).unwrap_err();
        assert!(err.contains("--policy"), "got: {err}");
    }

    /// The image is linked against the runtime table and the executor runs
    /// it through the runner: staged `cell.json`, resolved address in the
    /// text, derivation hash in the child's environment.
    #[cfg(unix)]
    #[test]
    fn load_links_and_runs_the_image_through_the_runner() {
        let ts = TempStore::new();
        let cell = ts._dir.path().join("k.avm");
        write(
            &cell,
            &cell_with_imports(
                &["uk_version"],
                vec![avm_exec::AvceReloc {
                    offset: 1,
                    addend: 0,
                    kind: "Abs8".into(),
                    symbol: "uk_version".into(),
                }],
            ),
        );
        let allow = ts._dir.path().join("allow.cedar");
        write(&allow, b"permit(principal, action, resource);\n");
        let executor = script(
            ts._dir.path(),
            "executor.sh",
            "#!/bin/sh\necho \"argv=$*\"\necho \"drv=$AVM_IMAGE_DRV\"\n\
             grep -o '\"text\": \"[0-9a-f]*\"' cell.json\n",
        );

        let json = cmd_load(
            &ts.ctx,
            &[
                cell.display().to_string(),
                "--policy".into(),
                allow.display().to_string(),
                "--resolve".into(),
                "uk_version=0xdead0000".into(),
                "--exec".into(),
                executor.display().to_string(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["spawned"], true);
        assert_eq!(v["exit"], 0);
        assert_eq!(v["calls"], serde_json::json!(["uk_version"]));
        let out = v["stdout"].as_str().unwrap();
        assert!(out.contains("argv=--image cell.json"), "got: {out}");
        assert!(
            out.contains(&format!("drv={}", hex(&[0x33u8; 32]))),
            "the derivation reaches the executor: {out}"
        );
        // 0xdead0000 little-endian at the reloc site.
        assert!(
            out.contains("0000adde00000000"),
            "the runtime address is patched into the image: {out}"
        );
    }

    /// The live half of the capability row: with a Cedar policy named, the
    /// build is judged on the cell's own capability table *before* a tool
    /// is spawned, and a refusal is an `EPERM` — the command fails with no
    /// compiler process ever having existed. The control run (same
    /// everything, a policy that permits `Execute`) does reach the compiler,
    /// so the refusal is Cedar's and not a broken fixture.
    #[cfg(unix)]
    #[test]
    fn a_cedar_denied_build_dies_with_eperm_before_any_tool_runs() {
        let ts = TempStore::new();
        let (helper, _avce) = {
            let (_austral, helper, avce, _plan, _argv) = fake_toolchain(ts._dir.path());
            (helper, avce)
        };
        let src = ts._dir.path().join("demo.aum");
        write(&src, b"module body Demo is end module body.\n");

        // A stand-in compiler that proves it ran by writing a marker, then
        // fails: reaching it is the thing the permit case asserts.
        let ran = ts._dir.path().join("compiler-ran");
        let aussie = script(
            ts._dir.path(),
            "marking-austral",
            &format!(
                "#!/bin/sh\nprintf ran >> '{mark}'\nexit 9\n",
                mark = ran.display()
            ),
        );

        let caps = ts._dir.path().join("caps.json");
        std::fs::write(
            &caps,
            r#"[{"uid":{"type":"ExecCap","id":"exec"},"attrs":{},"parents":[]}]"#,
        )
        .unwrap();
        let deny = ts._dir.path().join("deny.cedar");
        std::fs::write(&deny, "forbid(principal, action, resource);\n").unwrap();
        let allow = ts._dir.path().join("allow.cedar");
        std::fs::write(&allow, "permit(principal, action, resource);\n").unwrap();

        let build_args = |policy: &Path| {
            vec![
                src.display().to_string(),
                "--for".into(),
                "demo".into(),
                "--sign".into(),
                hex(&[2u8; 32]),
                "--caps".into(),
                caps.display().to_string(),
                "--policy".into(),
                policy.display().to_string(),
                "--austral".into(),
                aussie.display().to_string(),
                "--cps2cell".into(),
                helper.display().to_string(),
            ]
        };

        let denied = cmd_build(&ts.ctx, &build_args(&deny)).unwrap_err();
        assert!(
            denied.starts_with("EPERM: capability denied for action"),
            "a refused build must surface EPERM: {denied}"
        );
        assert!(denied.contains("Execute"), "{denied}");
        assert!(
            !ran.exists(),
            "the capability gate must refuse before any builder process exists"
        );
        assert!(
            open_store(&ts.ctx)
                .unwrap()
                .list_manifests()
                .unwrap()
                .is_empty(),
            "a denied build records nothing"
        );

        // Same refusal through the CLI entry point: non-zero exit, message
        // printed to stderr (which is where the `EPERM:` line lands).
        let mut argv = vec![
            "--store".to_string(),
            ts.ctx.store_dir.display().to_string(),
            "build".to_string(),
        ];
        argv.extend(build_args(&deny));
        assert_eq!(run(&argv).err(), Some(1));

        // Control: permit everything and the compiler *is* spawned, so the
        // refusal above came from the policy, not from the plumbing.
        let permitted = cmd_build(&ts.ctx, &build_args(&allow)).unwrap_err();
        assert!(!permitted.contains("EPERM"), "{permitted}");
        assert!(
            ran.exists(),
            "a permitted build must reach the compiler: {permitted}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn build_reports_a_compiler_failure_verbatim() {
        let ts = TempStore::new();
        let aur = script(
            ts._dir.path(),
            "failing-austral",
            "#!/bin/sh\necho 'error: totality rejected a while loop in demo' >&2\nexit 4\n",
        );
        let src = ts._dir.path().join("bad.aum");
        write(&src, b"module body Bad is end module body.\n");
        let err = cmd_build(
            &ts.ctx,
            &[
                src.display().to_string(),
                "--sign".into(),
                hex(&[1u8; 32]),
                "--for".into(),
                "bad".into(),
                "--austral".into(),
                aur.display().to_string(),
            ],
        )
        .unwrap_err();
        assert!(err.contains("austral compile failed"), "{err}");
        assert!(err.contains("totality rejected a while loop"), "{err}");
    }

    /// Loopback REST mock of the binary cache (real sockets, real HTTP).
    struct MockCache {
        base: String,
        manifests: Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>,
        puts: Arc<std::sync::atomic::AtomicUsize>,
        gets: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl MockCache {
        fn start() -> Self {
            use std::io::{Read, Write};
            use std::net::{TcpListener, TcpStream};
            use std::sync::atomic::{AtomicUsize, Ordering};
            use std::sync::Mutex;

            fn respond(s: &mut TcpStream, status: u16, body: &[u8]) {
                let reason = match status {
                    200 => "OK",
                    201 => "Created",
                    404 => "Not Found",
                    _ => "Error",
                };
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(body);
                let _ = s.flush();
            }

            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let chunks: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>> =
                Arc::new(Mutex::new(std::collections::HashMap::new()));
            let manifests = Arc::new(Mutex::new(std::collections::HashMap::new()));
            let puts = Arc::new(AtomicUsize::new(0));
            let gets = Arc::new(AtomicUsize::new(0));

            let (c2, m2, p2, g2) = (
                chunks.clone(),
                manifests.clone(),
                puts.clone(),
                gets.clone(),
            );
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut s) = stream else { continue };
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match s.read(&mut byte) {
                            Ok(1) => head.push(byte[0]),
                            _ => break,
                        }
                    }
                    let text = String::from_utf8_lossy(&head).to_string();
                    let mut lines = text.split("\r\n");
                    let req = lines.next().unwrap_or("");
                    let mut parts = req.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts
                        .next()
                        .unwrap_or("")
                        .trim_start_matches('/')
                        .to_string();
                    let mut len = 0usize;
                    for l in lines {
                        if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0u8; len];
                    if len > 0 && s.read_exact(&mut body).is_err() {
                        continue;
                    }
                    let (kind, hash) = match path.split_once('/') {
                        Some((k, h)) => (k.to_string(), h.to_string()),
                        None => (path.clone(), String::new()),
                    };
                    match (method.as_str(), kind.as_str(), hash.as_str()) {
                        ("POST", "chunks", "missing") => {
                            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                            let hashes = v["hashes"].as_array().unwrap();
                            let held = c2.lock().unwrap();
                            let missing: Vec<&str> = hashes
                                .iter()
                                .map(|h| h.as_str().unwrap())
                                .filter(|h| !held.contains_key(*h))
                                .collect();
                            drop(held);
                            let out = serde_json::json!({ "missing": missing }).to_string();
                            respond(&mut s, 200, out.as_bytes());
                        }
                        ("PUT", "chunks", h) => {
                            p2.fetch_add(1, Ordering::Relaxed);
                            c2.lock().unwrap().insert(h.to_string(), body);
                            respond(&mut s, 201, b"");
                        }
                        ("GET", "chunks", h) => {
                            g2.fetch_add(1, Ordering::Relaxed);
                            match c2.lock().unwrap().get(h).cloned() {
                                Some(d) => respond(&mut s, 200, &d),
                                None => respond(&mut s, 404, b""),
                            }
                        }
                        ("HEAD", "manifests", h) => {
                            let ok = m2.lock().unwrap().contains_key(h);
                            respond(&mut s, if ok { 200 } else { 404 }, b"");
                        }
                        ("PUT", "manifests", h) => {
                            m2.lock().unwrap().insert(h.to_string(), body);
                            respond(&mut s, 201, b"");
                        }
                        ("GET", "manifests", h) => match m2.lock().unwrap().get(h).cloned() {
                            Some(d) => respond(&mut s, 200, &d),
                            None => respond(&mut s, 404, b""),
                        },
                        _ => respond(&mut s, 404, b""),
                    }
                }
            });
            MockCache {
                base,
                manifests,
                puts,
                gets,
            }
        }

        fn seed_manifest(&self, id_hex: &str, bytes: Vec<u8>) {
            self.manifests
                .lock()
                .unwrap()
                .insert(id_hex.to_string(), bytes);
        }
    }

    #[test]
    fn push_then_pull_moves_only_missing_chunks() {
        use std::sync::atomic::Ordering;
        let mock = MockCache::start();
        let src = TempStore::new();
        let payload = vec![0x5Au8; 400 * 1024];
        let f = src._dir.path().join("p.bin");
        write(&f, &payload);
        let put_out = cmd_put(&src.ctx, &[f.display().to_string()]).unwrap();
        let id = put_out.split_whitespace().next().unwrap().to_string();
        let manifest = open_store(&src.ctx)
            .unwrap()
            .get_manifest(&unhex(&id).unwrap())
            .unwrap();

        let pushed = cmd_push(
            &src.ctx,
            &[id.clone(), "--remote".into(), mock.base.clone()],
        )
        .unwrap();
        let pv: serde_json::Value = serde_json::from_str(&pushed).unwrap();
        assert_eq!(
            pv["chunks_sent"].as_u64().unwrap() as usize,
            manifest.chunks.len(),
            "first push sends every chunk"
        );
        assert_eq!(mock.puts.load(Ordering::Relaxed), manifest.chunks.len());

        // Idempotent: a second push travels nothing.
        let again = cmd_push(
            &src.ctx,
            &[id.clone(), "--remote".into(), mock.base.clone()],
        )
        .unwrap();
        let av: serde_json::Value = serde_json::from_str(&again).unwrap();
        assert_eq!(av["chunks_sent"], 0);
        assert_eq!(mock.puts.load(Ordering::Relaxed), manifest.chunks.len());

        // Fresh store pulls: only missing chunks are requested.
        let dst = TempStore::new();
        let out_file = dst._dir.path().join("restored.bin");
        let pulled = cmd_pull(
            &dst.ctx,
            &[
                id.clone(),
                "--remote".into(),
                mock.base.clone(),
                "-o".into(),
                out_file.display().to_string(),
            ],
        )
        .unwrap();
        let ev: serde_json::Value = serde_json::from_str(&pulled).unwrap();
        assert_eq!(
            ev["chunks_fetched"].as_u64().unwrap() as usize,
            manifest.chunks.len()
        );
        assert_eq!(
            mock.gets.load(Ordering::Relaxed),
            manifest.chunks.len(),
            "exactly one GET per missing chunk"
        );
        assert_eq!(
            std::fs::read(&out_file).unwrap(),
            payload,
            "bytes round-trip"
        );
        assert!(
            dst.ctx
                .store_dir
                .join("manifests")
                .join(format!("{id}.mfx"))
                .exists(),
            "pulled manifest persisted"
        );
    }

    #[test]
    fn eval_consults_the_remote_tier() {
        let mock = MockCache::start();
        let ts = TempStore::new();
        let plan_path = ts._dir.path().join("plan.json");
        std::fs::write(
            &plan_path,
            r#"{"derivations":[
                {"name":"base","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["aa"],"env_vars":[]},
                {"name":"app","version":"1","builder_symbol":"stdenv.build","dependencies":["base"],"sources":[],"env_vars":[]}
            ]}"#,
        )
        .unwrap();
        let hashes: serde_json::Value = serde_json::from_str(
            &cmd_drv_hash(&ts.ctx, &[plan_path.display().to_string()]).unwrap(),
        )
        .unwrap();
        let base_id = hashes["resolved"]["base"].as_str().unwrap().to_string();
        // The remote already holds `base`'s built cell.
        mock.seed_manifest(&base_id, b"remote-cell".to_vec());

        let json = cmd_eval(
            &ts.ctx,
            &[
                plan_path.display().to_string(),
                "--remote".into(),
                mock.base.clone(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["remote_hits"], serde_json::json!(["base"]));
        assert_eq!(v["store_hits"], serde_json::json!([]));

        // An explicit target still parses next to the `--remote URL` pair
        // (the URL value must not be mistaken for the target).
        let scoped = cmd_eval(
            &ts.ctx,
            &[
                plan_path.display().to_string(),
                "app".into(),
                "--remote".into(),
                mock.base.clone(),
            ],
        )
        .unwrap();
        let sv: serde_json::Value = serde_json::from_str(&scoped).unwrap();
        assert_eq!(sv["resolved"], 2, "whole dependency closure of `app`");
    }

    /// The OCaml exporter now writes a per-derivation `totality` record
    /// ("proven" under `--pkg-totality=true`, otherwise "unproven").
    /// `load_plan` must accept it (release contract point 2), a v0 plan
    /// without the field must still parse, and the record must not move
    /// any Merkle id — drv-hash stays byte-identical either way.
    #[test]
    fn plan_totality_record_is_kept_but_never_rehashes() {
        let ts = TempStore::new();
        let mk = |totality: &str| {
            format!(
                r#"{{"derivations":[
                    {{"name":"base","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["aa"],"env_vars":[],"totality":"{totality}"}},
                    {{"name":"app","version":"1","builder_symbol":"stdenv.build","dependencies":["base"],"sources":[],"env_vars":[],"totality":"{totality}"}}
                ]}}"#
            )
        };
        let p = |fname: &str, body: &str| {
            let f = ts._dir.path().join(fname);
            std::fs::write(&f, body).unwrap();
            f.display().to_string()
        };
        let with_prov = p("prov.json", &mk("proven"));
        let with_unprov = p("unprov.json", &mk("unproven"));
        let v0 = p(
            "v0.json",
            r#"{"derivations":[
                {"name":"base","version":"1","builder_symbol":"stdenv.build","dependencies":[],"sources":["aa"],"env_vars":[]},
                {"name":"app","version":"1","builder_symbol":"stdenv.build","dependencies":["base"],"sources":[],"env_vars":[]}
            ]}"#,
        );

        let h_prov = cmd_drv_hash(&ts.ctx, &[with_prov.clone()]).unwrap();
        let h_unprov = cmd_drv_hash(&ts.ctx, &[with_unprov.clone()]).unwrap();
        let h_v0 = cmd_drv_hash(&ts.ctx, &[v0.clone()]).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&h_prov).unwrap(),
            serde_json::from_str::<serde_json::Value>(&h_unprov).unwrap(),
            "the totality record may not move a Merkle id"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&h_prov).unwrap()["order"],
            serde_json::from_str::<serde_json::Value>(&h_v0).unwrap()["order"],
            "a v0 plan must resolve identically"
        );
        // eval also tolerates the record (same load_plan path).
        let e = cmd_eval(&ts.ctx, &[with_prov]).unwrap();
        assert!(e.contains("\"store_hits\":[]"), "{e}");
    }

    #[test]
    fn hot_bench_synthetic_reports_both_rows() {
        let ts = TempStore::new();
        let out = cmd_hot_bench(
            &ts.ctx,
            &[
                "--size".into(),
                "1M".into(),
                "--acquire".into(),
                "1000".into(),
                "--fault-ins".into(),
                "3".into(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mode"], "synthetic");
        assert_eq!(v["size_bytes"], 1024 * 1024);
        assert_eq!(v["acquire_n"], 1000);
        assert!(
            v["acquire_mean_ns"].as_f64().unwrap() > 0.0,
            "acquire mean must be measured: {out}"
        );
        assert_eq!(
            v["acquire_evictions"], 0,
            "a resident fast path must not evict: {out}"
        );
        assert_eq!(v["fault_in_n"], 3);
        let median = v["fault_in_median_ms"].as_f64().unwrap();
        assert!(median > 0.0, "fault-in must be measured: {out}");
        assert!(v["fault_in_min_ms"].as_f64().unwrap() <= median);
        assert!(median <= v["fault_in_max_ms"].as_f64().unwrap());
        // A 1 MiB cell is not the spec's 50 MB row, so no verdict is claimed.
        assert_eq!(v["fault_in_row_applies"], false);
        assert!(v["fault_in_under_15ms"].is_null());
        // The configured store must stay untouched by a synthetic probe.
        assert!(!ts.ctx.store_dir.join("manifests").exists(), "{out}");
    }

    /// `--require` (the CI smoke) must pass when both rows hold and fail on
    /// a regressed row with the row named — the failure branch is judged on
    /// the report, so a slow host is not needed to exercise it.
    #[test]
    fn hot_bench_require_gates_both_latency_rows() {
        let passing = serde_json::json!({
            "acquire_mean_ns": 31.0, "acquire_under_1us": true,
            "fault_in_row_applies": true, "fault_in_median_ms": 6.5,
            "fault_in_under_15ms": true
        });
        assert_eq!(latency_regression(&passing), None);

        let slow_acquire = serde_json::json!({
            "acquire_mean_ns": 1300.0, "acquire_under_1us": false,
            "fault_in_row_applies": true, "fault_in_median_ms": 6.5,
            "fault_in_under_15ms": true
        });
        let why = latency_regression(&slow_acquire).unwrap();
        assert!(why.contains("acquire") && why.contains("1300"), "{why}");

        let slow_fault_in = serde_json::json!({
            "acquire_mean_ns": 31.0, "acquire_under_1us": true,
            "fault_in_row_applies": true, "fault_in_median_ms": 21.5,
            "fault_in_under_15ms": false
        });
        let why = latency_regression(&slow_fault_in).unwrap();
        assert!(why.contains("fault-in") && why.contains("21.5"), "{why}");

        // A cell that is not the spec's 50 MiB one has no fault-in verdict,
        // so it cannot fail that row.
        let small = serde_json::json!({
            "acquire_mean_ns": 31.0, "acquire_under_1us": true,
            "fault_in_row_applies": false, "fault_in_median_ms": 21.5,
            "fault_in_under_15ms": null
        });
        assert_eq!(latency_regression(&small), None);

        // And the flag is wired end to end: a real synthetic run passes.
        let ts = TempStore::new();
        cmd_hot_bench(
            &ts.ctx,
            &[
                "--size".into(),
                "1M".into(),
                "--acquire".into(),
                "100".into(),
                "--fault-ins".into(),
                "2".into(),
                "--require".into(),
            ],
        )
        .expect("--require must pass when the measured rows hold");
    }

    #[test]
    fn hot_bench_measures_a_named_cell() {
        let ts = TempStore::new();
        let f = ts._dir.path().join("probe.bin");
        write(&f, &vec![0x5Au8; 256 * 1024]);
        let put = cmd_put(
            &ts.ctx,
            &[f.display().to_string(), "--name".into(), "probe".into()],
        )
        .unwrap();
        let id: String = put.split_whitespace().next().unwrap().to_string();
        let out = cmd_hot_bench(
            &ts.ctx,
            &[
                id.clone(),
                "--acquire".into(),
                "50".into(),
                "--fault-ins".into(),
                "2".into(),
            ],
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mode"], "cell");
        assert_eq!(v["manifest"], id);
        assert_eq!(v["size_bytes"], 256 * 1024);
        assert_eq!(v["fault_in_row_applies"], false);
        assert!(v["fault_in_median_ms"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn size_parser() {
        assert_eq!(parse_size("4096").unwrap(), 4096);
        assert_eq!(parse_size("256K").unwrap(), 256 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert!(parse_size("bogus").is_err());
    }
}
