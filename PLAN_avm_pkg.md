# PLAN_avm_pkg — `avm-pkg`, the content-addressed package engine

**Status (2026-11-18): Sprints 1–5 shipped and measured. The capability
row is live on both halves: `avm-pkg build --policy` Cedar-authorizes the
cell's capability table and dies with `EPERM` before any tool process
exists (§4.13), and `avm-pkg load` gates a cell's *own* calls — `Execute`,
then one `Call:<symbol>` request per import it declares, against the
cell's own table — before relocating or spawning anything (§4.14).
Task 2.1 (OCaml totality pass + derivation export), the freshness-carrying
manifest / closure-guarded warm-path tier (§4.2, §5 item 2), the
end-to-end `build` (Austral source → plan.json + CPS → `cps2cell` → signed
`.avm` cell in the store, §4.9), the bootstrap payload digests (§4.10), the
hot-store latency probes and the Nix comparison (§4.12) all shipped.**

This document is the execution record for the owner's avm-pkg engineering
spec (received 2026-10-17). It tracks what was built, what was measured,
what was deliberately designed differently and why, and exactly where the
next agent picks up. Honesty rule from PLAN_valen §5 applies verbatim: a
task that cannot run is recorded as *not covered*, never as passing.

Provenance: everything here is implemented from the written spec — now
in-repo as `docs/pkg_manager_plan.md`. No external source tree was
available in this workspace (the packfile-engine prior art the spec names,
Vykar, has no checkout here), so `avm-cas` is an original implementation
of the spec's documented design; no external code was read or copied.
Per workspace policy #4 both provenance claims are rowed in this repo's
`ATTRIBUTION.md`: the *design-level* Vykar adaptation (spec §3.2's "Vykar
Adaptation") and the one algorithm-level row, the SplitMix64 mixer behind
the gear table (§4.1).

## §1 What exists

```
safestos/pkg/                    cargo workspace (offline-only deps; own Cargo.lock)
├── avm-cas/    dual-tier storage: cold CAS + ephemeral hot store
│   ├── chunker.rs     FastCDC-style CDC, min 16K/avg 64K/max 256K, deterministic gear table
│   ├── engine.rs      append-only packfiles, canonical manifests, atomic index, compacting GC
│   ├── compression.rs LZ4 (size-prefixed) with STORE fallback; unknown codec = hard error
│   ├── crypto.rs      Ed25519 manifest signatures over canonical unsigned encodings
│   ├── hot_store.rs   mmap-backed "read-only swap" page cache, LRU, zero-writeback eviction
│   └── remote.rs      Sprint 5.1 REST/S3-shaped binary cache: missing-only push/pull,
│                      client-side digest re-verification, fail-closed manifest order
├── avm-eval/    pure derivation-DAG evaluation
│   ├── hash.rs        BLAKE3 input-hash calculus (length-prefixed, sorted fields)
│   ├── graph.rs       Plan / Merkle resolution / topo order / cycle detection
│   └── memo.rs        L0 closure-guarded name lookup + L1 session memo
│                      + L2 id-keyed store presence + L3 remote tier
├── avm-exec/    Sprint 3 executor
│   ├── cell_builder.rs AVCE -> `.avm` (AVMC) serialize/parse/verify, §3.4 layout
│   ├── runner.rs       Cedar-gated, `unshare`-isolated builder spawn (task 3.1)
│   └── linker.rs       relocations resolved against hot-store-mapped provider cells
└── avm-cli/     `avm-pkg` binary: init, put, get, stat, store-stats, hot-stats,
                 gc, drv-hash, eval, build, push, pull — all 12 spec
                 subcommands wired (see §4.9)
```

Also in this tree but not part of this plan's surface: `avm-virtio` and
`avm-vmm` (VMM/device integration), and the Cranelift helper
`safestos/cranelift/src/bin/cps2cell.rs`, which `avm-pkg build` spawns
(spec §6.2 workspace separation: a process, not a linked crate).

Bootstrap recipes live as `stdlib/pkg/{Derivation,Stdenv,Bootstrap}.au*`;
the totality pass shipped as a flat `lib/Totality.ml` — see §2 and §4.8.

## §2 Task status

| spec task | status | evidence |
|---|---|---|
| 1.1 FastCDC chunker | **done** | `chunker.rs`; 64 MiB single-byte mutation acceptance in the default suite |
| 1.2 Packfile engine & manifests & LZ4 | **done** | `engine.rs` + `compression.rs`; roundtrip, dedup, reopen, GC tests |
| 1.3 Ed25519 signatures | **done** | `crypto.rs`; sign/verify/tamper/wrong-key/unsigned-fails-closed tests |
| 1.4 Ephemeral hot store | **done** | `hot_store.rs`; LRU test, 2 GiB/1 GiB zero-writeback acceptance, and the two §3 latency rows (30 ns acquire, 6.5 ms 50 MB fault-in) |
| 2.1 Totality pass (OCaml) | **done** | `lib/Totality.ml` (rules 1–5), gated on `--pkg-totality=true` at `Compiler.ml` between linearity and derivation export; `test/TotalityTest.ml` 10/10; flag matrix: no flag/`=false` pass, `=true` rejects while (rule 1) and recursion (rule 2), `=yes` fails loudly; `lib/DerivAst.ml` export + `test/DerivAstTest.ml` 11/11 incl. the `totality` plan record; end-to-end plan: 4 seed derivations `proven`, byte-identical on rerun, `drv-hash` byte-stable |
| 2.2 Merkle DAG engine | **done** | `graph.rs` + `hash.rs`; declaration/dep/source/env order-independence, cycle detection |
| 2.3 Memoization engine | **done** | `memo.rs`; warm/cold/id-keyed-presence tests, the L0 closure-guarded name tier (§4.2) and the 5,000-node acceptance |
| 3.1 Capability-checked sandbox | **done** | `runner.rs`; Cedar refusal is a real `EPERM` before any process (test `missing_network_cap_connect_is_ephem`), namespace entry is probed not assumed (`Isolation`), plus `sandboxed_child_cannot_reach_outside` / `permitted_builder_sees_staged_inputs`. Live on both paths: `avm-pkg build --policy` judges the cell's own capability table and runs the compiler/cps2cell through the runner (`a_cedar_denied_build_dies_with_eperm_before_any_tool_runs`, §4.13); `avm-pkg load` gates the cell's own calls one by one and runs the image through the runner (`load_refuses_a_denied_call_by_name`, `load_links_and_runs_the_image_through_the_runner`, §4.14) |
| 3.2 Cranelift `.avm` cell builder | **done** | `cell_builder.rs` (AVMC layout, mandatory Ed25519 trailer) + `cps2cell` in the bridge; CLI `build` ingests AVCE (or spawns the helper for `.cps`) and stores the cell under its derivation. `build_and_verify_cell_layout`, `avce_roundtrip_and_bounds` |
| 3.3 Hot-store dynamic linking | **done** | `linker.rs`; `resolves_symbol_from_mapped_provider_cell`, and the refusal paths (`unresolved_symbol_is_a_hard_error`, `unsupported_reloc_kind_is_a_hard_error`) |
| 4.1 CLI | **done** | all 12 spec subcommands real: `build`, `push`, `pull` wired in `main.rs`, plus `load` (the cell loader, §4.14) and `hot-bench` for the two latency rows (22 CLI tests, including the loopback-remote paths, both probe modes, `--require`, the CI smoke gate, and the shipped policy fixtures) |
| 4.2 Bootstrap package set | **done** | `stdlib/pkg/Bootstrap.aum` pins the four-recipe dependency graph (`bootstrap_seed` → `musl` → `busybox`/`hello`), `Stdenv.aum` the builder, and each recipe's real source-cell identity (tcc 0.9.27, musl 1.2.5, busybox 1.36.1, hello 2.12; ingested, round-tripped, and exported into plan.json with `totality: proven` — §3, §4.10). Hermetic test `avm-cas/tests/bootstrap_recipes.rs` parses the module, checks the four pinned ids are distinct 32-byte identities, exercises the name+version+content contract on generated payloads, and (ignored, real data) hydrates each pinned cell and compares it to its tarball |
| 5.1 Remote S3/REST sync | **done (HTTP shape)** | `remote.rs`; missing-only push/pull, tampered-cache rejection, fail-closed manifest order; CLI `push`/`pull` and `eval --remote` (L3). SigV4/TLS deferred (§4.11) |
| 5.2 Memory-pressure stress | **done (as spec'd shape)** | 2 GiB mapped vs 1 GiB budget acceptance, `--ignored` release test |
| 5.3 Benchmarks vs Nix | **done** | 1,000-node warm evaluation **4.33 ms** vs `nix-instantiate --eval --strict` **36.68 ms** (and **48.31 ms** for `nix-instantiate`); 10 one-byte rebuilds of a 1 MiB payload occupy **1.77 MB** vs Nix's **10.49 MB**. Method and caveats in §4.12 |

## §3 Acceptance measurements (this host, 2026-10-17; Nix rows 2026-11-18)

| criterion (spec §5) | target | measured | verdict |
|---|---|---|---|
| FastCDC dedup, minor source patch | ≥ 85 % chunks reused | **98.8 %** (79/80 on a 4 MiB payload, 1-byte edit) | pass |
| CDC resync, 64 MiB single-byte mutation | > 95 % chunks match | **99.9 %** (1218/1219 chunks kept, 1219 chunks ≈ 54.5 KiB avg) | pass |
| Warm evaluation, 5,000-node DAG | < 20 ms | **10.87 ms** store-warm / **2.07 ms** session-warm (release) | pass |
| Hot-store eviction, 2 GiB cells vs 1 GiB budget | 0 bytes cold writeback | pack sizes **and** mtimes byte-unchanged across the hydration/eviction storm; LRU victims dropped; every cell still reconstructible | pass |
| Hot cell acquire (fast path) | < 1 µs | **30–51 ns** mean over 100,000 acquisitions of a resident cell across runs (map hit + LRU tick + `Arc` clone only; no evictions) | pass |
| 50 MB evicted-cell fault-in | < 15 ms | **6.5 ms** median of five (6.46–6.51 ms across runs; samples 5.8–7.2 ms), after a prime pass; hydration only, cold pages warm | pass |
| Warm evaluation, 1,000-node DAG, **vs Nix** | < 20 ms and faster than Nix | **4.33 ms** median of 7 (3.03–4.79) vs `nix-instantiate --eval --strict` **36.68 ms** (34.20–41.37, **8.5×** slower) and `nix-instantiate` (evaluation + `.drv` writes) **48.31 ms** (43.17–62.75, **11.2×**) | pass |
| Disk after 10 minor rebuilds, **vs Nix** | no full-copy growth per rebuild | 1 MiB payload, one byte changed per version: store grows **1.05 MB → 1.77 MB** (78.6 KiB per rebuild, **92.3 %** of rebuild bytes reused), vs Nix's **10.49 MB** (10 full 1 MiB nars, no chunk reuse) — avm-pkg uses **83.1 % less** | pass |
| Capability gating (`EPERM` w/o `NetworkCap`) | blocked at type + OS level | **live build path**: `build --policy` on a policy that does not permit `Execute` exits 1 with `EPERM: capability denied for action Execute` and no builder process exists (a marker-writing stand-in compiler never runs; nothing is recorded in the store); the permit control reaches the compiler, so the refusal is Cedar's. A permitted build's compiler runs in the granted namespaces, where an outbound connect finds no route (`sandboxed_child_cannot_reach_outside`). **Live load path**: `avm-pkg load --policy` asks `Execute`, then one `Call:<symbol>` per declared import, against the cell's own table — a cell that may run but may not call `au_alloc` exits 1 with `EPERM: capability denied for action "Call:au_alloc"` before relocation, and the permit control links the image and runs it through the runner | pass (both paths; the loader's executor is named by the caller, §4.14) |
| Language totality | 100 % of unbounded constructs rejected at compile time | **pass for the shipped v1 rule set** (while / call cycles / foreign, embed, fptr): rejected at compile time under `--pkg-totality=true` — red probe `BadDrv.aum` (`rule 1`) leaves `rotten` absent from plan.json, control without the flag compiles and records `unproven` | pass |

Suite: `cargo test --workspace` → **137 passed, 0 failed, 6 ignored**, exit 0
(2026-11-18; per crate: avm-cas 31 + the `bootstrap_recipes` binary (1
passed, 1 ignored), avm-cli 22, avm-eval 22, avm-exec 14, avm-virtio 19,
avm-vmm 28; the six ignored are the heavy/timing acceptances: hot-store
×4, the 5,000-node warm-eval, and the real-payload Bootstrap round trip,
each exercised separately in release mode per the commands below). The CLI suite covers
`build` (signed cell, derivation + closure from `--plan`, refusal without
`--sign`/`--drv`, and the Austral-source path against a stand-in compiler
that records its argv: the CPS/plan/totality flags, the emitted plan's
identity + closure, `--no-totality` omitting the pass, and a compiler
failure reported verbatim), `push`/`pull` against a loopback REST cache
(first push sends every chunk, a second sends 0; a fresh pull fetches
exactly the missing chunks and the bytes round-trip), `eval --remote` (L3
reports `base` while L2 stays empty) and the L0 path end to end (a built
cell is adopted under its name; a patched source makes the record stale
and the evaluation falls back to hashing). The loader is covered at both
levels: `avm-exec`'s four tests check the `EPERM` ordering, the per-symbol
refusal, the runtime-table link (and its hard error when the table is
empty), and that the executor sees the image with a scrubbed environment;
the CLI's two cover the same through `cmd_load`, including the "no
`--policy` is a usage error" rule.

The four heavy hot-store acceptances ran together in **4.93 s** (release,
`--test-threads=1`, `$AVM_HOT_DIR=/dev/shm`): 2 GiB mapped over a 1 GiB
budget with every cold pack byte- and mtime-unchanged, 10.19 GiB churned
through a 64 MiB budget (326 evictions, 41 rounds) with the same
zero-writeback proof, and the two latency rows above.

**End-to-end `build`, measured on this host (2026-11-17).** With the
bridge built (`make bridge`, 7m33s) and the existing `./austral` binary:

```sh
LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release \
AVM_AUSTRAL=$PWD/austral \
AVM_CPS2CELL=$PWD/safestos/cranelift/target/release/cps2cell \
./safestos/pkg/target/debug/avm-pkg --store /tmp/avm-e2e/store build \
  stdlib/pkg/Derivation.aui,stdlib/pkg/Derivation.aum \
  stdlib/pkg/Stdenv.aui,stdlib/pkg/Stdenv.aum stdlib/pkg/Bootstrap.aum \
  --for hello --name hello --version 2.12 --sign <seed-hex32>
```

produced a **6,632-byte** cell (`4 functions`, `1 import`, `64
relocations`, x86_64) stored as manifest
`bfcb5723…0f3b66` with `drv_hash 4ca0d69d…f35d23cf` and closure
`1f71fb11…5e0ed3be`; `stat` shows both hashes, `get` returns the cell
byte-exactly through the hot store (`cmp` clean), and `eval plan2.json
hello` reports `closure_hits:["hello"]` with `visited:[]` — the stored
build is adopted without re-hashing the derivation.

**Bootstrap payloads, measured (2026-11-18).** The four source tarballs
were fetched over TLS at the versions `Bootstrap.aum` names, ingested with
`avm-pkg put <tarball> --name <pkg> --version <ver>`, and then hydrated
back out of the store and compared byte-for-byte with the tarball's
sha256 — all four match:

| recipe | tarball (upstream sha256) | store cell |
|---|---|---|
| `bootstrap_seed` (tcc) | `tcc-0.9.27.tar.bz2` (`de23af78…ceb9c`) | `55b85fce…e4da11` |
| `musl` | `musl-1.2.5.tar.gz` (`a9a118bb…c75e4`) | `40532f98…bfcad3` |
| `busybox` | `busybox-1.36.1.tar.bz2` (`b8cc24c9…de314`) | `bf28e188…9876d` |
| `hello` | `hello-2.12.tar.gz` (`cf04af86…bad0ab`) | `3321eda4…04f1` |

Recompiling the recipes with `--emit-derivations --pkg-totality=true`
yields a plan.json whose four entries carry those `sources` and
`totality: proven`; `avm-pkg drv-hash plan.json hello` resolves the chain
(`b073a1cc…cef98` for `hello`). The tarballs live in `target/pkg-sources/`
and the populated store in `target/pkg-store/` (both git-ignored); the
cell digests are reproducible from the `put` commands because the
identity covers name+version+content. Checked, not assumed: repeating the
four `puts` into a **fresh** store printed exactly the four digests pinned
in `Bootstrap.aum` — re-measured 2026-11-18 on a store rebuilt from scratch
by the commands below, where the four upstream `sha256sum`s also match the
prefixes above (`de23af78…`, `a9a118bb…`, `b8cc24c9…`, `cf04af86…`). The
`#[ignore]`d acceptance fails loudly with `NOT COVERED (never a pass)` when
those payloads are absent, rather than returning: a returning test is
reported `ok` by the harness, which would record an unrun check as a pass.

```sh
# from the repo root
mkdir -p target/pkg-sources target/pkg-store
curl -sSL -o target/pkg-sources/tcc-0.9.27.tar.bz2 \
  https://download.savannah.gnu.org/releases/tinycc/tcc-0.9.27.tar.bz2
curl -sSL -o target/pkg-sources/musl-1.2.5.tar.gz \
  https://musl.libc.org/releases/musl-1.2.5.tar.gz
curl -sSL -o target/pkg-sources/busybox-1.36.1.tar.bz2 \
  https://busybox.net/downloads/busybox-1.36.1.tar.bz2
curl -sSL -o target/pkg-sources/hello-2.12.tar.gz \
  https://ftp.gnu.org/gnu/hello/hello-2.12.tar.gz
A=safestos/pkg/target/release/avm-pkg
$A --store target/pkg-store init
$A --store target/pkg-store put target/pkg-sources/tcc-0.9.27.tar.bz2     --name tcc     --version 0.9.27
$A --store target/pkg-store put target/pkg-sources/musl-1.2.5.tar.gz      --name musl    --version 1.2.5
$A --store target/pkg-store put target/pkg-sources/busybox-1.36.1.tar.bz2 --name busybox --version 1.36.1
$A --store target/pkg-store put target/pkg-sources/hello-2.12.tar.gz      --name hello   --version 2.12
LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release ./austral compile \
  stdlib/pkg/Derivation.aui,stdlib/pkg/Derivation.aum stdlib/pkg/Bootstrap.aum \
  --target-type=c --no-entrypoint --output=target/pkg-sources/bootstrap.c \
  --use-cps-jit --allow-all --emit-derivations=target/pkg-sources/plan.json \
  --pkg-totality=true
```

Reproduce every number:

```sh
./examples/nix-pkg/run.sh          # Nix→AustralVM pairs: verify, package, execute
cd safestos/pkg
cargo test --workspace                                            # 137 passed / 6 ignored
AVM_HOT_DIR=/dev/shm CARGO_NET_OFFLINE=true \
  cargo test --workspace --release -- --ignored --test-threads=1  # all heavy/timing acceptances (hot-store ×4 + warm-eval + bootstrap payloads)
cargo test -p avm-cas  --release minor_edit   -- --nocapture      # 98.8 % dedup
cargo test -p avm-cas  --release acceptance_64mib -- --nocapture  # 99.9 % resync
cargo test -p avm-eval --release -- --ignored warm_eval --nocapture  # 10.9 ms / 2.1 ms
AVM_HOT_DIR=/dev/shm cargo test -p avm-cas --release \
  -- --ignored hot_acquire_under_1us --nocapture                    # 30–51 ns
AVM_HOT_DIR=/dev/shm cargo test -p avm-cas --release \
  -- --ignored fault_in_50mb --nocapture                            # 6.5 ms
AVM_HOT_DIR=/dev/shm ./target/release/avm-pkg hot-bench             # both rows as JSON
# Nix comparison (§4.12); target/nixbench is a generated, git-ignored dir:
./target/release/avm-pkg eval target/nixbench/plan1000.json n999    # 4.3 ms
nix-instantiate --eval --strict target/nixbench/graph1000.nix       # 36.7 ms
```

**Offline-cache caveat (2026-11-17).** `CARGO_NET_OFFLINE=true` now fails
before compiling anything: `ar_archive_writer v0.5.3` (a transitive
dependency of `psm`, pulled in through Cedar) is absent from this machine's
crate cache, so the suite was run without the flag. The workspace still
adds no new registry dependency — the CLI's `avm-exec` is a path dep, and
`ed25519-dalek` is the already-cached dev-dep.

## §4 Design decisions and deviations

**4.1 In-house FastCDC, not the `fastcdc` crate.** The crate is not in this
machine's offline cargo cache; the algorithm is fully specified by its
acceptance test instead. The gear table is SplitMix64-derived from the
fixed seed in `chunker.rs` (`GEAR_SEED`) and pinned by a test, so the
chunking is reproducible across builds — a deliberate, store-format-level
constant. Chunk digests are BLAKE3 of the *plaintext*, per spec §3.2.

**4.2 The name-keyed tier is closure-guarded, not bare (the spec's shape
made sound).** The spec says: *if a derivation hash exists in avm-cas, skip
traversing its sub-dependencies.* Adopting a stored id under a **name**
alone is unsound: edit a source, keep the name, and warm evaluation returns
a different target id than cold evaluation — a referential-transparency
violation that would poison every downstream derivation. Shipped (2026-11-17):

* **L2 stays id-keyed.** After a node's id is computed the store is asked
  `contains_id(id)`; hits are `store_hits` — the tier that skips *builds*.
* **L0 is the safe name-keyed tier.** A manifest written by `build --plan`
  records an input-**closure digest** beside `drv_hash`: BLAKE3 over the
  node's local fields plus its dependencies' *(name, closure)* pairs,
  recursively (`avm_eval::calculate_closure_digest`, computed by
  `Plan::closure_digest`). Warm evaluation computes the plan-side closure
  for a node and adopts the stored id **without descending into the
  subtree** only on an exact match. The contract that makes it sound is
  one-directional: `closure(a) == closure(b) ⇒ id(a) == id(b)` (induction
  over the subtree; the converse is never claimed, so a stale closure can
  only fail to match). A patched source, env pair, version, or dependency
  moves the closure and the node falls back to hashing.

Manifest format 2 carries the optional field; format-1 files still decode
and keep their byte-identical identity encoding (a `Manifest` remembers the
format it was read as), so existing stores stay valid — and a format-1
manifest has no closure, so it can never serve as an L0 hit. The honest
limit: the guard's own closure computation walks the subtree (one hash per
node). What L0 skips is the dependency-id chain and the per-node
memo/store bookkeeping, not the plan walk; a true traversal skip would
need ids carried in the plan (a lock file), which the spec does not define.

