# PLAN B — australVM (Austral JIT / module runtime)

Parallel workstream 2 of 3. Companion plans: `unfer/PLAN_parallel_unfer.md`,
`velysterm/PLAN_parallel_velysterm.md`.

## System context

Three repos form one system:
- **unfer** — the kernel: `prob_kernel::Session`, `unfer_ffi` (18 `uk_*` + 5 `uz_*` C
  symbols), `unfer_protocol`, 6 Austral modules.
- **australVM** (this repo) — OCaml Austral compiler (full upstream pipeline + `--use-cps-jit`
  path) and `safestos/cranelift` Rust JIT bridge (`austral_cranelift_bridge`) that
  **statically links `unfer_ffi` via a path dependency**, registers the 18 `uk_*` symbols
  (feature `unfer-kernel`, default on) + 5 `uz_*`, gates foreign calls through
  `AuthorizationEngine` (ManifestAuthEngine / Cedar / arctic threshold), and hosts modules
  via `modhost`.
- **velysterm** — frontend; irrelevant to this plan except via the frozen contract.

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
2. **Frozen contract** (additive-only; no renames/removals/signature changes): the 18 `uk_*`
   + 5 `uz_*` symbols and signatures; `module.toml` grant vocabulary; UK-#### codes.
3. **Commit discipline**: meaningful messages; commit after every stage.
4. Stages ordered small → large, each with an acceptance command. Do not skip ahead.

## Current state (2026-07-18)

- unfer's working tree is green again (was broken by a mid-refactor; fixed today). This
  repo's `cargo check` in `safestos/cranelift` passes against it.
- **Stale bridge artifact is live**: committed `target/release/libaustral_cranelift_bridge.so`
  / `modhost` (built 2026-06-30) are older than `lib.rs` (2026-07-01). The OCaml binary links
  the stale `.so`.
- `cargo test` in `safestos/cranelift` compiles now, but `dune build test/` is broken
  (`ounit2` missing), and `bin/test_jit.ml` is a manual print-based test.
- ~30 contradictory root-level status files; committed `.o`/`.so`/backup files; upstream
  CI covers only the OCaml side — zero CI for the JIT/integration.
- Auth gap: JIT principal = *calling function* name; manifests grant by *module* name.
  Live UK-4001 enforcement is modhost-out-of-band only; the JIT demo runs `AllowAll`.

---

## Stage B1 — Build robustness + bridge artifact (S)

1. Replace the `unfer_ffi` **path dependency** with a git dep pinned to a commit
   (`{ git = "...", rev = "..." }`), or if offline operation is required, keep the path dep
   but add `safestos/cranelift/build.rs` guard that fails early with a clear message when
   `../../../unfer` doesn't compile (`cargo check -p unfer_ffi`). Decide, document the choice
   in `safestos/STATUS.md`.
2. Add a single `make bridge` (top-level Makefile) that: builds the cranelift bridge
   (release) → redeploys the `.so`/`.a` → rebuilds the OCaml binary. This kills the
   documented AGENTS.md stale-artifact gotcha.
3. Run it now; commit the rebuilt artifacts (they are already tracked — keep that policy,
   just make them current).
4. Remove the hardcoded machine-specific absolute path in `lib/dune:58` (use an env var with
   a sane default, e.g. `AUSTRAL_BRIDGE_DIR`).

**Acceptance**: fresh clone → `make bridge && dune build lib/ bin/` succeeds; the shipped
`.so` timestamp ≥ `lib.rs` mtime; `git grep "/media/leo" lib/` is empty.

## Stage B2 — uk_*/uz_* symbol auto-sync (S)

Symbol registration (`lib.rs:86-103`) is manually synced with unfer_ffi exports — already
drifted once (`uk_subscribe`/`uk_poll` registered but Austral bindings dropped).

