# PLAN_avm_pkg — `avm-pkg`, the content-addressed package engine

**Status (2026-10-17): Sprints 1–2 core + CLI shipped and measured; Sprints 3–5 and
the OCaml totality pass not started — recorded, not covered.**

This document is the execution record for the owner's avm-pkg engineering
spec (received 2026-10-17). It tracks what was built, what was measured,
what was deliberately designed differently and why, and exactly where the
next agent picks up. Honesty rule from PLAN_valen §5 applies verbatim: a
task that cannot run is recorded as *not covered*, never as passing.

Provenance: everything here is implemented from the written spec. No
external source tree was available in this workspace (the packfile-engine
prior art the spec names, Vykar, has no checkout here), so `avm-cas` is an
original implementation of the spec's documented design; no external code
was read or copied. The single algorithm-level provenance row — the
SplitMix64 mixer behind the gear table (§4.1) — is recorded in this repo's
`ATTRIBUTION.md` as workspace policy #4 requires.

## §1 What exists

```
safestos/pkg/                    cargo workspace (offline-only deps; own Cargo.lock)
├── avm-cas/    dual-tier storage: cold CAS + ephemeral hot store
│   ├── chunker.rs     FastCDC-style CDC, min 16K/avg 64K/max 256K, deterministic gear table
│   ├── engine.rs      append-only packfiles, canonical manifests, atomic index, compacting GC
│   ├── compression.rs LZ4 (size-prefixed) with STORE fallback; unknown codec = hard error
│   ├── crypto.rs      Ed25519 manifest signatures over canonical unsigned encodings
│   └── hot_store.rs   mmap-backed "read-only swap" page cache, LRU, zero-writeback eviction
├── avm-eval/    pure derivation-DAG evaluation
│   ├── hash.rs        BLAKE3 input-hash calculus (length-prefixed, sorted fields)
│   ├── graph.rs       Plan / Merkle resolution / topo order / cycle detection
│   └── memo.rs        L1 session memo + L2 id-keyed store presence (build-skip tier)
└── avm-cli/     `avm-pkg` binary: init, put, get, stat, store-stats, hot-stats,
                 gc, drv-hash, eval (build/push/pull refuse honestly)
```

Plus: the spec's `stdlib/pkg/*.au` DSL, `lib/Pkg/Totality.ml`, `.avm` cell
format, hermetic runner, remote sync, and the bootstrap package set are
**not started** — see §5.

## §2 Task status