**4.3 Hot store backing: one anonymous mapping, frozen read-only (2026-11-17).**
Hydration reserves `total_size` bytes with `memmap2`'s **safe**
`MmapMut::map_anon`, fills the pages from the cold store, then freezes them
with the safe `make_read_only`. There is no backing file: eviction is a pure
drop, zero writeback holds by construction, no scratch file can leak, and
the mapping is page-aligned (4 KiB asserted). Contract kept: page-aligned,
zero writeback on eviction, cold store untouched; `$AVM_HOT_DIR` is now the
store's scratch/accounting directory (`hot-stats`), not a hydration target.

This replaces the earlier "tempfile + `MAP_PRIVATE` + immediate unlink"
design, chosen at the time to avoid an extra unsafe site. The first real
measurement of the spec's 50 MiB fault-in row showed that design at
**16.1 ms** median against a 15 ms target: the scratch write through
`write_all` was the floor (~15 ms for 50 MiB; parallel `write_all_at` did
not help, tmpfs serializes the copy). Writing through a mapping instead
measured ~6 ms, so the scratch file is gone — and with it the subsystem's
only `unsafe` block, since both `map_anon` and `make_read_only` are safe
APIs. Spec §6.3 is now satisfied by having zero `unsafe` in `pkg/`.
A second fix came with it: chunk reads go through
`ChunkStore::for_each_chunk`, which opens each packfile once and reads each
chunk with one positional read (the per-chunk `open`/`seek`/3-read shape
was measurable syscall overhead), while handing each decoded chunk straight
to the caller instead of retaining the cell in a `Vec<Vec<u8>>` (retention
cost a page fault per fresh page and was slower than streaming).