1. Add `safestos/cranelift/tests/symbol_sync.rs`: parse the `pub extern "C" fn uk_*`/`uz_*`
   set from the unfer_ffi source (path dep location via `CARGO_MANIFEST_DIR` relative path,
   or from unfer's `EXPECTED_SYMBOLS.txt` once unfer Plan A3 lands) and assert it equals the
   set registered in `cranelift_init()`. Refactor registration into a data-driven table so
   the test can enumerate it.
2. Also assert handle marshalling invariants already relied upon (Int64 handles, ptr+len
   JSON buffers) in 2 linkage smoke tests — extend beyond `uk_version`/`uk_init` to at least
   `uk_model_create`/`uk_free` round-trip.

**Acceptance**: commenting out one registration makes the test fail; `cargo test` in
`safestos/cranelift` green.

## Stage B3 — Live UK-4001 enforcement (M)

Close the documented auth gap: the JIT must enforce module grants at call time, not just
modhost out-of-band.

1. Thread the *module name* through the CPS encoding: `CpsGen.ml` already keys foreign
   decls by decl_id — extend the binary IR call instruction (or add a per-module header
   field) with the module principal. Coordinate the IR format change with a version bump
   constant; old binaries must fail cleanly.
2. In `cps.rs::check_call_permission`, use the module principal (falling back to the
   function principal with a deprecation warning) when querying the `AuthorizationEngine`.
3. Wire `ManifestAuthEngine` (not `AllowAll`) as the default when a manifest is present;
   keep `AllowAll` only behind an explicit `--allow-all` flag with the existing stderr
   warning.
4. E2E: `demo_module` positive test still passes; a grant-removal variant yields UK-4001
   at JIT call time (not just at modhost load).

**Acceptance**: `modhost` with a manifest missing `uk_evolve` fails at the call site with
UK-4001; `../unfer/demo_module/run_demo.sh` passes unchanged.

## Stage B4 — Unit-test `cps.rs` (M)

The 629-line JIT core has zero unit tests.

1. Opcode round-trip tests: for each CLIF-producing arm, feed a minimal hand-built binary-IR
   buffer through the 3-pass compiler and assert the produced function's behavior by JIT-
   executing it (cranelift makes this cheap) or by structural CLIF comparison.
2. Permission-gate tests: `__`/`au_`/self calls pass freely; an ungranted `uk_*` call is
   denied; denied paths never partially emit code.
3. Malformed-input tests: truncated buffers, bad opcode, bad arity → clean error, no panic.

**Acceptance**: `cargo test` covers every opcode arm and both permission outcomes; a fuzz
loop of 10k random byte buffers produces no panic.

## Stage B5 — Test harness wiring (S–M)

1. Convert `bin/test_jit.ml` (~10 print-based tests incl. deep-recursion TCO) into
   assertion-based dune tests; wire into `run-tests.sh`.
2. Fix `dune build test/`: add `ounit2` to the nix shell / opam deps.
3. Add at least one `--use-cps-jit` entry to the 289-program e2e suite in
   `test-programs/suites/` (start with the programs exercising records/unions/pointers —
   Phase-8 features).
4. Fix the `noreturn` warnings in `safestos/runtime/scheduler.c`.

**Acceptance**: `dune build test/ && dune runtest` green; `run-tests.sh` includes a CPS-JIT
section that fails when the JIT branch is broken.

## Stage B6 — CI for the new stack (M)

Current GitHub workflow is upstream-only.

1. New job: `cargo build && cargo test` in `safestos/cranelift` (with the unfer checkout as
   a sibling, pinned per B1's decision).
2. New job: `safestos/test/hotswap_e2e` shell suite.
3. New job: `../unfer/demo_module/run_demo.sh` end-to-end (positive + UK-4001 negative).
4. Cache the OCaml + cranelift builds; keep upstream jobs untouched.

**Acceptance**: PR touching only `safestos/` triggers the new jobs; breaking an `uk_*`
registration fails CI.

## Stage B7 — Hygiene + de-fragilize the C boundary (S)

1. Archive the ~25 contradictory PHASE_*/STATUS/SUMMARY files into `docs/history/`; leave
   one current `STATUS.md`; delete the stale upstream `ROADMAP.md`; fix AGENTS.md
   "14 uk_* functions" → 18 (+5 uz_*) and the stale `Austral_core` note.
2. Purge committed build artifacts (`safestos/**/*.o`, stray `*.so`, test binaries,
   `combined.o`, `lib/CpsGen_backup.ml`) — except the deliberate release bridge artifacts
   from B1; add `.gitignore` entries.
3. Gate the `eprintln!` debug output in `lib.rs` behind a `CPS_DEBUG` env check.
4. Replace the hardcoded 64-byte `_jit_fn_ptr` offset in `au_set_cell_jit_ptr` with a
   C-exported setter or a `static_assert` against `vm.h` so a struct-layout change fails at
   build time.

**Acceptance**: root of repo contains ≤ 8 md files; `git ls-files | grep -E '\.(o|so)$'`
shows only the intended release artifacts; flipping two fields in `CellDescriptor` breaks
the build with a clear assert message.

## Stage B8 — Genuine module hosting (L, optional capstone)

Today `--use-cps-jit` runs `run` at compile time as a side effect and keeps no handle.

1. Introduce a persistent compiled-module artifact/handle: compile once → serialize the JIT
   product + manifest → `modhost` loads it, calls exported entrypoints many times, and
   hot-swaps via the existing `cell_swap` capability-subset gate.
2. This turns modhost into the stated "module runtime": load-once / call-many / hot-swap,
   with per-call ManifestAuthEngine checks from B3.
3. Migrate `demo_module` and `qfm_module` to the hosted flow as proof.

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
6. **Example module** in `australVM/examples/modules/hello_kernel/` (this repo — do **not**
   write into unfer): a Haskell program that creates a model, evolves, and reads a
   probability through the Kernel effect; plus a positive/negative `run_demo.sh` mirroring
   unfer's demo_module shape (grant removal → UK-4001 at effect-dispatch time).
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
[`egison/egison-haskell`](https://github.com/egison/egison-haskell) (miniEgison, outdated —
2020, TH-based, won't build on GHC 9.12 as-is). Depends on B9 step 1 (tidepool toolchain
working).

**Two tidepool-specific constraints** (verify in step 1, they shape everything):
- **Eager evaluation**: tidepool's JIT is eager — miniEgison's lazy BFS enumeration over
  infinite targets (`matchAll primes …`) *hangs*. Bounded forms (`matchAllDFS` + `take`) or
  an explicitly thunked `Stream` type are required.
- **Custom Prelude**: tidepool standardizes on `Text`, eager `show`, etc. — the library
  must target that Prelude, not GHC's.

1. **Path decision** (S — do first, record in `safestos/STATUS.md`):
   - **(a) Refresh miniEgison**: fork `egison-haskell`, modernize (base ≥ 4.18, TH ≥ 2.20),
     keep the TH quasiquoters (`matchAll`, `[mc| … |]`). TH expands at GHC compile time,
     *before* `tidepool-extract` serializes Core — the JIT never sees TH. Test: a
     `compile_haskell` module using `[mc| $x : #(x+1) : _ -> x |]`.
   - **(b) TH-free reimplementation** (lower risk, likely default): port the matcher core —
     `Matcher` typeclass, `MAtom`, backtracking search — to a plain Haskell package
     (`australVM/examples/modules/egison-matchers/` or the tidepool prelude dir), exposing
     `matchAll / match / matchAllDFS / matchDFS` as ordinary functions over explicit clause
     lists. No TH anywhere. Fits tidepool's "API is the prompt" principle (LLM-authored
     modules write plain Haskell).
   Try (a) for syntax fidelity; fall back to (b) on any TH/Core-extract friction. Either
   way the *semantics* (matchers, non-linear patterns, BFS/DFS) must match the Egison paper
   (arXiv:1808.10603) and miniEgison's `src/Control/Egison/*`.
2. **Matcher library**: built-in `Something`, `Eql`, `List`, `Multiset`, `Set` matchers +
   user-defined matcher support (port the `UnorderedPair` example as the extensibility
   proof). Preserve miniEgison's three criteria: non-linear patterns with backtracking,
   extensible matching algorithms, ad-hoc pattern polymorphism.
3. **Eager adaptations**: enumerate results with `matchAllDFS` + explicit `take`/`fuel`
   (twin primes: `take 8 (matchAllDFS primes (List Integer) …)`); if laziness is needed,
   implement a `Stream` with manually thunked tails only if tidepool's Core support allows
   — otherwise document "finite targets or bounded DFS only" in the module recipe.
4. **Motivating test — Fock-space rewrite module** (`examples/modules/fock_match/`, this
   repo): operator strings as `[(Mode, Create | Annihilate)]`; a Multiset matcher finds
   contraction pairs `a_i a†_i` and normal-orders a small Hamiltonian. Validate against
   unfer's `nested_fock_algebra` on a shared fixture (cross-repo *read* of an unfer test
   fixture is fine; results must agree exactly).
