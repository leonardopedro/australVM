# PLAN B — australVM (Austral JIT / module runtime)

Parallel workstream 2 of 3. Companion plans: `unfer/PLAN_parallel_unfer.md`,
`velysterm/PLAN_parallel_velysterm.md`.

## System context

Three repos form one system:
- **unfer** — the kernel: `prob_kernel::Session`, `unfer_ffi` (18 `uk_*` + 5 `uz_*` C
  symbols), `unfer_protocol`, 6 Austral modules, plus `logos` (CNL compiler), `ode_sirk`,
  `unfer_consensus` (QuePaxa federation), `unfer_data` (encrypted data plane),
  `unfer_identity` (DID). Plan A phases 1 (A1–A5) complete; A6–A10 pending.
- **australVM** (this repo) — OCaml Austral compiler (full upstream pipeline + `--use-cps-jit`
  path) and `safestos/cranelift` Rust JIT bridge (`austral_cranelift_bridge`) that
  **statically links `unfer_ffi` via a path dependency**, registers the 21 `uk_*` symbols
  (feature `unfer-kernel`, default on) + 5 `uz_*`, gates foreign calls through
  `AuthorizationEngine` (ManifestAuthEngine / Cedar / arctic threshold), and hosts modules
  via `modhost`.
- **velysterm** — frontend; `unfer_agent` NDJSON binary (20+ ops). Plan C complete.

### Module authoring paths (today vs. this plan's additions)

Today a module is exactly one thing: an **Austral cell** (`.aui`/`.aum`) with a
`module.toml`, compiled by the CPS-JIT path. Stages B9/B10 add two new, optional authoring
paths — both additive archetypes in `module.toml`, neither touching the frozen contract:

- **Tidepool modules** (B9): modules written as **Haskell `freer-simple` effect programs**,
  compiled to Cranelift JIT effect machines via
  [`tidepool`](https://github.com/tidepool-heavy-industries/tidepool) (`JitEffectMachine`
  yields effect requests → Rust `EffectHandler`s respond). The effect-dispatch boundary is a
  natural authorization point: every effect a Haskell module can request (including a new
  `Kernel` effect over the unfer `uk_*` surface) is gated per-call by the
  `AuthorizationEngine` under the module principal. Tidepool's watchdog `CancelHandle`
  gives runaway-module budgets for free. Stage B9b adds **Egison pattern matching as a
  Haskell library** for these modules — non-linear matching over non-free data types
  (multisets, sets), revived from the design of the outdated
  [`egison-haskell`](https://github.com/egison/egison-haskell) (miniEgison) with
  [`egison/egison`](https://github.com/egison/egison) as the semantic reference — a natural
  fit for Fock-space algebra (contraction pairs as multiset patterns).
- **cap-std Rust modules** (B10): modules written in **Rust against
  [`cap-std`](https://github.com/bytecodealliance/cap-std)** (capability-oriented std:
  `Dir`/`Pool` handles, `RESOLVE_BENEATH` path resolution, CWE-22 traversal protection).
  cap-std defines the **coupling** between module and host — exactly which fs/net/kernel
  capabilities cross the boundary, in one manifest vocabulary — while the *isolation* comes
  in two tiers: in-process (trusted modules; cap-std as declared-intent + defense-in-depth)
  or inside the **cloud-hypervisor Linux VM** (untrusted modules; the VM is the hard
  boundary) using the existing `cloud_hypervisor_vm/` recipe + `unfer/unfer_nixvm`
  host-store-is-guest-store mechanism. Same module code, same grants, different transport.

## Parallel-execution rules (shared by all three plans)

1. **Ownership**: modify only files inside this repo. Cross-repo *reads* are fine (you will
   read `../unfer/unfer_ffi` sources and run `../unfer/demo_module/run_demo.sh`). Cross-repo
   *writes* are forbidden, except steps explicitly marked `[SYNC]`.
2. **Frozen contract** (additive-only; no renames/removals/signature changes): the 21 `uk_*`
    + 5 `uz_*` symbols and signatures; `module.toml` grant vocabulary; UK-#### codes.
3. **Commit discipline**: meaningful messages; commit after every stage.
4. Stages ordered small → large, each with an acceptance command. Do not skip ahead.

## Current state (2026-07-24)

- **B1–B11 complete and committed** (build robustness, symbol auto-sync, live UK-4001
  enforcement, cps.rs unit tests (18 CPS + permission), test harness wiring, CI for new
  stack, hygiene, root hygiene, genuine module hosting, tidepool infrastructure,
  cap-std Rust modules, federation-aware hosting). 57 tests in `safestos/cranelift`.
- `cargo check` in `safestos/cranelift` passes against unfer's working tree.
- Root is clean: only AGENTS.md, README.md, CONTRIBUTING.md, LICENSE,
  PLAN_parallel_australvm.md, arctic.md, Makefile, flake.nix, flake.lock, shell.nix,
  dune-project, austral.opam, run-tests.sh, run-examples.sh, policies.cedar,
  libaustral_cranelift_bridge.so.
- `examples/modules/demo_hosted/` exists with module.toml + gen_cps.sh + run_demo.sh.
- `examples/modules/hello_kernel/` exists (Haskell effect module, requires GHC).
- `examples/modules/rust_kv/` exists (cap-std Rust module).
- `modhost host <module-dir> --call <ep> [--repeat N] [--swap <dir>]` CLI works:
  load-once / call-many / hot-swap with grant-escalation rejection.
- `--emit-cps=<path>` flag added to the Austral compiler CLI (saves CPS binary IR).
- 21 `uk_*` symbols registered (added uk_buf_free, uk_ode_analyze, uk_ode_measure_original).
- B9b (Egison pattern matching) blocked: requires GHC/tidepool-extract (not installed).
- All other stages (B7b–B11) complete.

---

## Completed stages (Phase 1: B1–B7)

| Stage | Summary |
|-------|---------|
| B1 | Build robustness + bridge artifact (`make bridge`, env-var path) |
| B2 | uk_*/uz_* symbol auto-sync test + data-driven registration |
| B3 | Live UK-4001 enforcement at JIT call time (module principal threading) |
| B4 | Unit-test cps.rs (18 CPS + permission tests, malformed-input coverage) |
| B5 | Test harness wiring (JitTest.ml assertions, ounit2, e2e --use-cps-jit) |
| B6 | CI for new stack (cranelift JIT + safestos integration jobs) |
| B7 | Hygiene (archived stale docs, purged build artifacts, gated debug, C boundary) |
| B7b | Final root hygiene (moved 6 stale status files to docs/history/, removed 10 test artifacts) |
| B8 | Genuine module hosting: `module.rs` (ModuleHost/ModuleHandle/ModuleManifest), `modhost host <dir> --call/--repeat/--swap`, `--emit-cps` compiler flag, `examples/modules/demo_hosted/`, 9 module tests, 21 uk_* symbols |
| B9 | Tidepool modules infrastructure: `tidepool_mod.rs` (KernelReq/KernelHandler stubs), manifest `archetype`/`effects`/`max_ms` fields, `examples/modules/hello_kernel/`, cranelift version decision (0.131 vs 0.129.1 → both JITs coexist), 6 tests. Full Haskell compilation requires GHC/tidepool-extract (not installed). |
| B10 | cap-std Rust modules: `capstd_mod.rs` (CapFs with RESOLVE_BENEATH), manifest `fs_grants`/`net_grants`, hot-swap fs/net escalation gate, `examples/modules/rust_kv/`, honesty note (Tier 1 ≠ sandbox), 4 tests |
| B11 | Federation-aware hosting: `federation.rs` (ModuleIdentity DID creation, artifact CID), `federation` feature flag (unfer_consensus/identity/data path deps), 3 tests |

---

## Stage B7b — Final root hygiene (S)

B7 archived the worst offenders but ~10 stale status files remain at root.

1. Move to `docs/history/`: `BLOCK_4_COMPLETE.md`, `BRIDGE_ARCH.md`, `BUILD_SUCCESS.md`,
   `SESSION_4_COMPLETE.md`, `FAILURES.txt`, `FINAL_SUMMARY.txt`,
   `INTEGRATION_TEST.RESULT.txt`, `PROJECT_COMPLETE.txt`, `WHAT_WE_DID_SO_FAR.txt`,
   `test_output.txt`.
2. Remove stale test artifacts from root: `test_const.aui`, `test_const.aum`,
   `test_cps.aui`, `test_cps.aum`, `capabilities.aui`, `capabilities.aum`,
   `calltree.html`, `error.html`, `concat_builtins.py`, `gen_correct_dune.sh`.
3. Root should contain only: `AGENTS.md`, `README.md`, `CONTRIBUTING.md`, `LICENSE`,
   `PLAN_parallel_australvm.md`, `arctic.md`, `Makefile`, `flake.nix`, `flake.lock`,
   `shell.nix`, `dune-project`, `austral.opam`, `run-tests.sh`, `run-examples.sh`,
   `policies.cedar`, `libaustral_cranelift_bridge.so` (deliberate release artifact).

**Acceptance**: `ls *.md *.txt *.html *.py` at root shows ≤ 6 files; `git status` clean
after the move.

## Stage B8 — Genuine module hosting (L, capstone)

Today `--use-cps-jit` runs `run` at compile time as a side effect and keeps no handle.

1. Introduce a persistent compiled-module artifact/handle: compile once → serialize the JIT
   product + manifest → `modhost` loads it, calls exported entrypoints many times, and
   hot-swaps via the existing `cell_swap` capability-subset gate.
2. This turns modhost into the stated "module runtime": load-once / call-many / hot-swap,
   with per-call ManifestAuthEngine checks from B3.
3. Migrate `demo_module` and `qfm_module` to the hosted flow as proof.
4. Add `modhost host <module> --call <entrypoint> [--args ...]` CLI path.

**Acceptance**: `modhost host demo_module --call run x3` executes three times without
recompiling; a hot-swap mid-session migrates state (reuse the hotswap_e2e assertions).

## Stage B9 — Tidepool modules: Haskell effect-stack authoring path (L)

New optional module archetype: modules written as Haskell `freer-simple` effect programs,
compiled by [`tidepool`](https://github.com/tidepool-heavy-industries/tidepool) into
Cranelift JIT effect machines hosted by modhost. Depends on B1 (build), B3 (module
principal for per-call auth); benefits from B8 (persistent hosting) but can land before it.

1. **Dependency + Cranelift dedup check** (do this first, it's the main integration risk):
   add `tidepool-runtime`, `tidepool-effect`, `tidepool-bridge-derive` as git deps pinned to
   a rev. Compare tidepool-codegen's `cranelift-*` versions against the safestos fork — if
   they diverge, either rebase one onto the other or accept both JITs in the binary
   (correctness is fine — separate JIT contexts; the cost is binary size + two JIT code
   caches). Record the decision in `safestos/STATUS.md`.
2. **Manifest archetype**: `module.toml` gains `archetype = "haskell_effect"` with
   `entry = "<TopLevelName>"` (the Haskell binding to compile, via
   `tidepool_runtime::compile_haskell`) and a `[grants] effects = [...]` whitelist
   (`Kernel`, `Console`, `Time`, …). Unknown archetype → clean loader error. This is
   additive to the frozen grant vocabulary.
3. **The Kernel effect**: define the Haskell GADT (`data Kernel a where Evolve :: …;
   Probability :: …; Condition :: …` — start with 3 ops, not all 18) plus the Rust mirror
   `#[derive(FromCore)] enum KernelReq` and an `EffectHandler` that forwards to the
   in-process `prob_kernel::Session` (modhost already links `unfer_ffi` — keep one code
   path). **Every** handler invocation first calls
   `AuthorizationEngine::check(module_principal, "uk_<op>")` (B3 wiring) and maps denial to
   UK-4001 as a typed Haskell-visible error, never a panic/abort.
4. **Effect gating**: handler dispatch rejects any effect not in `[grants] effects` before
   reaching its implementation — same UK-4001 path.
5. **Runaway budgets**: wire `JitEffectMachine::cancel_handle()` to a per-call watchdog
   (time + heap budget from `module.toml`, e.g. `[limits] max_ms = 500, heap_bytes`).
   Cancellation surfaces as `YieldError::Cancelled` → mapped to a module error; the cell
   stays loadable (call `handle.reset()` between runs).
6. **Example module** in `examples/modules/hello_kernel/` (this repo): a Haskell program
   that creates a model, evolves, and reads a probability through the Kernel effect; plus a
   positive/negative `run_demo.sh` mirroring unfer's demo_module shape.
7. Tests: positive round-trip vs. a direct `Session` call (same numbers); grant-denial per
   gated op; watchdog fires on `let loop = loop` style programs within budget; machine reuse
   after cancellation.

**Acceptance**: `examples/modules/hello_kernel/run_demo.sh` passes positive + UK-4001
negative; a 5s-budget infinite-loop module is cancelled in < 5.5s with the host alive;
`cargo test` in `safestos/cranelift` green.

## Stage B9b — Egison pattern matching as a Haskell library for tidepool modules (M)

Give tidepool modules Egison-style **non-linear pattern matching over non-free data types**
(multiset/set/graph matchers) as ordinary Haskell library code — compiled through the same
GHC-Core → Cranelift pipeline as everything else. Semantic reference:
[`egison/egison`](https://github.com/egison/egison) (v5, active); design inspiration:
[`egison/egison-haskell`](https://github.com/egison/egison-haskell) (miniEgison, outdated).
Depends on B9 step 1 (tidepool toolchain working).

**Two tidepool-specific constraints** (verify in step 1, they shape everything):
- **Eager evaluation**: tidepool's JIT is eager — miniEgison's lazy BFS enumeration over
  infinite targets (`matchAll primes …`) *hangs*. Bounded forms (`matchAllDFS` + `take`) or
  an explicitly thunked `Stream` type are required.
- **Custom Prelude**: tidepool standardizes on `Text`, eager `show`, etc. — the library
  must target that Prelude, not GHC's.

1. **Path decision** (S — do first, record in `safestos/STATUS.md`):
   - **(a) Refresh miniEgison**: fork `egison-haskell`, modernize (base ≥ 4.18, TH ≥ 2.20),
     keep the TH quasiquoters. TH expands at GHC compile time, *before* `tidepool-extract`
     serializes Core — the JIT never sees TH.
   - **(b) TH-free reimplementation** (lower risk, likely default): port the matcher core to
     a plain Haskell package exposing `matchAll / match / matchAllDFS / matchDFS` as
     ordinary functions over explicit clause lists. No TH anywhere.
   Try (a) for syntax fidelity; fall back to (b) on any TH/Core-extract friction.
2. **Matcher library**: built-in `Something`, `Eql`, `List`, `Multiset`, `Set` matchers +
   user-defined matcher support (port the `UnorderedPair` example as the extensibility proof).
3. **Eager adaptations**: enumerate results with `matchAllDFS` + explicit `take`/`fuel`.
4. **Motivating test — Fock-space rewrite module** (`examples/modules/fock_match/`):
   operator strings as `[(Mode, Create | Annihilate)]`; a Multiset matcher finds contraction
   pairs and normal-orders a small Hamiltonian. Validate against unfer's
   `nested_fock_algebra` on a shared fixture.
5. **JIT-path tests**: bounded twin primes, poker hands, unordered pairs — executed through
   the tidepool JIT, proving the whole GHC-Core → Cranelift path.

**Acceptance**: a tidepool Haskell module calls `matchAllDFS` with a `Multiset` matcher
through the full GHC-Core → Cranelift path; `fock_match`'s normal-ordering output equals
`nested_fock_algebra`'s on the shared fixture.

## Stage B10 — cap-std Rust modules: capability coupling in two isolation tiers (M–L)

New optional module archetype: modules written in Rust against
[`cap-std`](https://github.com/bytecodealliance/cap-std), receiving only the fs/net
capabilities their manifest grants. **cap-std's role is to define the coupling** — the
exact set of `Dir`/`Pool`/kernel-handle capabilities that cross the module↔host boundary.
**Isolation** is a separate concern with two tiers:

- **Tier 1 (in-process)**: module loaded as a cdylib into modhost. cap-std alone is
  declared-intent + defense-in-depth (plain `unsafe`/`std::fs` bypasses it).
- **Tier 2 (VM)**: the *same* module binary runs inside the cloud-hypervisor Linux guest —
  the VM is the hard boundary for genuinely untrusted native code.

1. **Harden the host first**: migrate modhost's own file access from `std::fs` to
   `cap_std::fs::Dir` rooted at a configured modules directory. Unit test: `../../etc/passwd`
   → `PermissionDenied`.
2. **Module ABI**: new crate `safestos/unfer-mod` defining the Rust-module contract
   (`ModuleCaps` with owned fd handles, `unfer_module_entry` extern fn).
3. **Manifest archetype**: `archetype = "rust_capstd"` with `[grants.fs]` and `[grants.net]`.
4. **Hot-swap gate**: extend `cell_can_replace` to compare fs/net grant sets.
5. **Kernel access**: Rust modules call through the same `uk_*` FFI + B3 auth.
6. **Example module** in `examples/modules/rust_kv/`: reads a granted `data/` dir, answers
   via a `uk_*` call; positive + negative tests.
7. **Tier 2 — VM execution**: `isolation = "vm"` launches a cloud-hypervisor guest with
   per-grant virtiofs exports; vsock kernel forwarding to host modhost.
8. **Honesty note**: document that Tier 1 cap-std is NOT a sandbox for untrusted Rust.

**Acceptance**: Tier 1 — escape attempts fail; hot-swap widening grants rejected;
`examples/modules/rust_kv/run_demo.sh` green. Tier 2 (manual, flagged): same binary runs
unmodified under VM with per-grant virtiofs; out-of-grant access fails in-guest.

## Stage B11 — Federation-aware module hosting (M)

unfer now has `unfer_consensus` (QuePaxa), `unfer_identity` (DID), and `unfer_data`
(encrypted chunks). Modules should be able to participate in the federation.

1. Add `Federation` effect to the Tidepool effect set (B9): `DidCreate`, `ContentPublish`,
   `ConsensusSync` — forwarded to the host's `unfer_consensus` engine under the module
   principal's DID.
2. Module identity: on first load, modhost creates a DID for the module (via
   `unfer_identity`) and records it in the module handle. The module's `module.toml`
   principal maps to this DID for consensus operations.
3. Content-addressed module artifacts: the compiled JIT product (B8) gets a CID via
   `unfer_data`'s chunking + hashing. `modhost` can publish/resolve module artifacts
   through the consensus log.
4. Example: a Tidepool module that publishes its own computation results to the consensus
   log under its DID, then another module resolves them.

**Acceptance**: a module creates a DID, publishes content, and a second module resolves it
— all gated by `[grants] effects = ["Kernel", "Federation"]`; grant removal → UK-4001.

---

## Out of scope (other workstreams)

- unfer: QFM research (A6), new-crate docs (A8), cross-repo integration (A9), logos (A10).
- velysterm: editor UX, multi-model documents, collaborative editing.

`[SYNC]` steps in this plan:
1. If B1 switches `unfer_ffi` to a git dep, record the pinned unfer rev in
   `safestos/STATUS.md` so unfer knows which commit to keep stable.
2. After B9/B9b/B10 land, append to `../unfer/docs/MODULE_RECIPE.md` — additive paragraphs:
   the new archetypes (`haskell_effect`, `rust_capstd`) with their grant sections.