Profiling the remaining ~6 ms (2026-11-18) split it as: index lookup +
mapping reservation ~0.08 ms; one single-threaded read+decompress+verify
pass over the cell 11.8–12.5 ms (so the 16-way split is worth ~2×); a
single-threaded 50 MiB memcpy into a fresh mapping 10–14 ms (page faults
dominate); full parallel hydrate 5.0–6.0 ms. That rules out the obvious
next step: an interleaved A/B of “decompress into a `Vec`, then memcpy into
the mapping” against “decompress and verify directly in the mapping”
(added, measured, then reverted) showed **no** difference — 5.62/5.75 ms,
6.24/6.11 ms, 5.66/5.70 ms for old/new — because the copy overlaps other
workers' reads and faults rather than adding serial time. Hydration is
memory- and fault-bound, not copy-bound, so the plain `for_each_chunk`
path stays (no unmeasured code shipped).

**4.4 Determinism everywhere (spec §6.4).** Manifests and the chunk index
encode through fixed-order canonical binary (hand-rolled, bounds-checked),
never through unordered iteration; GC compacts in sorted-digest order;
plans iterate `BTreeMap`s; env/sources/deps are sorted before BLAKE3. The
test `declaration_order_is_irrelevant` and `drv-hash` byte-stability pin
it. Identity excludes the signature, so signing never moves a cell's hash.