5. **JIT-path tests**: port miniEgison's test semantics — bounded twin primes, poker hands,
   unordered pairs, non-linear value patterns `#(x+1)` — executed **through the tidepool
   JIT** (not plain GHCi), proving the whole TH/Core/Cranelift path.
6. **Grants**: the library is pure Haskell — no new effects, no grant-vocabulary change.
7. **Documented follow-up (do not implement here)**: the full Egison interpreter
   (`egison/egison`, `hs-src/`) as an embedded library for the CAS/tensor-index-notation
   surface is heavy and its IO/parser footprint likely exceeds tidepool's Haskell subset;
   if the CAS is wanted later, prefer a gated `Egison` effect handler (B9 step 4 machinery)
   over embedding. Record this trade-off in the module docs.

**Acceptance**: a tidepool Haskell module calls `matchAllDFS` with a `Multiset` matcher
through the full GHC-Core → Cranelift path; poker-hand and bounded twin-prime tests pass
under the JIT; `fock_match`'s normal-ordering output equals `nested_fock_algebra`'s on the
shared fixture; no TH survives into the runtime path (extract-time only, if path (a)).

## Stage B10 — cap-std Rust modules: capability coupling in two isolation tiers (M–L)

New optional module archetype: modules written in Rust against
[`cap-std`](https://github.com/bytecodealliance/cap-std), receiving only the fs/net
capabilities their manifest grants. **cap-std's role is to define the coupling** — the
exact set of `Dir`/`Pool`/kernel-handle capabilities that cross the module↔host boundary,
expressed once in `module.toml`. **Isolation** is a separate concern with two tiers:

- **Tier 1 (in-process)**: module loaded as a cdylib into modhost. For trusted/semi-trusted
  modules — cap-std alone is declared-intent + defense-in-depth here (plain `unsafe`/
  `std::fs` bypasses it; say so in the docs).
- **Tier 2 (VM)**: the *same* module binary runs inside the cloud-hypervisor Linux guest —
  the VM is the hard boundary for genuinely untrusted native code. Reuses the existing
  substrate: `cloud_hypervisor_vm/` (this repo: NixOS `vm-perf`/`vm-sec` images, virtiofs
  `/nix` share, GPU passthrough, launch scripts) and `unfer/unfer_nixvm` (the
  `unfer-ffi`-in-guest flake + the content-addressed host-store-is-guest-store pattern).

Also hardens modhost itself against path traversal. Depends on B1; pairs naturally with
B8's persistent hosting (capability table lives in the module handle).

1. **Harden the host first**: migrate modhost's own file access (cell loading, manifest
   reading, state serialization paths) from `std::fs` to `cap_std::fs::Dir` rooted at a
   configured modules directory. Unit test: a `module.toml` path of `../../etc/passwd` or a
   symlink escaping the root → `PermissionDenied`, never an open. (CWE-22; this is valuable
   even if the archetype below slips.)