| spec task | status | evidence |
|---|---|---|
| 1.1 FastCDC chunker | **done** | `chunker.rs`; 64 MiB single-byte mutation acceptance in the default suite |
| 1.2 Packfile engine & manifests & LZ4 | **done** | `engine.rs` + `compression.rs`; roundtrip, dedup, reopen, GC tests |
| 1.3 Ed25519 signatures | **done** | `crypto.rs`; sign/verify/tamper/wrong-key/unsigned-fails-closed tests |
| 1.4 Ephemeral hot store | **done** | `hot_store.rs`; LRU test + 2 GiB/1 GiB zero-writeback acceptance (release, ~4.1 s) |
| 2.1 Totality pass (OCaml) | **not started** | design + confirmed hook facts in §5 |
| 2.2 Merkle DAG engine | **done** | `graph.rs` + `hash.rs`; declaration/dep/source/env order-independence, cycle detection |
| 2.3 Memoization engine | **done (as specified semantics; see §4.3)** | `memo.rs`; warm/cold/id-keyed-presence tests + 5,000-node acceptance |
| 3.1 Capability-checked sandbox | **not started** | §5 |
| 3.2 Cranelift `.avm` cell builder | **not started** | §5 |
| 3.3 Hot-store dynamic linking | **not started** | §5 |
| 4.1 CLI | **partial** | 9 of 12 spec subcommands real; `build`/`push`/`pull` refuse with plan pointers |
| 4.2 Bootstrap package set | **not started** | §5 |
| 5.1 Remote S3/REST sync | **not started** | §5 |
| 5.2 Memory-pressure stress | **done (as spec'd shape)** | 2 GiB mapped vs 1 GiB budget acceptance, `--ignored` release test |
| 5.3 Benchmarks vs Nix | **partial** | §3 has absolute numbers; Nix comparison not run |

## §3 Acceptance measurements (this host, 2026-10-17)

| criterion (spec §5) | target | measured | verdict |
|---|---|---|---|
| FastCDC dedup, minor source patch | ≥ 85 % chunks reused | **98.8 %** (79/80 on a 4 MiB payload, 1-byte edit) | pass |
| CDC resync, 64 MiB single-byte mutation | > 95 % chunks match | **99.9 %** (1218/1219 chunks kept, 1219 chunks ≈ 54.5 KiB avg) | pass |
| Warm evaluation, 5,000-node DAG | < 20 ms | **10.87 ms** store-warm / **2.07 ms** session-warm (release) | pass |
| Hot-store eviction, 2 GiB cells vs 1 GiB budget | 0 bytes cold writeback | pack sizes **and** mtimes byte-unchanged across the hydration/eviction storm; LRU victims dropped; every cell still reconstructible | pass |
| Hot cell acquire (fast path) | < 1 µs | *not measured* | not covered |
| 50 MB evicted-cell fault-in | < 15 ms | *not measured* (2 GiB acceptance hydrates 250 MiB cells well inside it, but no timed probe) | not covered |
| Capability gating (`EPERM` w/o `NetworkCap`) | blocked at type + OS level | blocked: executor not started | not covered |
| Language totality | 100 % of unbounded constructs rejected at compile time | blocked: `Totality.ml` not started | not covered |

Suite: `cargo test --workspace` → **42 passed, 0 failed, 2 ignored** (the two
ignored are the heavy acceptances above), exit 0. CLI exercised end-to-end
through the real binary (signed `put`, byte-exact `get` roundtrip, `gc`,
refusals).

Reproduce every number:

```sh
cd safestos/pkg
CARGO_NET_OFFLINE=true cargo test --workspace                     # 42 pass / 2 ignored
AVM_HOT_DIR=/dev/shm CARGO_NET_OFFLINE=true \
  cargo test --workspace --release -- --ignored --test-threads=1  # both heavy acceptances
cargo test -p avm-cas  --release minor_edit   -- --nocapture      # 98.8 % dedup
cargo test -p avm-cas  --release acceptance_64mib -- --nocapture  # 99.9 % resync
cargo test -p avm-eval --release -- --ignored warm_eval --nocapture  # 10.9 ms / 2.1 ms
```

## §4 Design decisions and deviations

**4.1 In-house FastCDC, not the `fastcdc` crate.** The crate is not in this
machine's offline cargo cache; the algorithm is fully specified by its
acceptance test instead. The gear table is SplitMix64-derived from the
fixed seed in `chunker.rs` (`GEAR_SEED`) and pinned by a test, so the
chunking is reproducible across builds — a deliberate, store-format-level
constant. Chunk digests are BLAKE3 of the *plaintext*, per spec §3.2.

**4.2 L2 memoization is id-keyed, not name-keyed (the spec's shape made
sound).** The spec says: *if a derivation hash exists in avm-cas, skip
traversing its sub-dependencies.* Adopting a stored id under a **name**
would adopt whatever inputs the store happened to build: edit a source,
keep the name, and warm evaluation returns a different target id than cold
evaluation — a referential-transparency violation that would poison every
downstream derivation. Shipped instead: ids are always recomputed (cheap —
that is what the 5,000-node measurement shows), and after each node's id is
known the store is asked `contains_id(id)`; hits are reported as
`store_hits`, which is exactly the tier an executor uses to skip *builds*.
The literal traversal-skip remains desirable and needs a freshness-carrying
manifest: record the input closure (sources + env + dep ids) beside
`drv_hash`, then a name-keyed lookup guarded by that digest is sound. That
is the first item in §5.

**4.3 Hot store backing: tempfile + `MAP_PRIVATE` + immediate unlink, not
memfd.** Same observable contract — page-aligned mapping (4 KiB asserted),
clean pages, zero writeback on eviction, scratch never leaks into the cold
store — without an extra unsafe site; `memmap2`'s wrapper is the only
`mmap` in the subsystem (spec §6.3 satisfied strictly: the crate is
`#![deny(unsafe_op_in_unsafe_fn)]` and contains exactly one `unsafe`
block, documented as the sanctioned mmap call). `$AVM_HOT_DIR=/dev/shm`
puts hydration on tmpfs as the spec's diagram wants; the 2 GiB acceptance
runs that way.

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

**4.7 CLI honesty (spec §6).** `build`, `push`, `pull` print why they
cannot work and which sprint owns them, exiting non-zero — no stub that
pretends. `hot-stats` reports budget/hot-dir/scratch and says plainly that
live residency is per-process until the Sprint 3 runtime integration,
rather than inventing a number.

## §5 Not covered — next steps in order

1. **Totality pass (spec task 2.1).** Confirmed integration facts: `lib/dune`
   uses an **explicit module list** (add the module there; spec's `lib/Pkg/`
   needs its own `dune` stanza or a flat `lib/Totality.ml`), function bodies
   with `while`/recursion live in the MAST stage (`Stages.ml`,
   `Monomorphize.ml` reference `MWhile`), CLI-flag plumbing precedent is
   `CliParser.ml` (`--target-type`, `--use-cps-jit` around lines 183–231),
   pipeline entry is `Compiler.ml`. Proposed v1 rules (sound over-
   approximations, each syntactic): package modules reject *all* `while`
   statements, any call cycle through SCC analysis, and `Foreign_Import` /
   `@embed` (no unmediated I/O until capabilities exist). Plus the JSON
   derivation export `avm-cli`'s `plan.json` already consumes. Constraint
   from spec §6.2: do not touch `lib/Linear/`. Not started: doing it half-
   wired would be worse than absent.
2. **Freshness-carrying manifest for §4.2's traversal-skip** (one manifest
   field + guarded name lookup; makes the spec's literal warm-path shape
   sound).
3. **`.avm` cell format + cell builder (task 3.2)** — header layout from
   spec §3.4 is fully specified; serialize/verify can be pure `avm-cas`
   functions before any Cranelift hookup; then the bridge.
4. **Hermetic runner (task 3.1)** — namespace isolation needs privileges
   this container may not grant; probe `unshare(CLONE_NEWNET)` first and
   record the result honestly.
5. **Remote sync (5.1)**, **bootstrap packages (4.2)**, **Nix comparison
   (5.3)**, hot-acquire/latency timing probes (§3 rows marked *not
   measured*).

## §6 CI

No workflow added (spec adds none; the australVM CI surface stays
linkcheck-only per PLAN_CROSS_PROJECT_REVIEW_2026-10-15.md §8). All claims
above are local-gate claims with the reproduction commands in §3; the
workspace aggregate gate covers this tree's docs.