**4.5 Compression split as spec'd; zstd deferred with the remote.** LZ4
(size-prefixed) for local packs, STORE fallback for incompressible chunks;
zstd level 3/19 was scoped by the spec to cold/remote archival (Sprint 5)
and is not stubbed — an unknown codec id fails loudly at decode.

**4.6 Offline dependency set.** blake3, dashmap, ed25519-dalek, lz4_flex,
memmap2, tempfile, serde/serde_json — all already in the machine's cargo
cache; no network was used (`CARGO_NET_OFFLINE=true`), and the workspace
carries its own `Cargo.lock`.

**4.7 CLI honesty (spec §6).** Every subcommand is now wired, and the
refusal paths that remain are input-shaped, not stubs: `build` refuses
without a signature seed or a derivation identity, and without a runnable
`cps2cell` for `.cps` input it names the helper it looked for and how to
build it. `hot-stats` reports budget/hot-dir/scratch and says plainly that
live residency is per-process (the CLI hydrates and exits), rather than
inventing a number.

**4.8 Totality scheduling + plan record (release contract).** The v1
checker is heuristic (rules 1–5 are sound over-approximations: they reject
what they cannot *prove* bounded), so the requirement is scoped — the
relaxation lives at exactly one place, the `if !pkg_totality` call in
`Compiler.ml` immediately after linearity and before derivation export.
Nothing else runs it: ordinary typechecking is untouched (probe matrix:
no flag / `=false` → rc 0; `=true` → rules 1–2 reject; bare spelling still
works; `=yes` fails loudly), the liquid totality gate only examines
pragma-carrying functions, and `lib/Linear/` was never touched (spec §6.2).
Flag default is off in both the parser and `Compiler.pkg_totality =
ref false`. Running with `--pkg-totality=true` *selects* the compilations
into the vetted non-recursive set; everything not selected leaks into the
plan deterministically: `DerivAst` stamps each derivation
`"totality": "proven" | "unproven"` (pure function of invocation + module,
byte-identical reruns, last declaration of a name wins the merge, v0 plans
without the field decode as `unproven`, unknown values fail loudly).
`avm-eval::Derivation` carries the same record (`Option<Totality>`,
`is_proven()` false for absent) and deliberately keeps it **out of** the
Merkle id calculus, so recording status never re-hashes a plan — pinned by
`plan_totality_record_is_kept_but_never_rehashes`. Deviation from the
spec: the pass lives at flat `lib/Totality.ml`, not `lib/Pkg/Totality.ml`
(`lib/dune` uses an explicit module list; a subpackage would need its own
stanza for no gain). Non-goal per the contract: no per-`.aum` caches —
plan.json is the only carrier of this information.