2. **Module ABI**: new crate `safestos/unfer-mod` defining the Rust-module contract:
   ```rust
   pub struct ModuleCaps { /* dir handles as owned fds, one per granted path */ }
   #[no_mangle] extern "C" fn unfer_module_entry(caps: *const ModuleCaps) -> i64;
   ```
   The host opens each granted path with `Dir::open_ambient_dir` (ambient use is fine *in
   the trusted host*), converts to owned fds, and passes them across the boundary; the
   module rebuilds `Dir::from_raw_fd` — from then on `dir.open("../escape")` and absolute
   symlinks fail with `PermissionDenied` (`RESOLVE_BENEATH`). This is exactly the
   WASI/cap-std fd-as-capability model.
3. **Manifest archetype**: `archetype = "rust_capstd"` with
   `[grants.fs] paths = [{ path = "data", access = "ro" }, { path = "tmp", access = "rw" }]`
   and optional `[grants.net] addrs = [...]` (cap-std `Pool`). Host validates every path
   exists *before* load; unknown archetype → clean error.
4. **Hot-swap gate**: extend `cell_can_replace`'s capability-subset check to compare fs/net
   grant sets — a replacement may drop or keep capabilities, never gain new ones.
5. **Kernel access**: Rust modules call the kernel through the same handle-based `uk_*`
   FFI already registered in `cranelift_init` — no new symbol surface; calls still pass
   through `check_call_permission` (B3) when the module is JIT-loaded, or through the host
   shim for native cdylib modules (document which path applies).