**4.9 CLI completion (2026-11-17).** `build`, `push`, `pull` are wired to
the subsystems that already existed; nothing is stubbed. `build` accepts
Austral sources (`mod.aum`, or `mod.aui,mod.aum` — one or more, imports
listed *before* their importers because that is the order the compiler
resolves modules in), `.cps` IR, or an `.avce` container. For sources it
runs the compiler as
`austral compile … --target-type=c --no-entrypoint --use-cps-jit
--allow-all --emit-cps=… --emit-derivations=… [--pkg-totality=true]`
(flag, env `AVM_AUSTRAL`, or `austral` on `PATH`), takes the identity and
closure from the emitted plan (or `--plan`/`--drv`), converts the CPS with
`cps2cell` (flag, `AVM_CPS2CELL`, or `PATH`; a separate process, so the
pkg workspace keeps its pure-crate separation), requires an explicit
`--sign` seed (the cell trailer is mandatory), and stores the cell under
`drv_hash`. The totality pass runs by default and its record is reported;
`--no-totality` opts out and reports `unproven`.

Two recorded limits of the current compiler surface: `--emit-cps` names a
single path and each module with functions overwrites it, so a
multi-module build's cell carries the function bodies of the **last**
module that defines functions (list the target module last); and a module
that defines no functions at all emits no CPS (`build` says so instead of
storing an empty cell). `push` uploads only the chunks the cache lacks (one
`POST /chunks/missing` round-trip) then the manifest; `pull` fetches the
manifest first *without persisting it* (new `Remote::fetch_manifest`)
because its chunk list is what the missing-only fetch needs, then
installs it only after every chunk is present — `put_manifest` stays the
fail-closed gate. `eval --remote` asks one `HEAD /manifests/<id>` per plan
node and reports L3 hits separately from L2, with ids bit-identical
whichever tier answers. `build --plan` also records the input closure, so
the cell can serve as an L0 hit later (§4.2); the manifest format moved to
2 for that optional field, and format-1 files remain readable with their
original identities.

**4.10 Bootstrap recipes pin the graph and their payloads.**
`stdlib/pkg/Bootstrap.aum` declares the four seed `Derivation` constants,
their literal dependency edges, and each recipe's source-cell identity.
The digests are the manifest identities `avm-pkg put` printed for the four
upstream tarballs at the versions the recipes name (tcc 0.9.27, musl
1.2.5, busybox 1.36.1, hello 2.12), and the digest covers
name+version+content, so re-ingesting with the same name and version
reproduces it exactly. Every digest was hydrated back out of the store and
its bytes compared to the tarball's sha256 (all four match), and the
recipes compile to a plan.json whose entries carry those sources and
`totality: proven`. What the recipes still do not do is *execute* the
builders: `stdenv.build` is pinned by name, and the sandbox runner is
exercised separately (§4.7).

**4.11 Remote transport is plain HTTP for now.** The REST protocol maps
1:1 onto S3 object verbs, but TLS/SigV4 are deferred as client features:
the client is built without TLS and the acceptance tests run against a
loopback mock. (The earlier recording said this machine had no egress;
that stopped being true — the bootstrap payloads and the Nix comparisons
were downloaded — but it does not change the client's feature set.) A
pull re-verifies every chunk digest and the manifest identity before
anything reaches the store, so a lying cache is refused rather than
trusted.

**4.12 The Nix comparison (task 5.3), method and what it does not say.**
Nix 2.34.9 on the same host, warm file cache, median of 7 whole-command
runs. *Evaluation:* `target/nixbench/plan1000.json` is a 1,000-node chain
(`n0` … `n999`, each depending on the previous) fed to `avm-pkg eval
… n999`; the Nix side is `target/nixbench/graph1000.nix`, the same chain as
`builtins.derivation`s (each taking the previous `outPath` in its
environment), evaluated with `nix-instantiate --eval --strict` and, to
include the `.drv` writes a real build performs, plain `nix-instantiate`.
Both numbers are end-to-end process wall time, so each includes its tool's
startup; the derivation hashes and the memo table are not run in
isolation. The builders are never executed (instantiation only), so this
measures evaluation, not compilation — which is what the spec's row asks
for. *Storage:* ten versions of one 1 MiB incompressible payload, one byte
changed per version, through `avm-pkg put` and — because a sandboxed Nix
rebuild needs a store bash this machine does not have staged for that
purpose — `nix-store --add` of the same ten files. Both measure the same
property: what one 1 MiB output version costs to keep. Nix's answer is one
full nar per version (10,485,760 bytes; its store does not deduplicate
file contents at chunk granularity), avm-pkg's is the first copy plus
78.6 KiB per rebuild (chunk reuse). The generated inputs live in
`target/nixbench/` and are not committed; the commands in §3 regenerate
them.

**4.13 The live capability gate: what `--policy` actually enforces.**
`avm-exec`'s runner was written before anything called it: its Cedar check,
its probed `unshare` tiers and its scrubbed staging cwd were exercised only
by its own tests, so "a denied build dies with EPERM" was a library
property, not a CLI one. `build --policy <cedar.txt>` closes that gap
(2026-11-18):

* the cell's `--caps` table is parsed as Cedar entities and judged against
the policy *before* the input is read — the request is `Execute` on
  `Sandbox::"host"` for principal `Cell::"builder"`, i.e. the runner's own
  question;
* when the policy does not permit it, the command exits 1 with
  `avm-pkg: EPERM: capability denied for action Execute` and **no builder
  process exists** — the test's stand-in compiler writes a marker file and
  the marker is absent; nothing is written to the store either;
* when it does, the compiler and `cps2cell` are spawned *through* the
  runner: paths are made absolute (the child's cwd is now the staging
dir), the environment is scrubbed except `LD_LIBRARY_PATH` (the compiler's
  shared library is outside its loader's default path), and the child runs
  in whichever namespaces `Isolation::probe` measured. The test's control
  run uses the same fixture with a permitting policy and the compiler does
  run, so the refusal is the policy's, not a broken setup.

Deliberate limits, so the row is not over-claimed: (a) without `--policy`
there is no gate — a policy set cannot be guessed, and the CLI says so
rather than pretending; (b) a `--caps` table that is not Cedar entities
(`["net"]` in the older cells) is refused when a policy is present, since a
capability the authorizer cannot read is not an authorizable capability;
(c) this gates *builder* processes. Gating an executed cell's own calls is
the loader's job and still outstanding (§5 item 7).

Measured on this host with the real compiler, 2026-11-18:

```sh
printf '[{"uid":{"type":"ExecCap","id":"exec"},"attrs":{},"parents":[]}]' > /tmp/avm-caps.json
printf 'forbid(principal, action, resource);\n' > /tmp/avm-deny.cedar
LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release \
./safestos/pkg/target/release/avm-pkg --store /tmp/avm-e2e/store build \
  stdlib/pkg/Derivation.aui,stdlib/pkg/Derivation.aum stdlib/pkg/Bootstrap.aum \
  --for hello --sign <seed-hex32> \
  --caps /tmp/avm-caps.json --policy /tmp/avm-deny.cedar \
  --austral $PWD/austral --cps2cell $PWD/safestos/cranelift/target/release/cps2cell
# avm-pkg: EPERM: capability denied for action "Execute"        (exit 1)
```

Swapping only the policy for `permit(principal, action, resource);` and the
store path for a fresh one makes the same build succeed *inside* the
runner: 4 functions, 1 import, 76 relocations, `drv_hash b073a1cc…` (the
same derivation as the ungated bootstrap export, §4.10). The pair is the
row: the gate is the only difference between the two runs.

**4.14 The loader: gating a cell's own calls.** §4.13 fences the processes
that *build* a cell; a cell that is later loaded still has its own calls.
`avm-exec/src/loader.rs` and `avm-pkg load <cell.avm> --policy <cedar.txt>`
(2026-11-18) close that half:

* the cell is parsed from its own bytes and judged with **its own**
  capability table — nothing is read from a side file, so the table that
  ships inside the `.avm` is the table that decides;
* `Execute` is asked first, then **one request per declared import**
  (`Call:<symbol>`), deduplicated and sorted. Allowing one symbol is not
  allowing the others, and the refusal names the symbol:
  `EPERM: capability denied for action "Call:au_alloc"`, exit 1, thrown
  before any relocation is applied and before any process exists;
* only then is the image linked. `--resolve SYM=HEX` supplies the runtime
  symbol table a JIT hands its loader (`uk_*`/`uz_*` addresses); an import
  with no entry is the linker's hard "unresolved symbol" error, never a
  silent zero. `linker::link_with` is the new entry point that takes that
  table (owner definitions win, then provider cells, then externs);
* with `--exec <program>` the linked image is rendered to `cell.json` in
  the runner's staging directory and the program is spawned **through the
  runner**: granted namespaces, scrubbed environment, staging dir as cwd,
  derivation hash in `AVM_IMAGE_DRV`. The program is named absolutely for
  the same reason the gated build's tools are.

What it does *not* do, stated so nobody reads more into it: this crate has
no `unsafe`, so the loader never jumps into the cell's machine code itself —
the executor is the caller's program (a JIT, an emulator, a test harness),
and the property the loader enforces is that a forbidden call cannot be
reached, not that the executor is trustworthy. Multi-cell images are
and provider linking is part of the loader now: `load_with` takes
`--provider` cells, gates each by its own table, and `linker::link_cells`
relocates the whole image (§4.17). What remains the caller's job is the
mapping and the call — the executor that does that in this repo is
`examples/nix-pkg/exec-image.rs`.

Measured on this host, 2026-11-18, against the real bootstrap cell built in
§4.13 (4 functions, 1 import, 76 relocations):

```sh
printf 'permit(principal, action, resource);\nforbid(principal, action == Action::"Call:au_alloc", resource);\n' > /tmp/avm-deny-call.cedar
LD_LIBRARY_PATH=$PWD ./safestos/pkg/target/release/avm-pkg load /tmp/hello.avm --policy /tmp/avm-deny-call.cedar
# avm-pkg: EPERM: capability denied for action "Call:au_alloc"   (exit 1)
printf 'permit(principal, action, resource);\n' > /tmp/avm-allow.cedar
LD_LIBRARY_PATH=$PWD ./safestos/pkg/target/release/avm-pkg load /tmp/hello.avm --policy /tmp/avm-allow.cedar --base 0x400000 --resolve au_alloc=0x7f0000000000
# {"calls":["au_alloc"],"symbols":5,"text_bytes":4311,"spawned":false,…}
```

The same cell without `--resolve` reaches the link step and fails with
`unresolved symbol "au_alloc" (owner defines 4 symbol(s), providers supply 0,
the runtime table has 0)` — the call gate passed, the link honestly could
not. That is the shape of the whole feature: each question answers for
itself.

**4.15 Copyable policy fixtures (deny/allow).** The commands in §4.13 and
§4.14 used `/tmp` files; a reader should not have to reconstruct them. Four
files ship under `safestos/pkg/policies/`, and the CLI suite pins every one
of them (`shipped_policy_fixtures_parse_and_decide_what_they_say`), so an
example that drifts from its files goes red instead of misleading:

| file | what it says |
|---|---|
| `caps-exec.json` | Cedar entities: an `ExecCap` and a `NetworkCap` — the table shape a cell carries |
| `allow.cedar` | `permit(principal, action, resource);` — the control run |
| `deny-execute.cedar` | `forbid(…);` — refuses `Execute`, so the build never starts |
| `deny-call-au-alloc.cedar` | permits everything except `Action::"Call:au_alloc"` — the loader's per-call case |

The deny/allow **build** pair, run from the repository root (the second
command is the first with one flag changed):

```sh
LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release \
./safestos/pkg/target/release/avm-pkg --store /tmp/avm-demo/store build \
  stdlib/pkg/Derivation.aui,stdlib/pkg/Derivation.aum stdlib/pkg/Bootstrap.aum \
  --for hello --sign <seed-hex32> \
  --caps safestos/pkg/policies/caps-exec.json \
  --policy safestos/pkg/policies/deny-execute.cedar \
  --austral $PWD/austral --cps2cell $PWD/safestos/cranelift/target/release/cps2cell
# avm-pkg: EPERM: capability denied for action "Execute"          (exit 1)
# … swap deny-execute.cedar for allow.cedar and the same build succeeds.
```

And the loader's per-call case, on a cell built with `allow.cedar`
(`-o /tmp/hello.avm`):

```sh
LD_LIBRARY_PATH=$PWD ./safestos/pkg/target/release/avm-pkg load /tmp/hello.avm \
  --policy safestos/pkg/policies/deny-call-au-alloc.cedar
# avm-pkg: EPERM: capability denied for action "Call:au_alloc"   (exit 1)
LD_LIBRARY_PATH=$PWD ./safestos/pkg/target/release/avm-pkg load /tmp/hello.avm \
  --policy safestos/pkg/policies/allow.cedar --resolve au_alloc=0x7f0000000000
# {"calls":["au_alloc"],"symbols":5,"text_bytes":4311,…}      (exit 0)
```

**4.16 Joint test with the Nix→AustralVM verifier (2026-11-18).**
`examples/nix-pkg/` is the end-to-end test of this engine against
`dnx-verify` (`../dnx/crates/dnx-verify`), the tool that reduces a Nix
expression and an AustralVM fragment to DeltaNets and compares their
canonical kernel UNF digests. Fourteen translation pairs are authored
independently on each side: arithmetic, application (direct and curried),
booleans (`<`, `not`, `and`), float, `let` (single and multiple bindings),
negative arithmetic, a version computation, and one **declared mismatch**
(`1 + 2 * 3` against the left-to-right `((1 + 2) * 3)`, 7 vs 9) that the
manifest requires to differ and that is never packaged. `run.sh`
1. requires every expectation to be met (`13 equivalent, 1 different`),
2. requires a one-literal mutation to be reported **different** (exit 1,
   not the exit-2 "could not compare" path),
3. writes a derivation per package whose `sources` field is the *verified*
   Nix net digest,
4. builds each module into a signed cell with `build --plan --for`, and
   hydrates every cell back out byte-identically, and
5. **executes** each cell through `avm-pkg load --exec`: the call gate, the
   relocation and the sandboxed runner, with `examples/nix-pkg/exec-image.rs`
   (a caller-supplied executor compiled by the script) mapping the image and
   calling `main`. All thirteen cells return their Nix value (20, 42, 1,
   3.75, 42, 212, 6, 42, 7, 0, 1, 3, −2). Float64 is an IEEE-754 bit pattern
   in the same 64-bit word every CPS value uses: `CpsGen` writes
   `Int64.bits_of_float` (exact, not rounded), and `Compiler_cps` lowers
   Float64 `+ - * /` to the `au_f64_add`/`au_f64_subtract`/`au_f64_multiply`/
   `au_f64_divide` bridge primitives, which bitcast, compute in `f64` and
   bitcast back (the stub page maps them at the same addresses as `au_f*`,
   and the scripts resolve both spellings). The bridge test
   `float_helpers_bitcast_and_compute` pins the helpers. The execution step
   carries its own negative control (`examples/nix-pkg/linker-control.sh`):
   it requires the Cedar `Execute` denial to refuse the cell, then reverts
   `link_with`'s rel32 fix, rebuilds `avm-pkg`, and requires `load --exec` to
   stop returning 42 before restoring the source and rebuilding. CI runs it
   after `run.sh`, so the job cannot stay green on a broken linker or a gate
   wired to nothing.

The derivation identity is the engine's own: `avm-pkg drv-hash` / `build`
compute it from the plan's name/version/source fields (e.g. `apply` →
`753c4701…c3f774c5`), and the recorded closure is what makes the stored build
an L0 hit later. Writing the execution half found three engine defects, all
fixed at the cause: `link_with` subtracted the rel32 field size in addition
to Cranelift's `-4` addend, so every intra-cell call jumped four bytes past
its target (`apply`, `nested`, `version` segfaulted); and opcode `0x19`
(`not`) used a bitwise `bnot`, returning `-2` for `not true` — now `bxor 1`,
pinned by `not_opcode_complements_zero_and_one`. A third surfaced when the
float cell was executed: literals were rounded into the integer literal
opcode and Float64 `+` called the integer helper; both are fixed as above.
Remaining limits: the
verifier's fragment is closed terms over `Int64`/`F64`/`Bool`, so an
attribute-set derivation cannot be compared this way; the kernel translator
accepts `let x: T = e;` while this compiler requires `:=`, so the modules use
`:=` for both; and the kernel translator's statement grammar has no `if`,
so conditionals are absent from the set. Details are in
`examples/nix-pkg/README.md`; CI runs the script in the `nix-pkg` job.

**4.17 Multi-cell images with provider cells (2026-11-18).** A cell's import
no longer has to resolve to a runtime stub: `avm-pkg load <owner> --provider
<cell>` links the owner against a provider cell's exported symbols, gated by
the provider's own capability table (it runs in the same process, so its
calls are judged too). `linker::link_cells` builds one symbol table for the
whole image and applies every cell's relocations — owner *and* providers —
at each cell's base, which is what makes a provider that calls the owner or
the runtime work; `link_with` remains the owner-only entry point. Bases are
assigned deterministically by the CLI (owner base + 0x01000000, +0x02000000,
…), recorded per provider in `cell.json`, and the executor maps each provider
text there after checking it is page-aligned and does not overlap the owner
or the stub page.