6. **Example module** in `australVM/examples/modules/rust_kv/`: a small cap-std module
   (uses `cap-directories`/`cap-tempfile` patterns) that reads a granted `data/` dir and
   answers via a `uk_*` call; positive + negative (`../escape`, widened-grant hot-swap)
   tests in its `run_demo.sh`.
7. **Honesty note in docs**: in Tier 1, cap-std is *not* a sandbox for untrusted Rust
   (`unsafe` / `std::fs` bypass it) — it is declared-intent + defense-in-depth, and the
   coupling definition. Untrusted native modules belong in Tier 2, where the VM supplies
   the isolation and cap-std still supplies the coupling. Say this explicitly in the module
   docs so nobody over-trusts the Tier-1 boundary.
8. **Tier 2 — VM execution**: add `isolation = "vm"` to the `rust_capstd` archetype
   (default `"in_process"`). On load, modhost: (a) packages the module as a Nix derivation
   (reuse `unfer_nixvm/flake.nix`'s `buildRustPackage` pattern — the module then appears
   inside the guest *for free* via the shared `/nix` virtiofs mount, no copy/skew);
   (b) launches/joins a `vm-sec-with-unfer` guest via the `cloud_hypervisor_vm` recipe;
   (c) materializes each `[grants.fs]` entry as its **own virtiofsd export** (one exporter
   per granted path, `ro`/`rw` per manifest — never a shared broader root) and each
   `[grants.net]` entry via the guest's slirp/tap allowlist; (d) a thin in-guest agent
   (`modhost-guest`, new small bin in this repo) maps those mounts to `cap_std::fs::Dir` /
   `Pool` and calls the module's `unfer_module_entry` — **module code is byte-identical to
   Tier 1**, only capability transport differs.
9. **Tier-2 kernel coupling**: the guest already has `libunfer_ffi` via `unfer_nixvm`.
   Default: `uk_*` calls forward over **vsock** to the host modhost, which applies the
   B3 `AuthorizationEngine::check(module_principal, …)` per call against one shared kernel
   `Session` state (UK-4001 on deny) — one kernel, one auth decision point, both tiers.
   Document the variant "in-guest kernel instance" (results never leave the sandbox,
   separate state) for high-isolation jobs.
10. **Tier-2 lifecycle + tests**: module exit → VM teardown (or VM pool reuse with
    per-module snapshot restore — reuse the cell `serialize.c` state format if practical);
    the `cell_can_replace` capability-subset check applies identically (compare grant sets
    on swap). Tests: module attempting `dir.open("/etc/passwd")` outside its export fails
    *inside the VM*; a fork-bomb/inf-loop module dies with its VM while the host stays up
    (this is the Tier-2 property Tier 1 cannot give); vsock kernel round-trip matches a
    direct in-host `Session` call. Gate VM tests behind an env flag (`AUSTRAL_VM_TESTS=1`)
    since they need `sudo`/device access — CI runs the Tier-1 ones.

**Acceptance**: Tier 1 — escape attempts (`..`, absolute symlink) fail in tests; a
hot-swap widening `[grants.fs]` is rejected by `cell_can_replace`; the host path-traversal
unit test passes; `examples/modules/rust_kv/run_demo.sh` green. Tier 2 (manual, flagged):
the same `rust_kv` binary runs unmodified under `isolation = "vm"` with per-grant virtiofs
exports; out-of-grant fs access fails in-guest; a hostile native module cannot survive VM
teardown; vsock-forwarded `uk_evolve` under a stripped grant → UK-4001 from the host.

---

## Out of scope (other workstreams)

- unfer: protocol docs, FFI symbol CI gate, module_builder tool, QFM research, and the
  canonical example modules (`demo_module` etc. stay there; B9/B10's examples live in
  *this* repo under `examples/modules/`).
- velysterm: agent ops, frontends.

`[SYNC]` steps in this plan:
1. If B1 switches `unfer_ffi` to a git dep, record the pinned unfer rev in
   `safestos/STATUS.md` so unfer knows which commit to keep stable.
2. After B9/B9b/B10 land and unfer Plan A2 has landed, append to
   `../unfer/docs/MODULE_RECIPE.md` — additive paragraphs only: the two new archetypes
   (`haskell_effect`, `rust_capstd`) with their grant sections (`effects`, `fs`, `net`),
   and the tidepool-module authoring notes from B9b (eager evaluation: finite targets or
   bounded `matchAllDFS` only; `Text` Prelude). If A2 hasn't landed, defer the paste and
   note it in the final report.