The provider set can also come from the derivation plan: `--plan <json> --for
<name>` adds every derivation `<name>` depends on, dependency-first, looked up
in the store **by the derivation id the plan resolved** (a manifest records
its cell's `drv_hash`, so the id is the lookup key and there is no filename to
substitute). An explicit `--provider` whose file stem names that derivation
wins; a missing cell fails the load naming the derivation and id, and more
than one cell for the same derivation is refused as ambiguous. The plan is
metadata, not a payload — a `.avm` cell still does not carry its dependencies.

A provider contributes executable text, so structure is not a trust decision:
`load` requires `--key <pubkey-hex32>` whenever a provider is supplied and
checks the Ed25519 trailer of every provider (and of the owner) before
linking; `build` reports `"pubkey"` so a caller can verify what it just built.
A provider with no key is refused, and one signed by another builder fails the
signature check. Provider text and the owner are verified, never trusted by
layout alone. Each provider's report entry also lists `imports` (declared in
cell order), `calls` (what the gate judged, sorted de-duplicated) and
`resolved` (each call with the address it resolved to in the shared symbol
table, `null` if no reloc/definition supplied one); the owner's `calls` field
keeps its existing meaning.

`examples/nix-pkg/missing-provider.sh` pins the refusal for a dependency that
was never built: the load exits non-zero naming the derivation and the id the
plan resolved, before reading any owner cell (it needs no compiler; the owner
path is deliberately absent). CI runs it right after the multi-cell step.

`examples/nix-pkg/multi-cell.sh` is the end-to-end check. `MathLib` exports
`double`, `DoubleApp` imports it, the owner cell alone fails with `unresolved
symbol "double"`, and the plan closure resolves it so `double(42)` runs in
the provider's image and returns 84. The second half is a mutually-referencing
pair — owner `Callback` defines `tick` and calls `compute`, provider `Wave`
defines `compute` and calls `tick` — where module imports must be acyclic, so
both sides use `Foreign_Import` and are compiled separately; the owner alone
misses `compute`, and the linked image runs 42 → provider → owner `tick` →
`+1` = 53. C code generation sets `set_link_time_imports`: a symbol the JIT
cannot resolve is bound to a trap thunk instead of failing the compile, so a
cross-cell import is emitted in the CPS for the loader to resolve and no
unresolved-symbol diagnostic is logged; the thunk aborts if called, so
nothing can run through an unresolved address. JIT execution targets keep the
strict resolution. The provider and owner are compiled separately because
one CPS emission carries one module's functions — that separation is what
puts the callee in another cell. CI runs the script after the linker control.

## §5 Not covered — next steps in order

1. ~~**Totality pass (spec task 2.1)**~~ **DONE** (flat `lib/Totality.ml`,
   not the spec's `lib/Pkg/` — recorded as a deviation in §4.8). Shipped:
   rule 1–5 syntactic checks behind `--pkg-totality=true` (default off,
   both spellings accepted), derivation export in `lib/DerivAst.ml` with a
   per-entry `totality` plan record, tests in `test/TotalityTest.ml` +
   `test/DerivAstTest.ml`, and the §3 acceptance row. Integral to it:
   fixes behind the same gate — a cycle guard in `CodeGen.ml`'s
   declaration sort (self-referential structs used to hang) and a `MonoTy`
   universe escape in `TypeSystem.ml`'s `replace_variables` (crashed while
   instantiating monomorphs whose type arguments were themselves
   monomorphs). Constraint from spec §6.2 was honored: `lib/Linear/` is
   untouched.
2. ~~**Freshness-carrying manifest + guarded name lookup**~~ **DONE** —
   manifest format 2 records the input-closure digest beside `drv_hash`
   (`Manifest::closure`, `put_buffer_with_closure`), `Plan::closure_digest`
   computes the plan side, and `MemoTable`'s L0 tier adopts a stored id
   only on an exact closure match (§4.2). Recorded limit: the guard walks
   the subtree to hash its closure, so this skips the id chain and the
   per-node bookkeeping, not the plan walk itself.
3. ~~**`.avm` cell format + cell builder (task 3.2)**~~ **DONE — end to
   end** — `avm-exec/src/cell_builder.rs`, the bridge's `cps2cell`, and the
   CLI `build` path from Austral source through plan.json + CPS (§4.9,
   §3 measurement).
4. ~~**Hermetic runner (task 3.1)**~~ **DONE** — `avm-exec/src/runner.rs`
   probes namespace availability instead of assuming it and reports the
   exact isolation it entered; Cedar refusal surfaces a real `EPERM`
   before any process exists. Monitored caveat: inside the mount namespace
   the host tree stays visible (unmounting `/usr`,`/bin`,`/tmp` needs
   in-namespace mount surgery), so "forbid host paths" is still enforced
   by a scrubbed environment and staged cwd, not by unmounting.
5. ~~**Remote sync (5.1)**~~ **DONE (HTTP shape)** — `avm-cas/src/remote.rs`
   plus the CLI `push`/`pull`/`eval --remote` wiring; TLS/SigV4 and zstd
   remain deferred (§4.5, §4.11).
6. ~~**Bootstrap payloads (4.2)**~~ **DONE** — tcc 0.9.27, musl 1.2.5,
   busybox 1.36.1 and hello 2.12 ingested, digests pinned in
   `Bootstrap.aum`, each hydrated back and byte-compared, plan export
   re-run (§3, §4.10); `avm-cas/tests/bootstrap_recipes.rs` keeps the
   digest set and the round-trip contract under test (hermetic shape +
   ignored real-payload acceptance).
7. ~~**Cell-loader capability gating (§3 row, remaining half)**~~
   **DONE (gate; executor supplied by the caller)** — `avm-exec/src/loader.rs`
   asks `Execute`, then one `Call:<symbol>` per declared import, against the
   cell's own table, and refuses with `EPERM` naming the symbol before any
   relocation or process (§4.14). The linked image runs through the runner
   via a caller-named `--exec` program, because this crate carries no
   `unsafe` and therefore never jumps into cell text itself. Provider-cell
   linking is wired into the loader (`load_with`, `linker::link_cells`,
   §4.17) and exercised by `examples/nix-pkg/multi-cell.sh`.
8. ~~**Nix comparison (5.3)**~~ **DONE** — 1,000-node warm evaluation
   4.33 ms vs Nix's 36.68 ms (8.5×), ten rebuilds 1.77 MB vs 10.49 MB
   (83.1 % less); method and caveats in §4.12, rows in §3.

## §6 CI

No separate workflow was added (the spec adds none), but the existing
`build-and-test.yml` carries a `nix-pkg` job that runs
`examples/nix-pkg/run.sh` against a bridge build (verify → package →
round-trip → execute; §4.16) and an `avm-pkg` job — added after cross-project
review round 4 found the engine's unit tests green and run by nobody. It
runs `cargo test --workspace` (137 passed / 6 ignored, 7 binaries) and,
since 2026-11-18, the two §3 hot-store rows as a smoke:
`avm-pkg hot-bench --require` on a release build, which exits non-zero when
either row regresses and attaches the measured report. Everything else in
this plan stays a local-gate claim with the reproduction commands in §3;
the workspace aggregate gate covers this tree's docs.
