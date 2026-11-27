# Umbrella / Rust Interop & Capability Runtime: Unified Production Plan

**Status: in progress, 2026-11-27.** This document merges
[`PLAN_umbrella_production.md`](PLAN_umbrella_production.md) and
[`PLAN_valen.md`](PLAN_valen.md) into one authoritative plan for AustralVM's
Rust interop, adapter runtime, capability enforcement, and Umbrella
orchestrator, and reconciles all shared seams with
[`PLAN_distributed_security.md`](PLAN_distributed_security.md),
[`PLAN_gpu_kernels.md`](PLAN_gpu_kernels.md),
[`PLAN_avm_pkg.md`](PLAN_avm_pkg.md), and
[`docs/spectrum_plan.md`](docs/spectrum_plan.md).

- **Foundation shipped (`PLAN_valen.md` P0–P3):** P1 group borrowing
  (`lib/AbstractionPass.ml`, `test-programs/suites/018-group-borrow`), P2
  `extern rust` Tier 1 (`RS_FFI_SYMBOLS`, `test-programs/suites/019-extern-rust`),
  and P3 `umbrella/` orchestrator (`umbrella/`).
- **Umbrella cards (`U00`–`U22`):** `U00`–`U22` all have measured
  implementations (recorded in [the execution record](docs/umbrella/EXECUTION.md)
  and §8 below), mostly `implemented with gaps` or `passed within stated profile`.
- **Shared seams shipped:** Distributed D08/D09/D17/D18 (`umbrella/src/{project,trust,frontend,main}.rs`,
  `umbrella/modules/{distributed_io,DistributedRequest.aum}`,
  `umbrella/tests/{distributed_request,distributed_tls}.rs`) and GPU G14
  (`umbrella/modules/GpuService.aum`).
- **Pending & open defects:** A 2026-11-27 source audit of `umbrella/` and
  `../rust` surfaced new findings **UR15–UR24** (§3) and three high-leverage
  remediation cards (**U00b**, **U06b**, **U10b**). Cards **U23–U32** remain
  pending.
- **Explicit non-claim:** **Not production-ready.** U23 (supervised workers),
  U24 (fuzz/sanitizers/mutations), U25 (locked packaging), U26 (isolation), U27
  (multi-arch), U28 (CI), U29 (runbooks), and U30 (budgets/release gate) have
  not been completed.
- **Paused work (Bend integration):** Per owner decision (2026-11-27), all
  Bend-language integration plans remain **paused** and excluded from this plan
  (§1.2). Only the purely Rust-side compiler-oracle design is absorbed into
  optional card `U32`, while `../rust` remains strictly a read-only reference.

Companion deliverables: [source review](docs/UMBRELLA_PRODUCTION_REVIEW.md),
[test matrix](docs/umbrella/TEST_MATRIX.md), and
[execution ledger](docs/umbrella/EXECUTION.md).

---

## 1. Production scope, plan consolidation & paused work

The product is a lightweight Austral CPS/Cranelift compiler plus an `umbrella`
orchestrator that builds trusted Rust adapters, resolves their typed exports,
and runs mixed modules with explicit capabilities. Existing Rust libraries
should be reusable through generated, Rust-checked wrappers, including useful
owned types and methods. Users need a strict project manifest, repeatable
builds, fast lightweight adapter linking, clear diagnostics, installation,
supervised workers, and real filesystem/network capability enforcement.

“Full featured” means the supported surface below is complete and tested. It
does not mean arbitrary Rust layouts, macros, lifetimes, trait objects, unsafe
APIs or arbitrary native binaries become safe to import automatically.

| Area | Current implementation (measured 2026-11-27) | Production target |
|---|---|---|
| Build | `cargo.rs` selects exact package/target cdylib via `cargo metadata` + JSON stream; `--locked`/`--offline`/features/profile/target-dir CLI flags; JSON project schema (`schema: 1`, `deny_unknown_fields`). Gaps: raw `Command::output()` in driver, `build-finished` ignored, `RUSTFLAGS`/`.cargo/config.toml` logged not enforced | Explicit locked/offline build with bounded child supervision, `build-finished` + target/ELF checks, strict wrapper/config policy, and staged build/check/run |
| Interop | Versioned descriptors (`abi scalar 1`, `abi buffer 1/2`, `abi handle 1`, `abi text 1`, checked `Option`/`Either`/`Result`). Gaps: `TextAdapter` not wired in driver run path; generated adapter crates link full `austral_cranelift_bridge` (~585 MB, ~20 min) | Extracted lightweight `avm-adapter-rt` (<1 MB, seconds); full descriptor parity across compiler, `interface.rs`, and driver; generated Austral declarations |
| Calls | `compile_entry_i64` verifies zero-arg `Int64` entry; typed export pins (`trusted.signatures`) + descriptor pins (`descriptor_sha256`). Gap: `main.rs` prints `RESULT` without checking `JIT_PANIC`/`cranelift_last_error()`, and call denial exits `2` | Verified entry ABI, typed host status separated from all `i64` bit patterns, checked `JIT_PANIC`, multiple calls and declared argument shapes |
| Values | Scalars (`Unit`/`Bool`/`Int32`/`Nat32`/`Int64`/`Nat64`/`Float32`/`Float64`), bounded byte buffers, `Option[T]`/`Either[L,R]`/`Result[T,E]` via `__checked`/`__free`, `abi text 1` (`&str` → `String`) in CLI wrapper builder, linear `WidgetHandle`/`View`. Gaps: `TextAdapter` compiler emission, aggregate payloads, methods/`Drop`/generics demo | Checked scalars, bytes/text, `Option`/`Result`/`Either`, opaque owned values, methods, `Drop`, owned collections and selected concrete generics |
| Ownership | P1 group borrowing (erased pre-CPS); two-word generational `HandleRegistry<T>` + `ProviderValue` destructors; `Borrow` runtime claims + `View`; `__free` transfer tracking for checked unions. Gap: ordering of release vs dispose is runtime-checked | Linear local wrappers, allocator-specific destroy thunks, scoped borrows/invalidation, compile-negative + runtime ownership guarantees |
| Callbacks | `CallbackRegistry` (`Call`/`Instance` lifetime, `Caller`/`Any` thread, `Forbid`/`Allow` reentrancy, `ErrorPolicy`), `__with`, `__retain`/`__invoke_retained`/`__retire_callbacks`, `subscribe` protocol. Gap: compiler does not emit `callback`/`subscribe` from pragmas | Scoped synchronous callbacks with compiler pragma emission, lifetime/thread/reentrancy enforcement, and generation retirement |
| Filesystem | Descriptor-relative `RootDir` (`openat`/`O_NOFOLLOW`), per-root `FsRights`, cumulative write quota, `revoke`/`revoke_all`, `rs_cap_*` verbs, `cap_fs` compiled demo. Gaps: `openat2 RESOLVE_BENEATH` hardlink check, per-root quota, project-document binary run, `rust_kv` still stubbed | Host-created rooted handles with enforced operation rights, hardlink defense, per-root quotas, and real `rust_kv`/project e2e |
| Network | `netstd_mod.rs` (`NetPolicy`, CIDR `IpNet`, `rustls`+`ring` TLS, deadline, revocation, UDP/listen/raw refusal, project `capabilities.network`). Gaps: proxy-hop & cross-instance socket-token reuse fixtures, project-document dial run | Explicit bounded dial capability, actual resolved-target/redirect checks, TLS and timeout/error contract |
| Modules | Kahn topological source ordering + `--emit-cps-bundle` in `frontend.rs` / `Compiler.ml`, content-addressed `cache.rs`. Gaps: multi-provider refused, adapter doc still last module's, duplicate short names across packages | Explicit mixed module graph, qualified names across packages, per-provider ABI records, dependency/link validation |
| Runtime | Per-instance `InstanceContext` + `auth::enter_caller`, generation drain + callback retirement on adapter reload. Gaps: `safestos_load_auth_manifest` + static symbol registry + `LOADED` still process-global; no worker supervisor | Instance/generation ownership, one-generation-per-supervised-worker IPC, hard deadlines, deterministic failure status and cleanup |
| Isolation | `trusted-native` / `dev` profiles active; `untrusted-native` explicitly refused at preflight. Bridge has Linux `sandbox.rs` (user/net/ipc ns + seccomp + Landlock) and `docs/spectrum_plan.md` VMs not yet wired to Umbrella | Clearly separated `trusted-native` and verified `untrusted-native` profiles; untrusted mode disabled until U26 escape controls pass |
| Tooling | `scripts/test-umbrella` (13 cases in 3 lanes), 17 integration test files in `umbrella/tests/`, `--json` events. Gaps: 12 of 17 suites unwired, stale counts pinned, 6 lanes empty, no fuzz/sanitizers/mutations/packaging/runbooks | All lanes populated and synchronized, JSON diagnostics, traceable artifacts, fuzz/Miri/ASan/mutation controls, relocatable packaging & runbooks |

Initial native release platforms: Linux `aarch64` and `x86_64`, each tested
natively. The current loader does not implement Windows support just because
`cargo.rs` recognizes `.dll`. Cross-building an adapter may be supported
separately; cross-running that adapter in a host-native JIT must refuse.
macOS/Windows/32-bit require a separate platform matrix before they are
advertised.

### 1.1 Plan lineage and shared-seam ownership

| Plan | Relationship to this document | Shared seams & current state (2026-11-27) |
|---|---|---|
| [`PLAN_valen.md`](PLAN_valen.md) | **Merged here.** Historical P0–P3 record preserved with a superseded banner | P0 (`ATTRIBUTION.md`), P1 group borrowing (`018-group-borrow`), P2 `extern rust` Tier 1 (`019-extern-rust`), P3 (`umbrella/`). Architectural constraint, tiers, group borrowing, Valen/`../rust` analysis, and attribution absorbed |
| [`PLAN_distributed_security.md`](PLAN_distributed_security.md) | **Shared seams reconciled**; consensus/QuePaxa/crypto remain there | **D08 ↔ U09/U18** (verified: `InstanceContext` + `bind_instance_with_identity`), **D09 ↔ U08/U19–U22** (verified: `trust.rs` + `distributed_io`), **D17 ↔ U13/U14** (verified: `Distributed.{aui,aum}`, `umbrella/tests/distributed_{request,tls}.rs`), **D18 ↔ U07/U28** (verified: `--liquid-required`, `verification.rs`), **D24 ↔ U10/U25** (open: claim manifest + artifact identity) |
| [`PLAN_gpu_kernels.md`](PLAN_gpu_kernels.md) | **Shared seam**; WGSL/MLIR kernel compilation remains there | **G14** (`umbrella/modules/GpuService.aum`, manifest-gated `rs_gpu_*` service client) |
| [`docs/spectrum_plan.md`](docs/spectrum_plan.md) & `safestos/cranelift/src/sandbox.rs` | **Isolation backends for U26** | `sandbox.rs` (Linux user/net/ipc namespaces + `no_new_privs` + seccomp-bpf + Landlock) and Cloud-Hypervisor / Uhyve / Hermit unikernels |
| [`PLAN_avm_pkg.md`](PLAN_avm_pkg.md) | **Packaging / CAS seam for U25** | Signed `.avm` cell format, `cps2cell`, and content-addressed store (`safestos/pkg/`) |
| [`PLAN_umbrella_bend.md`](PLAN_umbrella_bend.md) (UB01 only) | **Non-Bend compiler-oracle pinning absorbed into U32**; all Bend work paused | Pinning an opt-in `RUSTC` oracle lane without Bend; `../rust` kept read-only |

### 1.2 Paused / excluded: Bend language integration

Per owner instruction (2026-11-27), **Bend language integration is paused**.
The following plans remain untouched and are **out of scope** for this plan:

- [`PLAN_umbrella_bend.md`](PLAN_umbrella_bend.md) (UB00, UB02–UB08 — Bend CLI,
  three-language build graph, `Process.run`/`.bendtt` bridges)
- [`PLAN_bend_austral_integration.md`](PLAN_bend_austral_integration.md)
- [`docs/LIQUID_BEND_PLAN.md`](docs/LIQUID_BEND_PLAN.md)
- [`docs/SAFESTOS_PARALLEL_PLAN.md`](docs/SAFESTOS_PARALLEL_PLAN.md)

No Umbrella milestone or release gate depends on Bend, BendRT, or `.bendtt`.

---

## 2. Architecture and design commitments

### 2.1 Core architectural constraint and interop tiers (from `PLAN_valen.md`)

> australVM's compiler stays JIT and lightweight. The Rust compiler is
> integrated **either** as an optional plugin over the existing seam **or** as
> a host orchestrator (`umbrella`) that includes australVM and Cargo/rustc as
> separate components — never by baking `rustc` into the OCaml compiler.

Interop progresses in explicit, layered tiers:

- **Tier 0 — Frozen kernel (`uk_*`/`uz_*`):** `safestos/cranelift/src/lib.rs`
  registers `au_*`, `uk_*`, and `uz_*`; every gated call passes
  `check_call_permission` in `safestos/cranelift/src/cps.rs`.
- **Tier 1 — `extern rust` (`rs_*`):** Austral declarations carry
  `Rust_Ownership` / `Handle_Ownership` pragmas checked in `lib/TypingPass.ml`
  and erased before CPS (`lib/Compiler_cps.ml`), emitting canonical adapter
  descriptors (`--emit-adapters`) while keeping `check_call_permission` live.
- **Tier 2 — `umbrella` orchestrator (`umbrella/`):** Owns Cargo subprocess
  invocation, generated wrapper compilation, explicit `RTLD_LOCAL` provider
  admission, CPS bundle compilation, capability injection, and execution.
- **Group borrowing (`PLAN_valen.md` §3 / P1, shipped):** Lexically scoped
  group regions (`lib/AbstractionPass.ml::group_region$*`,
  `test-programs/suites/018-group-borrow`) desugar into nested linear borrows
  before CPS lowering. Group borrowing **never** authorizes simultaneous Rust
  `&mut` references across FFI.

### 2.2 Separate build authority, native admission and invocation

Use a versioned strict JSON project document (`schema: 1`, `--project <json>`,
`umbrella/src/project.rs` with `#[serde(deny_unknown_fields)]` and section-qualified
unknown-field diagnostics) for Rust crates, profiles (`dev`, `trusted-native`,
`untrusted-native`), build/load permissions, typed trust pins, capability
roots/network policy, and call grants. (Earlier drafts provisionally named
`Umbrella.toml`; the implemented and tested contract is `--project <json>`,
while `module.toml` preserves the bridge's existing manifest grant and `[verify]`
vocabulary. Do not silently reinterpret legacy `fs` strings without explicit
rights as granting every operation in strict capability roots.)

Validation order:

```text
parse CLI / JSON project / policy preflight -> verify tools / target / lock & features
  -> Cargo metadata and locked build -> exact cdylib + sha256/exports check
  -> compile-only Austral CPS bundle -> verify typed exports & descriptor sha256
  -> build/load lightweight adapters (U10b) -> admit RTLD_LOCAL provider in worker
  -> inject host-created fs/net capabilities -> JIT -> checked invocation -> cleanup
```

Cargo build scripts and procedural macros are native execution. `dlopen` can
run library initializers (`.init_array`). A denied target call only establishes
that the call was blocked; it cannot undo prior native initialization. Tests
must track build, load, and call stages separately. If a deployment requires
zero untrusted execution, its build and load phases need U26 isolation, not
just a runtime call gate.

### 2.3 Versioned adapter ABI & lightweight runtime split (`avm-adapter-rt`)

Adapter contracts use canonical versioned text blocks (`schema 1`,
`umbrella/src/interface.rs`) emitted by `lib/Compiler_cps.ml` and validated
against `trusted.signatures` and `trusted.descriptor_sha256` before wrapper
compilation:

- **`abi scalar 1`:** Checked transport for `Unit`, `Bool`, `Int32`, `Nat32`,
  `Int64`, `Nat64`, `Float32` (`to_bits`/`from_bits`), `Float64` (`to_bits`/`from_bits`).
- **`abi buffer 1/2`:** `Nat8` `Address`/`Pointer` + 64-bit length pair;
  checked provenance/null-on-zero-length/non-negative/bounds before slice
  construction.
- **`abi handle 1`:** Two-word generational handles (`owner << 32 | slot`,
  `provider_gen << 32 | slot_gen`) with `__new`, `__use`, `__dispose`,
  `__borrow`, `__view`, `__borrow_mut`, `__write`, `__release`, `__with`,
  optional `callback` policy and `subscribe` retained-token protocol.
- **`abi text 1`:** Validated UTF-8 `&str` in and owned `String` `(ptr, len)` out.
- **Checked unions (`__checked` / `__free`):** `Option[T]`, `Either[L, R]`,
  `Result[T, E]` via out-pair `[tag, payload]` read from `union_layouts`, with
  static refusal sentinel recognition and `Compiler_cps.ml` release tracking.

**Critical architectural improvement (`U10b` / UR15):** Currently, generated
adapter crates compile with `rustc --extern austral_cranelift_bridge=...`
resolved via `current_exe().parent()/deps/*.rlib`. That pulls Cranelift, Cedar,
rustls, and `unfer_ffi` into every adapter cdylib (~585 MB per generation,
~20 min test link times). Extract the pure adapter runtime (`adapters.rs`,
`handles.rs`, `callbacks.rs`) into a zero-heavy-dependency crate
**`safestos/adapter-rt` (`avm-adapter-rt`)**, re-exported by
`austral_cranelift_bridge`, and pass its explicit staged/installed `.rlib` path
to `rustc`. Target size per adapter cdylib: **< 1 MB**, compile time **< 2 s**.

Generated descriptor, Rust wrapper and Austral declaration should come from one
schema with independent golden vectors and direct Rust call tests. Unsupported
Rust layouts (`String`/`Vec`/trait-object fat pointers/arbitrary enums/lifetimes/variadics)
remain rejected until explicitly modeled.

### 2.4 Ownership, capability identity and native lifetime

Rust-owned objects stay in a per-instance typed/generational registry. Austral
receives opaque `Linear` wrappers around host handles with private constructors.
Raw `Address` and bare integer tokens are not ownership proofs. Each object
records its provider generation and allocator-specific destructor. Never
`dlclose` native code or reset the JIT while a dependent call, handle, or
callback is live.

Borrowed views obey Rust's aliasing rules whenever a wrapper constructs `&` or
`&mut`: immutable borrows share; mutable borrows require exclusivity; any live
borrow refuses `__dispose` and generation drain. Reentrant callbacks never hold
the registry mutex across user code.

The trusted host alone opens ambient directory roots (`O_DIRECTORY | O_NOFOLLOW`
with per-component `openat` walking) and network policies (`NetPolicy` + CIDR +
`rustls` anchors). Classify every adapter explicitly as **capability-aware**,
**trusted-ambient**, or **isolated**, and refuse profiles whose requirements its
implementation cannot satisfy.

### 2.5 Runtime limits, worker supervision and failure semantics

For the first production runtime, **one loaded generation per supervised worker
process** is the target architecture (U23). This cleanly contains legacy global
bridge state (`safestos_load_auth_manifest`, static symbol registry, `LOADED`)
and allows hard deadlines via process-group termination (`kill(-pgid, SIGKILL)`).
A process is a lifecycle boundary, not automatically a security sandbox (U26).
In-process `trusted-native` remains an explicit lower-isolation profile.

Separate host execution status from all `i64` payload bit patterns (`-1`,
`i64::MIN`, `i64::MAX`). Catch unwinding panics inside generated `extern "C-unwind"`
wrappers (`STATUS_PANIC = 2`); check `cranelift_last_error()` alongside
`JIT_PANIC` (`i64::MIN`) in `execute_function` callers (UR17). Abort panic
profiles, traps, and segfaults are worker failures, never catchable exceptions.

Compile-only commands (`--emit-cps-only`) never initialize the bridge or execute
target code. Cache entries (`cache.rs`) store compilation/build bytes only,
never authorization verdicts.

### 2.6 Considered and not adopted (from `PLAN_valen.md` §6)

- **Patched `rustc` / `per-instance-mir` in the default stable path:** Not
  adopted; we do not maintain a fork of `valen-lang/rust` (`../rust` is a
  read-only reference only, §3.3 & `U32`).
- **LLVM AOT compilation replacing Cranelift JIT:** Two primary backends would
  double the verification and maintenance surface.
- **Generational GC or reference counting beside Austral linearity:** Adds a
  second memory model; host `HandleRegistry` handles explicit cross-FFI values
  only.
- **`.valen` source compatibility:** Design ideas are adapted cleanly; no source
  code is copied from `../Valen` or `../rust`.

---

## 3. Implementation audit (2026-11-27): `umbrella/` & `../rust`

### 3.1 Historical findings (`UR01`–`UR14`) reconciliation

| Finding | Original defect (`docs/UMBRELLA_PRODUCTION_REVIEW.md`) | Status on 2026-11-27 |
|---|---|---|
| UR01 | `cap-std` unwired; `rust_kv/run_demo.sh` printed fake PASS | **Partially closed.** `cap_fs` compiled demo + `openat` rights + quota + net policy work; `rust_kv` stays fail-closed exit 3 (`HAR02`) until U22 |
| UR02 | Accepted FFI types exceeded I64 word ABI | **Closed for scalars/buffers/unions.** `TypingPass.ml` + `U11`/`U12`/`U15` mediate via versioned wrappers; aggregates remain rejected |
| UR03 | `Rust_Ownership` erased with free raw pointers | **Partially closed.** `U13`/`U14` linear `WidgetHandle`/`View` + `HandleRegistry`; static/negative view fixtures remain (U14) |
| UR04 | Cargo picked first `.so`/`.dylib`/`.dll` | **Closed (`cargo.rs`), with minor stream gaps (UR19)** |
| UR05 | `RTLD_GLOBAL` + process-global symbol resolution | **Closed for driver/adapters (`RTLD_LOCAL` + `ProviderSet` + absolute `DT_NEEDED`);** legacy global fallback & `bind` remain (UR18/UR20) |
| UR06 | Cargo/dlopen ran before manifest checks | **Closed (`project.rs` + `policy::preflight` + `trust::verify_*`)** |
| UR07 | Entry names unchecked; empty fallback | **Closed (`compile_entry_i64`)** |
| UR08 | Multi-module last-writer-wins; advisory C fallback | **Partially closed (`--emit-cps-only` + `--emit-cps-bundle` + Kahn order);** multi-package/duplicate short names remain (U17) |
| UR09 | Predictable `/tmp/umbrella-<pid>.cps`; raw `Command::output()` | **Partially closed (`Staging` for front-end, `process.rs` for front-end);** `cargo_build`/`adapters.rs` still un-migrated (UR16) |
| UR10 | Global `safestos_load_auth_manifest` merging | **Partially closed (`InstanceContext` + `auth::enter_caller`);** legacy global install & single-process state remain (UR20) |
| UR11 | `JIT_PANIC` / last-error unchecked; no cdylib panic tests | **Partially closed (`__checked` + `STATUS_PANIC`);** `main.rs` `execute_function` still unchecked (UR17); no worker supervisor (U23) |
| UR12 | Prefix/lossy CLI parsing; wrong exit codes | **Mostly closed (`cli.rs`);** `main.rs:761` call-denial still exits `2` instead of `1` (UR18) |
| UR13 | No `--locked`/offline; checkout path assumptions | **Partially closed (`--locked`/`--offline`/cache);** relocatable packaging & `doctor` pending (U25) |
| UR14 | No umbrella test suite | **Partially closed (17 test files + `scripts/test-umbrella`);** 12 of 17 unwired and counts stale (UR21) |

### 3.2 New findings (`UR15`–`UR24`) from the 2026-11-27 code audit

- **UR15 (P0 — Adapter link bloat & non-relocatable discovery):**
  `umbrella/src/adapters.rs:38-66` (`bridge_rlib()`) globs
  `current_exe().parent()/deps/libaustral_cranelift_bridge*.rlib`, linking the
  entire Cranelift/Cedar/rustls/`unfer` dependency graph into every generated
  adapter cdylib (~585 MB per generation, noted at `adapters.rs:154`; ~20 min
  per `cargo test` run in `umbrella`). **Fix:** `U10b`, `U25`.
- **UR16 (P1 — Unwired child supervision, unbounded pipe drain & predictable adapter scratch):**
  `umbrella/src/main.rs:242,268` (`cargo_build`), `adapters.rs:129`
  (`build_at`), `trust.rs:108` (`nm`), and `cache.rs:328` (`toolchain_probe`)
  still use raw `Command::output()`. `process.rs:57-65,116-123` uses unbounded
  `read_to_end` and blocks in `out_rx.recv()` after the deadline loop if a
  grandchild kept the pipe open. `adapters.rs:230,252` writes to
  `temp_dir()/umbrella-adapters-<pid>-<gen>` via `create_dir_all` + `atexit`
  instead of `staging::Staging`. **Fix:** `U06b`.
- **UR17 (P0 — `execute_function` / `JIT_PANIC` unchecked in driver):**
  `umbrella/src/main.rs:1073-1102` prints `RESULT {result}` (and JSON `"done"`)
  without checking `cranelift_last_error()` or `JIT_PANIC` (`i64::MIN`), so a
  panicking helper or native fault becomes `RESULT -9223372036854775808`
  instead of an error. That block is also duplicated between lines 1073 and
  1091. **Fix:** `U15`.
- **UR18 (P1 — Call-denial exit code `2`, missing `TextAdapter` driver wiring & residual `RTLD_GLOBAL`):**
  `main.rs:761` exits with code `2` (reserved for usage errors) on runtime
  policy call denial instead of `1`. `main.rs:337-345,927-932` parses scalars,
  buffers, and handles, omitting `TextAdapter` (`abi text 1`) on the module run
  path (`read_descriptor_document` includes it only for `--descriptor-document`).
  `main.rs:744-753` (no-project path) and `adapters.rs:431-432` (`adapters::bind`)
  still call `policy::dlopen` (`RTLD_GLOBAL`). **Fix:** `U03`, `U06b`, `U08`, `U21`.
- **UR19 (P1 — Cargo stream & target validation gaps):**
  `cargo.rs:271-344` ignores `build-finished.success`, does not verify
  `target.kind` contains `"cdylib"` on matched stream events, matches
  `.dll`/`.dylib` without host ELF/arch validation (`BLD05`), and logs rather
  than enforces `RUSTFLAGS`/`.cargo/config.toml` policy in strict mode (`PKG07`).
  **Fix:** `U05`/`U06b`, `U25`, `U27`.
- **UR20 (P1 — Residual process-global state & in-process execution):**
  `main.rs:378` still calls process-global `safestos_load_auth_manifest`;
  `provider.rs:100` and `adapters.rs:307` mutate the global static symbol
  registry; `adapters.rs:328` keeps global `LOADED`; no supervised worker
  process exists yet. **Fix:** `U09`, `U23`.
- **UR21 (P1 — Test harness drift):**
  `umbrella/tests/` contains 17 integration test files (`buffer_roundtrip`,
  `cache`, `callbacks_retained`, `cargo_select`, `checked_adapters`, `cli`,
  `distributed_request`, `distributed_tls`, `either_adapters`,
  `either_allocations`, `entry`, `handles`, `interface`, `linking`, `policy`,
  `scalar_roundtrip`, `text_adapters`), yet `scripts/test-umbrella` wires only
  5 of them, pins stale test counts (`SUITE_UMBRELLA_CLI` at 6 vs 9 in U21;
  `SUITE_UMBRELLA_ENTRY` at 4 vs 12 in U18), and leaves 6 of 9 lanes empty.
  **Fix:** `U00b`, `U28`.
- **UR22 (P1 — Sibling `../unfer` build coupling):**
  `umbrella/Cargo.toml` pulls default bridge features (`unfer-kernel` →
  `../../../unfer/unfer_ffi`) and `safestos/cranelift/Cargo.toml` unconditionally
  depends on `unfer_protocol`. When `../unfer` is dirty/mid-merge, default
  umbrella builds fail, forcing ad-hoc reduced-feature commands
  (`cedar,tier1-rs`). Meanwhile `safestos/cranelift/src/capstd_mod.rs` now
  implements Linux `openat`/`O_NOFOLLOW` directly without requiring the
  offline-missing `cap-std` crate (though `tests/capstd_tests.rs` still sits
  behind `capstd`). **Fix:** `U00b`, `U19`, `U25`.
- **UR23 (P2 — JIT memory-builtin lowering gap):**
  `examples/modules/cap_fs/CapFs.aum` is restricted to a byte-for-byte copy
  because `positiveOffset`/`load`/`store` lower to unresolved `__embed$2` under
  the JIT, blocking buffer transformation in U22 and replacement of
  `examples/modules/rust_kv/`. **Fix:** `U22`.
- **UR24 (P2 — Audit of `../rust` fork `c6f757802519a0fc2619dd79e0be879601899b06`):**
  Inspected directly at `/home/leo/Projects/rust` (branch `per-instance-mir`,
  unbuilt — no `build/` directory, 7 files, +95/-3 lines over Jan 2026 nightly
  `d940e56841dd`):
  1. **`per_instance_mir` query** (`rustc_middle/src/query/mod.rs`,
     `rustc_mir_transform/src/lib.rs`, `rustc_monomorphize/src/collector.rs`):
     runs *post-borrowck* at mono collection; injected MIR is **not**
     borrow-checked, and Valen's `collect_and_partition_mono_items.rs` works
     around warm-build query skipping.
  2. **`fill_extra_modules` + raw LLVM hooks** (`rustc_codegen_llvm/src/lib.rs`,
     `rustc_codegen_ssa/src/{base.rs,back/write.rs,traits/backend.rs}`): marked
     `// DO NOT SUBMIT` in four places; strictly LLVM AOT-only.
  3. **Cranelift backend independence:** `compiler/rustc_codegen_cranelift/src/base.rs:47`
     calls `tcx.instance_mir(instance.def)` directly (never consults
     `per_instance_mir`), and `rustc_codegen_cranelift` does **not** implement
     `ExtraBackendMethods` (only `rustc_codegen_llvm` and `rustc_codegen_gcc`
     do, and only `rustc_codegen_llvm` wires `fill_extra_modules`). Thus both
     Valen hooks are strictly LLVM-AOT mechanisms, not Cranelift JIT seams.
  4. **Decision:** Keep `../rust` **strictly as a read-only engineering
     reference** (no maintained fork, no patching or building `../rust`, no
     source copied; see §3.3). A read-only `rustc_driver` oracle (`U32a`)
     querying stock `rustc_private` (`fn_abi_of_instance`, `layout_of`) needs
     neither hook and preserves full borrow checking; `U32b` records the
     deferred AOT-only evaluation bounds.

### 3.3 `../rust` engineering reference map (read-only)

`../rust` carries full implementations of `rustc_codegen_cranelift`,
`rustc_abi`, `rustc_middle`, and `rustc_driver_impl` that serve as concrete
blueprints for Umbrella without building or forking `rustc`:

| `../rust` reference source | Concrete mechanism | Umbrella / Bridge seam |
|---|---|---|
| `compiler/rustc_codegen_cranelift/src/driver/jit.rs` (`create_jit_module`, `dep_symbol_lookup_fn`, lines 19–33, 95–107, 169–215) | Postorder dylib loading + per-`JITBuilder` `symbol_lookup_fn` closure over owned `Library` handles instead of `RTLD_GLOBAL`/global tables; explicit entry `Signature` check before `transmute` | **U08 / U09 / U18** (instance-scoped JIT symbol resolution without global state) |
| `compiler/rustc_codegen_cranelift/src/abi/pass_mode.rs` (`ArgAbiExt`, `reg_to_abi_param`, `cast_target_to_abi_params`), `abi/mod.rs` (`clif_sig_from_fn_abi`, `conv_to_call_conv`, `import_function`), `compiler/rustc_abi/src/callconv.rs` (`HomogeneousAggregate`) | Exact mapping from `PassMode` (`Ignore`/`Direct`/`Pair`/`Cast`/`Indirect`) and `RegKind` (`Integer`/`Float`/`Vector`) to Cranelift `AbiParam` (`.uext()`/`.sext()`, `StructArgument`, `StructReturn`) and `CallConv`, plus `ModuleError::IncompatibleDeclaration`/`IncompatibleSignature` diagnostics | **U11 / U12 / U21 / U27 / U32a** (exact boundary between word/pair-transportable types and wrapper-mediated aggregates on `x86_64` vs `aarch64`) |
| `compiler/rustc_codegen_cranelift/src/unwind_module.rs` & `debuginfo/unwind.rs` (`UnwindModule`, `UnwindContext::register_jit`, `.eh_frame` CIE/FDE + `DW.ref.rust_eh_personality` for `x86_64`/`aarch64`) | Unwinding across Cranelift JIT frames fails unless `.eh_frame` FDEs are emitted and registered after `finalize_definitions()` | **U15 / U23** (validates catching panics inside `extern "C-unwind"` wrappers + containing abort/foreign-unwinder panics as U23 worker crashes, plus blueprint if JIT unwinding is ever added) |
| `compiler/rustc_driver_impl/src/lib.rs` (`Callbacks::{config, after_expansion, after_analysis}` → `Compilation::Stop`, lines 252–383) & `compiler/rustc_middle/src/ty/layout.rs` (`layout_of`, `fn_abi_of_instance`, lines 732, 1370) | Out-of-process `rustc_driver` lifecycle stopping after full typecheck/borrowck (`tcx.ensure_ok().analysis(())`) and before codegen/linking | **U32a** (read-only descriptor oracle emitting `umbrella/src/interface.rs` schemas with zero MIR replacement or fork dependency) |
| `compiler/rustc_codegen_cranelift/src/base.rs:47` & `../rust` commit `c6f75780251` (`rustc_monomorphize/src/collector.rs`, `rustc_codegen_llvm/src/lib.rs`) | Proves `cg_clif` calls `tcx.instance_mir` directly and ignores `per_instance_mir` and `ExtraBackendMethods` | **U32b** (bounds optional AOT-only study without modifying or building `../rust`) |

---

## 4. Execution discipline, test lanes and build profiles

### 4.1 Execution discipline

Read applicable `AGENTS.md`, this plan's current task, its review finding and
its [test matrix](docs/umbrella/TEST_MATRIX.md) rows. Preserve
[`PLAN_distributed_security.md`](PLAN_distributed_security.md), all paused Bend
plans (§1.2), and all unrelated dirty files. Read `../Valen` and `../rust`
strictly as read-only references (see §3.3); never edit, patch, or require
building either reference repository.

One patch should normally touch at most three production files plus relevant
tests. Split a task into numbered substeps if necessary. First reproduce the
named symptom with a small fixture, then fix it, then run the targeted test plus
its relevant existing suite. Use child-process isolation for malformed native
calls, crashes and untrusted fixtures; never explore invalid pointers in the
agent's own process.

After each patch record: task/case IDs, source revisions and dirty inputs,
toolchain/features/target, exact command, exit/signal/timeout, observed oracle,
evidence path and remaining gaps in [the execution record](docs/umbrella/EXECUTION.md)
and §8. Do not regenerate expected outputs simply to make a failure disappear.
Every test needs a real success oracle and a discriminating failure oracle; no
unconditional `PASS`, zero-case pass, blanket `#[ignore]`, skip-as-pass, or
arbitrary `sleep` barriers. Existing held-out MM0 statement approval
(`scripts/mm0-comparator`) remains the owner's separate workflow.

### 4.2 Test lanes and commands

`scripts/test-umbrella` runs `umbrella`'s lane harness (`--lane`, `--case`,
`--all`, `--strict`, `--json`, `--list`, `--seed`). Currently populated lanes:
`cli` (`HAR01`, `HAR03`), `build` (`BLD08` + 12 `SUITE_*` entries), and
`capability` (`HAR02`). Remaining lanes (`abi`, `e2e`, `robustness`, `package`,
`isolated`, `oracle`) must be wired by **`U00b`** and **`U28`**.

| Lane | Contents | Dependencies / execution policy |
|---|---|---|
| `cli` | Real binary parsing and mock-tool stage boundaries | No compiler/network; every case must execute |
| `build` | Cargo message selection, tool supervision, staging and unit/wrapper suites | Local fixture crates only; cache tools once |
| `abi` | Descriptor, values, handles, ownership, real cdylib calls | Built fixture providers; native crash cases in children |
| `capability` | Actual rooted fs (`openat`/`O_NOFOLLOW`) and loopback network access | Unique roots/ports/certs; `capstd` where tested |
| `e2e` | Real compiler → bundle → adapter → JIT → capability IO | Fresh bridge/compiler identity; no mocks at the core seam |
| `robustness` | Worker crashes/cancellation, fuzz corpus and mutation controls | Fixed seeds and bounded runtime; separate sanitizer sublane |
| `package` | Relocation, target/version checks, locked/rebuilt artifacts | Native CI per supported target |
| `isolated` | Build and native escape controls (`sandbox.rs` / VM) | Tested isolation backend; required only for `untrusted-native` |
| `oracle` | Optional read-only `rustc_driver` / ABI oracle (`U32`) | Exact pinned toolchain; independent feature gate |

Command templates, from the repository root:

```bash
# All dune commands and make targets needing dune run in nix develop.
nix develop "git+file://$PWD#ocaml" -c make bridge
nix develop "git+file://$PWD#ocaml" -c make all
nix develop "git+file://$PWD#ocaml" -c make test

# Umbrella lane harness (non-strict allows gaps; --strict fails on required gaps).
nix develop "git+file://$PWD#ocaml" -c python3 scripts/test-umbrella --lane cli --strict --json
nix develop "git+file://$PWD#ocaml" -c python3 scripts/test-umbrella --lane capability --strict --json

# Umbrella integration tests (default features, or reduced features when ../unfer is mid-merge).
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path umbrella/Cargo.toml --lib
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --no-default-features --features cedar,tier1-rs --manifest-path safestos/cranelift/Cargo.toml --lib

# Existing focused bridge & compiler suites.
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs --test tier1_extern_rust
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs,capstd --test capstd_tests
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs --test static_seam
nix develop "git+file://$PWD#ocaml" -c bash -c 'export LD_LIBRARY_PATH="$PWD/safestos/cranelift/target/release"; python3 test-programs/runner.py 018-group-borrow && python3 test-programs/runner.py 019-extern-rust && python3 test-programs/runner.py 025-linear-handles'

python3 scripts/check-links
git diff --check
```

---

## 5. Release milestones and task dependencies

| Milestone | Required result | Tasks |
|---|---|---|
| Foundation (Shipped) | Attribution, group borrowing, `extern rust` Tier 1, prototype `umbrella/` | `PLAN_valen.md` P0–P3 |
| R0 — honest baseline | Real evidence, synchronized harness, explicit unsupported cases, no false demo pass | `U00`, `U00b`, `U01`–`U04` |
| R1 — safe orchestration & lightweight RT | Exact artifacts, supervised children, compile-only output, scoped auth, `avm-adapter-rt` (<1 MB) | `U05`, `U06`, `U06b`, `U07`–`U10`, `U10b` |
| R2 — useful library interop | Scalars/bytes/text, owned values/borrows, checked unions/`JIT_PANIC`, scoped callbacks, mixed graph & cache | `U11`–`U18` |
| R3 — real capability modules & workers | Enforced fs/net rights, useful Rust adapters, `__embed$2` fix, real `rust_kv` demo, supervised workers | `U19`–`U23` |
| R4 — production trusted-native profile | Fuzz/sanitizer/mutation controls, locked packaging (`doctor` + `avm-pkg`), two native platforms, CI, runbooks, budgets/gate | `U24`–`U25`, `U27`–`U30` |
| R4-isolated | Separately validated untrusted build/native execution (`sandbox.rs` + `docs/spectrum_plan.md`) | `U26` plus R4 |
| Extensions | Optional async host handles (`U31`) and pinned `rustc` descriptor/ABI oracle (`U32a`/`U32b`) | `U31`, `U32a`, `U32b` |

Shared seams with [`PLAN_distributed_security.md`](PLAN_distributed_security.md):
D08 ↔ U09/U18 [verified], D09 ↔ U08/U19–U22 [verified], D17 ↔ U13/U14
[verified], D18 ↔ U07/U28 [verified], D24 ↔ U10/U25 [open]. Implement each
shared seam once and link the same evidence from both ledgers.

---

## 6. Task cards (`U00`–`U32` + remediation cards)

Matrix IDs reference [the test matrix](docs/umbrella/TEST_MATRIX.md). Paths
marked **new** do not exist yet.

### U00 — Build an honest baseline and cheap test harness
**Status:** `implemented with gaps`. **Depends:** none. **Finding:** UR14.
**Cases:** HAR01/HAR03, BLD08. **Files:** `umbrella/tests/support/`,
`scripts/test-umbrella`, `docs/umbrella/EXECUTION.md`.

- **Done:** `scripts/test-umbrella` implements `--lane`/`--case`/`--all`/`--strict`/`--json`/`--list`/`--seed`, records setup/identity/build/test times separately, fails on unknown/empty selectors, wires `HAR01`–`HAR03`, `BLD08`, and 12 `SUITE_*` entries.
- **Remaining:** Wire the remaining 12 `umbrella/tests/*.rs` suites and remaining 6 lanes via **U00b**; align `BLD08` version-mismatch detection with U25.
- **Accept:** Unknown/empty selector exits 2; required gap fails `--strict`; failing child cannot report pass.

### U00b — Reconcile test harness inventory & decouple standalone umbrella features
**Status:** `pending`. **Depends:** U00. **Findings:** UR21, UR22.
**Cases:** HAR01/HAR03, AUT06, PKG06. **Files:** `scripts/test-umbrella`,
`umbrella/Cargo.toml`, `docs/umbrella/EXECUTION.md`.

1. Wire all 17 `umbrella/tests/*.rs` suites into `scripts/test-umbrella` across
   their proper lanes (`cli`, `build`, `abi`, `capability`, `e2e`) and update the
   stale pinned test counts (`SUITE_UMBRELLA_CLI`, `SUITE_UMBRELLA_ENTRY`, etc.)
   against measured `cargo test` runs.
2. Add an explicit feature flag in `umbrella/Cargo.toml` (e.g., `default = ["full-bridge"]`,
   or an explicit standalone profile without `unfer-kernel`) so umbrella can
   compile and test standalone when sibling `../unfer` is dirty/absent, while
   keeping full-feature production verification distinct and never silently
   counted as `unfer-kernel` coverage.
3. Update `REQUIRED_CASES` so no existing suite can silently vanish.

**Accept:** `scripts/test-umbrella --list` accounts for all 17 `umbrella/tests/`
suites; `--lane` runs execute non-empty verified suites; reduced vs full bridge
features are recorded distinctly in JSON identity.

### U01 — Remove false capability demo success
**Status:** `implemented with gaps`. **Depends:** U00. **Finding:** UR01.
**Cases:** HAR02. **Files:** `examples/modules/rust_kv/run_demo.sh`,
`examples/modules/rust_kv/fixtures/printed_success.txt`, `scripts/test-umbrella`.

- **Done:** `run_demo.sh` exits 3 with `UNSUPPORTED: capability execution is not implemented`, marks existing bridge tests as unexecuted references, and `HAR02` verifies the real demo, `printed_success.txt` rejection, and an `echo PASS` mutation guard.
- **Remaining:** U22 replaces this gap with the real compiled Austral + Rust adapter + capability IO pipeline.
- **Accept:** Missing implementation cannot produce `ALL TESTS PASSED`.

### U02 — Define strict profiles and perform policy preflight
**Status:** `implemented`. **Depends:** U00. **Findings:** UR01/UR06.
**Cases:** AUT01, BLD06, RUN06. **Files:** `umbrella/src/project.rs`,
`umbrella/src/policy.rs`, `umbrella/src/main.rs`, `umbrella/tests/policy.rs`.

- **Done:** Strict `schema: 1` JSON document with `deny_unknown_fields` and section-qualified diagnostics; separate `build`/`load`/`call` preflight decisions; `untrusted-native` refused before any stage; `stageprobe` `.init_array`/build-script marker test.
- **Remaining:** Wire project `grants` directly into per-instance `ManifestAuthEngine` without needing separate `--manifest` CLI flag; isolate blocking initializers in U23 worker.
- **Accept:** No build/load authorization reaches neither marker; denied call never claims build/initializer containment.

### U03 — Fix CLI grammar, diagnostics and exit contracts
**Status:** `implemented with gaps`. **Depends:** U00. **Finding:** UR12, UR18.
**Cases:** CLI01–CLI06. **Files:** `umbrella/src/cli.rs`,
`umbrella/src/main.rs`, `umbrella/tests/cli.rs`.

- **Done:** Exact `--name`/`--name=value` parsing over `OsString`, `--` delimiter, non-UTF-8 path preservation, exit 2 on usage error before spawning tools, `--json` NDJSON events with diagnostics on stderr, legacy demo compatibility.
- **Remaining:** Fix `umbrella/src/main.rs:761` so runtime call-grant refusal exits `1` (operational failure) instead of `2` (usage error) (UR18); complete runtime-refusal JSON events (U15).
- **Accept:** All malformed invocations exit 2 without spawning Cargo/frontend; operational/policy denials exit 1; JSON stdout contains only protocol events.

### U04 — Close unchecked ABI and entry invocation immediately
**Status:** `passed within stated profile`. **Depends:** U00/U02.
**Findings:** UR02/UR07. **Cases:** ABI01/ABI02, BLD05.
**Files:** `lib/TypingPass.ml`, `lib/TypeErrors.ml`, `safestos/cranelift/src/{cps,lib}.rs`,
`umbrella/src/main.rs`, `test-programs/suites/019-extern-rust/`.

- **Done:** `compile_entry_i64` refuses empty names, missing exports, non-zero arity, non-`Int64` returns, and non-64-bit hosts; `TypingPass.ml` refuses unsupported raw `rs_*` transport types (`008`/`009` fixtures); boundary i64 values tested.
- **Remaining:** Host ELF/architecture preflight on prebuilt `.so`/`.dylib`/`.dll` artifacts before `dlopen` (covered in `U06b`/`U27`).
- **Accept:** Mismatched signatures or foreign architectures refuse before native invocation.

### U05 — Select exact Cargo artifacts and make builds explicit
**Status:** `implemented with gaps`. **Depends:** U02/U03.
**Findings:** UR04/UR13/UR19. **Cases:** BLD01–BLD05, BLD09, PKG07.
**Files:** `umbrella/src/cargo.rs`, `umbrella/src/main.rs`,
`umbrella/tests/cargo_select.rs`, `umbrella/tests/fixtures/cargo-workspace/`.

- **Done:** `wanted_from_metadata_for` + `select_shared_artifact` match exact `package_id` and `target.name`; explicit `--cargo-package`/`--target-dir`/`--profile`/`--features`/`--locked`/`--offline` argv; refuses `RUSTC_WRAPPER`/`RUSTC_WORKSPACE_WRAPPER`; logs `RUSTFLAGS`/`.cargo/config*`.
- **Remaining (see `U06b`):** Verify `build-finished.success == true` and `target.kind` contains `"cdylib"` on stream artifacts; cross-check requested vs built features; enforce strict policy on `RUSTFLAGS`/`.cargo/config*` (`PKG07`).
- **Accept:** Dependency `.so`/proc-macros never win selection; failed/incomplete builds refuse; locked builds leave `Cargo.lock` untouched.

### U06 — Bound child processes and use private staging
**Status:** `implemented with gaps`. **Depends:** U03. **Findings:** UR09/UR16.
**Cases:** BLD07, CPS02, RUN03/RUN08, PKG07.
**Files:** `umbrella/src/process.rs`, `umbrella/src/staging.rs`.

- **Done:** `process::run` spawns in `process_group(0)`, drains stdout/stderr on threads, kills `-pgid` at deadline, distinguishes exit codes vs signals; `Staging::create` creates private non-reused directories cleaned on drop.
- **Remaining:** Closed by **U06b** below.
- **Accept:** No unbounded buffering, pipe hang after leader exit, predictable `/tmp` adapter dirs, or orphaned grandchildren.

### U06b — Harden child supervision, pipe bounds, adapter staging & Cargo stream checks
**Status:** `pending`. **Depends:** U05, U06. **Findings:** UR16, UR18, UR19.
**Cases:** BLD03/BLD05/BLD07, CPS02, RUN03, PKG07.
**Files:** `umbrella/src/process.rs`, `umbrella/src/adapters.rs`,
`umbrella/src/cargo.rs`, `umbrella/src/main.rs`, `umbrella/src/trust.rs`,
`umbrella/src/cache.rs`.

1. Cap captured stdout/stderr per stream in `process::run` and enforce the
   deadline across pipe drain (`recv_timeout`) so a surviving grandchild holding
   a pipe FD cannot wedge the parent.
2. Route `cargo_build` (`main.rs`), `adapters::build_at`, `trust::exports` (`nm`),
   and `cache::toolchain_probe` through `process::run` with explicit deadlines
   and controlled environment allowlists instead of raw `Command::output()`.
3. Replace `adapters.rs` `temp_dir().join("umbrella-adapters-...")` + `atexit`
   with `staging::Staging`.
4. In `cargo.rs`, require `build-finished` (`success: true`) and `cdylib` target
   kind on matched stream artifacts; reject foreign ELF/non-Linux-`.so`
   artifacts before `dlopen` (`BLD05`); fix `main.rs:761` call denial exit `2` → `1`.

**Accept:** Noisy/hanging grandchild test completes within deadline; `cargo build`
with `build-finished: false` or non-host ELF refuses before mapping; adapter
scratch directories use private `Staging`.

### U07 — Add a fail-closed compile-only CPS artifact contract
**Status:** `implemented with gaps`. **Depends:** U04/U06. **Finding:** UR08.
**Cases:** CPS01/CPS02/CPS05–CPS07. **Files:** `umbrella/src/frontend.rs`,
`umbrella/src/staging.rs`, `lib/Compiler.ml`, `lib/Compiler_cps.ml`,
`lib/CliParser.ml`, `lib/CliEngine.ml`.

- **Done:** `--emit-cps-only` lowers without initializing bridge or executing native code; `frontend::compile` removes `CPS_JIT_ADVISORY`, drops `--allow-all`, threads `--liquid-required` (`D18`), verifies staged paths, byte counts, function counts, and deduplicated `Export` wire signatures.
- **Remaining:** Merge or validate adapter documents across all compiled modules instead of taking only the last module's `adapters` document (shared with U17).
- **Accept:** Bad JIT lowering never becomes runnable; compile-only fixture creates zero runtime/initializer markers.

### U08 — Bind providers explicitly and own loaded-library lifetimes
**Status:** `implemented with gaps`. **Depends:** U04/U05. **Findings:** UR05/UR18/UR20.
**Cases:** LNK01–LNK04, AUT05. **Files:** `umbrella/src/provider.rs`,
`umbrella/src/policy.rs`, `umbrella/src/main.rs`, `umbrella/tests/linking.rs`.
**Reference (`../rust` §3.3):** `rustc_codegen_cranelift/src/driver/jit.rs`
(`dep_symbol_lookup_fn` / per-`JITBuilder` `symbol_lookup_fn` closure +
postorder dylib loading).

- **Done:** `ProviderSet::load` maps `RTLD_LOCAL`, rejects duplicate paths/exports and reserved prefixes (`au_`/`__`/`uk_`/`uz_`), binds exact `dlsym(handle)` addresses into static registry before JIT init, retains handles without `dlclose`.
- **Remaining:** Support multiple explicit providers per project with per-provider ABI records; remove `RTLD_GLOBAL` fallback in `main.rs:744-753` and `adapters::bind` (`adapters.rs:431-432`); replace global symbol state with an instance-scoped resolver closure (following `driver/jit.rs`) or isolate per worker (U23).
- **Accept:** Preloaded conflicting symbols cannot win; multiple providers defining the same symbol refuse deterministically.

### U09 — Scope authorization and injected capabilities per instance
**Status:** `implemented with gaps`. **Depends:** U02/U08. **Findings:** UR10/UR20.
**Cases:** AUT02–AUT04, LNK03. **Files:** `safestos/cranelift/src/{auth,module,cps}.rs`,
`umbrella/src/{project,main}.rs`.

- **Done:** `InstanceContext` + `auth::bind_instance_with_identity` + `auth::enter_caller` + `replace_policy` isolate module A/B snapshots and prevent merged grant escalation on narrowed reload (`D08`).
- **Remaining:** Retire legacy global `safestos_load_auth_manifest` call from `main.rs:378`; add concurrent A/B worker tests (U23); unify handle owner tags with instance IDs.
- **Accept:** Module A never inherits B's grants; narrowed rebind refuses removed grants with `UK-4001`.

### U10 — Generate a versioned checked adapter contract
**Status:** `implemented with gaps`. **Depends:** U04/U05/U07/U08.
**Findings:** UR02/UR05/UR15. **Cases:** ABI03/ABI06, API03.
**Files:** `umbrella/src/{interface,adapters,trust,project,main}.rs`,
`umbrella/tests/{interface,entry,linking}.rs`.

- **Done:** Canonical descriptor text + parser + `rustc`-verified wrapper generation; adapters link admitted provider by absolute `DT_NEEDED`; `trust::verify_signatures` and `trust::verify_descriptor` check pins before `ProviderSet::load`; `--emit-trust-record` outputs deterministic pins; unsupported lifetimes/variadics/layouts refuse.
- **Remaining:** Canonicalize/strip comments before `descriptor_sha256`; support generated Austral `.aui`/`.aum` bindings from schema; close adapter link bloat via **U10b**.
- **Accept:** Signature/descriptor/provider mutations refuse before `dlopen` or native call.

### U10b — Extract lightweight `avm-adapter-rt` runtime crate
**Status:** `pending`. **Depends:** U10, U13, U16. **Finding:** UR15 (P0).
**Cases:** ABI03–ABI08, OWN03–OWN09, PKG01/PKG05.
**Files:** **new** `safestos/adapter-rt/Cargo.toml`, `safestos/adapter-rt/src/{lib,adapters,handles,callbacks}.rs`,
`safestos/cranelift/Cargo.toml`, `safestos/cranelift/src/lib.rs`,
`umbrella/src/adapters.rs`, `umbrella/Cargo.toml`.

1. Move pure adapter runtime modules (`adapters.rs`, `handles.rs`, `callbacks.rs`,
   with no `cranelift`, `cedar-policy`, `rustls`, or `unfer_*` dependencies)
   into `safestos/adapter-rt` (`crate-type = ["rlib"]`) and re-export them from
   `austral_cranelift_bridge` so all existing bridge APIs/tests keep working
   unchanged.
2. Update `umbrella/src/adapters.rs` (`build_at` / `build_cached`) to link
   generated adapter cdylibs against `libavm_adapter_rt.rlib` using an explicit
   staged/configured path (env/manifest/install-prefix, never guessing via
   `current_exe()/deps` globbing).
3. Measure cdylib binary size and compile latency before/after.

**Accept:** Generated adapter cdylib shrinks from ~585 MB to < 1 MB and
compiles in < 2 s; all `interface`, `handles`, `callbacks_retained`,
`scalar_roundtrip`, `buffer_roundtrip`, `checked_adapters`, `either_adapters`,
`either_allocations`, and `text_adapters` suites pass with zero behavior change.

### U11 — Complete scalar conversions and target calling conventions
**Status:** `passed within stated profile`. **Depends:** U10.
**Cases:** ABI04/ABI05. **Files:** `safestos/cranelift/src/adapters.rs`,
`umbrella/src/interface.rs`, `lib/Compiler_cps.ml`, `umbrella/tests/scalar_roundtrip.rs`,
`umbrella/modules/AdapterLive.aum`.
**Reference (`../rust` §3.3):** `rustc_codegen_cranelift/src/abi/{pass_mode.rs,mod.rs}`
(`ArgAbiExt`, `clif_sig_from_fn_abi`, `conv_to_call_conv`) and
`rustc_abi/src/callconv.rs` (`HomogeneousAggregate`) for exact `PassMode` →
Cranelift `AbiParam` / `CallConv` rules.

- **Done:** All 8 scalar types (`Unit`, `Bool`, `I32`, `U32`, `I64`, `U64`, `F32`, `F64`) encoded/decoded with `-0.0`/NaN/infinity bit-preservation, compiler emission + golden descriptors, and live JIT execution (`AdapterLive.aum` + `scalar_roundtrip.rs`).
- **Remaining:** Repeat natively on Linux `aarch64` and `x86_64` in U27 and validate `PassMode` boundaries against `pass_mode.rs` rules.
- **Accept:** Accepted scalar signatures match direct Rust calls bit-for-bit; unsupported signatures refuse.

### U12 — Implement bounded bytes, text and caller-owned output protocols
**Status:** `implemented with gaps`. **Depends:** U10/U11. **Finding:** UR03.
**Cases:** OWN01/OWN02. **Files:** `umbrella/src/interface.rs`,
`lib/{TypingPass,Compiler_cps}.ml`, `umbrella/tests/{buffer_roundtrip,text_adapters}.rs`,
`umbrella/modules/BufferLive.aum`.

- **Done:** `abi buffer 1/2` (`Address`/`Pointer` + length, canary/embedded-NUL/zero-length/SIGABRT negative tests, `BufferLive.aum` live JIT); `abi text 1` (`TextAdapter` `&str` → `String` `(ptr, len)` + `text_adapters.rs`).
- **Remaining:** Wire `TextAdapter` emission into OCaml compiler (`TypingPass.ml`/`Compiler_cps.ml`) and `umbrella/src/main.rs` (UR18); add caller-supplied capacity checks beyond declared length and owned result handles/UTF-8 error reporting; ensure text adapter panics map to structured status rather than aborting.
- **Accept:** Valid binary/UTF-8 roundtrips match direct Rust; malformed lengths/invalid UTF-8 refuse before dereference.

### U13 — Add linear owned Rust values and exact destruction
**Status:** `implemented with gaps`. **Depends:** U10/U12. **Finding:** UR03.
**Cases:** OWN03–OWN05/OWN08/OWN09, LNK05.
**Files:** `safestos/cranelift/src/handles.rs`, `umbrella/src/{interface,adapters}.rs`,
`umbrella/modules/WidgetHandle.{aui,aum}`, `test-programs/suites/025-linear-handles`,
`umbrella/tests/handles.rs`.

- **Done:** `HandleRegistry<T>`, two-word generational handles, `ProviderValue` RAII drop guard, `__new`/`__use`/`__dispose`, generation bump on `adapters::build_next`, linear Austral `Widget` wrapper with negative linearity fixtures (`025-linear-handles`).
- **Remaining:** Allow modules to pass callbacks to `__with` (or equivalent scoped accessor); handle generation exhaustion/overflow test (`OWN05`); decouple handle runtime via **U10b**.
- **Accept:** Compile-negative linearity fixtures fail; runtime stale/foreign handles refuse; drop count equals successful allocations.

### U14 — Enforce borrowed-view validity and Rust aliasing
**Status:** `implemented with gaps`. **Depends:** U13. **Finding:** UR03.
**Cases:** OWN06/OWN07. **Files:** `safestos/cranelift/src/handles.rs`,
`umbrella/src/interface.rs`, `lib/{Common,CstUtil,TypingPass,Monomorphize,Stages,Compiler_cps}.ml`,
`umbrella/modules/WidgetHandle.{aui,aum}`, `umbrella/tests/handles.rs`.

- **Done:** Shared immutable & exclusive mutable `Borrow` claims, `__borrow`/`__view`/`__borrow_mut`/`__write`/`__release` wrappers, `Handle_Ownership` `Write` pragma plumbing, opaque Austral `View` token (`holdView`/`holdViewMut`/`viewRead`/`viewWrite`/`release`).
- **Remaining:** Add Chest/Gem child-replacement invalidation fixture and compiler-negative view-leak/use-after-release fixtures; support multi-word/buffer view writes beyond one 64-bit word; test reentrant overlapping mutation (`OWN07`).
- **Accept:** Stale child access and conflicting mutable borrows refuse before touching freed memory; static vs runtime checks recorded.

### U15 — Handle Result/Option, panic and all execution failures explicitly
**Status:** `implemented with gaps`. **Depends:** U10/U13.
**Finding:** UR11, UR17 (P0). **Cases:** ABI01/ABI06/ABI08, OWN08, CLI06.
**Files:** `umbrella/src/main.rs`, `umbrella/src/interface.rs`,
`safestos/cranelift/src/{handles,lib}.rs`, `lib/{TypingPass,CheckedOptions,Compiler_cps,BuiltInModules,BuiltIn}.ml`,
`umbrella/tests/{checked_adapters,either_adapters,either_allocations}.rs`.
**Reference (`../rust` §3.3):** `rustc_codegen_cranelift/src/{unwind_module.rs,debuginfo/unwind.rs}`
(explains why unwinding across Cranelift JIT frames fails without `.eh_frame`
registration, validating local wrapper `catch_unwind` + U23 worker handling).

- **Done:** `STATUS_OK`/`STATUS_REFUSED`/`STATUS_PANIC` ABI on handle & checked wrappers; `Option[T]`/`Either[L,R]`/`Result[T,E]` typed admission + `__checked` + `__free` ownership-transfer release (`5000/5000` alloc/free verified); `-C panic=abort` child fixture.
- **Remaining:**
  1. **Close UR17 (P0):** Check `cranelift_last_error()` / `JIT_PANIC` after `execute_function` in `umbrella/src/main.rs` (deduplicating the two call sites) so a panicked helper never emits `RESULT -9223372036854775808` or JSON `"done"`.
  2. Emit JSON structured stage/status failure events (`CLI06`).
  3. Support aggregate payloads beyond single scalar per case and handle non-matched/nested-expression transfer limits cleanly.
- **Accept:** Every failure has a distinct stage/status; `i64::MIN` domain returns remain distinguishable from `JIT_PANIC` via `cranelift_last_error()`.

### U16 — Add scoped synchronous callbacks safely
**Status:** `implemented with gaps`. **Depends:** U14/U15.
**Cases:** API04–API07, OWN07. **Files:** `safestos/cranelift/src/callbacks.rs`,
`umbrella/src/{interface,adapters}.rs`, `umbrella/tests/callbacks_retained.rs`.

- **Done:** `CallbackRegistry`/`CallbackScope`/`CallbackToken`/`CallbackPolicy` (`Lifetime`, `ThreadAffinity`, `Reentrancy`, `ErrorPolicy`); `__with` drops lock across user code; retained `<target>__with__retain`/`__invoke_retained`/`__retire_callbacks` + `subscribe` protocol; generation retirement on `adapters::load_for_jit`.
- **Remaining:** Emit `callback` policy and `subscribe` descriptor lines from compiler `Handle_Ownership` pragmas; support Austral first-class/scoped callback invocation beyond host tests; test cross-thread error refusal under `ErrorPolicy` variations (`API06`).
- **Accept:** Retired/stale/wrong-thread/reentrant tokens refuse before jumping or creating Rust references; callback panics contain cleanly.

### U17 — Build and link a mixed module/workspace graph
**Status:** `implemented with gaps`. **Depends:** U05/U07/U10. **Finding:** UR08.
**Cases:** CPS03/CPS04, API02. **Files:** `umbrella/src/{frontend,main}.rs`,
`lib/{Compiler,CliParser,CliEngine,Vm_plugin}.ml`, `umbrella/tests/entry.rs`.

- **Done:** `frontend::read_module_sources` + cycle/duplicate checks + Kahn topological ordering; `--emit-cps-bundle` emits all modules in one CPS artifact (`EitherProducer`/`EitherConsumer` cross-module test).
- **Remaining:** Support explicit project dependency graphs spanning multiple Rust crates/versions, aggregate adapter documents from *all* compiled modules (not only the last module), and qualified cross-module identity collision detection (`CPS04`).
- **Accept:** Multi-crate + multi-module graphs resolve exact qualified providers regardless of argument order; duplicate short names resolve or fail explicitly.

### U18 — Make cache invalidation and generation replacement correct
**Status:** `implemented with gaps`. **Depends:** U08/U09/U13/U17.
**Findings:** UR05/UR10/UR13. **Cases:** RUN07/RUN08, LNK04–LNK06, AUT04, OWN09, API05.
**Files:** `umbrella/src/{cache,adapters,main}.rs`, `safestos/cranelift/src/handles.rs`,
`umbrella/tests/cache.rs`.

- **Done:** Content-addressed `cache.rs` keying front-end artifact, provider cdylib, and adapter crate over full visible inputs; corrupt entries rebuilt; hits still run `trust::verify_*` and `UK-4001`; `load_for_jit` drains older generations' handle tables before retiring callbacks.
- **Remaining:** Cache JIT object products safely (or document worker reuse); add file locking/atomic rename for concurrent cache invocations (`RUN08`); cover path dependencies outside the crate dir / build-script `rerun-if-changed` inputs.
- **Accept:** Cold/warm outputs match; concurrent cache runs (`RUN08`) never corrupt entries; drained old handles never resolve in new tables.

### U19 — Enforce actual filesystem capabilities
**Status:** `implemented with gaps`. **Depends:** U09/U13. **Finding:** UR01, UR22.
**Cases:** FS01–FS08, AUT06. **Files:** `safestos/cranelift/src/{capstd_mod,module,lib}.rs`,
`umbrella/src/{project,main}.rs`, `safestos/cranelift/Cargo.toml`.

- **Done:** `RootDir` (`O_DIRECTORY | O_NOFOLLOW`) + per-component `openat` walking, per-root `FsRights` (`read`/`write`/`create`/`list`/`delete`/`rename`), `renameat2(RENAME_NOREPLACE)`, cumulative write quota, revocation (`rs_cap_revoke`/`revoke_all`), symlink-swap race test (`400` iterations), project JSON `capabilities.{rights,roots,quota_bytes}`.
- **Remaining:** Add `openat2(RESOLVE_BENEATH)` / external hardlink defense (`FS03`), per-root live usage quotas (`FS08`), binary-level `umbrella --project` fs capability execution test (`AUT06`/`FS01`–`FS07`), and restore offline `capstd_tests.rs` buildability without `unfer-kernel` conflicts.
- **Accept:** Outside sentinels stay unchanged across traversal/symlink/hardlink/race/quota tests; denied operations return exact reason (`CAP_ERR_REFUSED`/`CAP_ERR_REVOKED`).

### U20 — Implement bounded network capabilities
**Status:** `implemented with gaps`. **Depends:** U09/U13/U19.
**Cases:** NET01–NET06. **Files:** `safestos/cranelift/src/{netstd_mod,capstd_mod,module,lib}.rs`,
`umbrella/src/{project,main}.rs`, `safestos/cranelift/tests/fixtures/tls/`.

- **Done:** `NetPolicy` (endpoints, CIDR `IpNet`, `rustls`+`ring` trust anchor, bounded deadlines), `rs_cap_net_{dial,read,write,shutdown,unsupported}`, zero-accept denied listener test, DNS-rebinding range check, TLS identity verification, HTTP redirect re-check, manifest swap narrowing, project `capabilities.network`.
- **Remaining:** Add explicit proxy-hop fixture (`NET03`), cross-instance socket-token reuse refusal (`NET05`), and full binary-level `umbrella --project` network dial test; bound/reclaim abandoned resolver threads (`getaddrinfo`).
- **Accept:** Denied endpoints accept 0 connections; DNS rebinding/redirects/TLS mismatches/UDP-listen-raw refuse with typed errors.

### U21 — Expose useful ordinary Rust libraries through checked adapters
**Status:** `implemented with gaps`. **Depends:** U11/U12/U13/U14/U15/U17/U19/U20.
**Cases:** API01–API03, ABI06. **Files:** `umbrella/src/{interface,cli,main}.rs`,
generated Austral bindings, local adapter crates, `umbrella/tests/text_adapters.rs`.

- **Done:** `--descriptor-document` with `--emit-wrappers` / `--build-wrappers --provider` builds loadable `libumbrella_adapters_1.so`; `abi text 1` (`TextAdapter`) verified in `text_adapters.rs`.
- **Remaining:** Wire `TextAdapter` into compiler + `umbrella/src/main.rs` module run path (UR18); generate Austral `.aui`/`.aum` bindings from schema; build and test an ordinary Rust crate exercising free functions, inherent methods, `Drop`, concrete generic instantiations, owned collections, and one capability-aware fs/net client path with explicit trust classification (`capability-aware` vs `trusted-ambient`).
- **Accept:** Generated Austral + Rust wrappers perform genuine library calls matching a direct Rust oracle; all unsafe seams documented and boundary-tested.

### U22 — Replace the prototype demo with a real capability workflow
**Status:** `implemented with gaps`. **Depends:** U01/U16/U21. **Finding:** UR01, UR23.
**Cases:** HAR02, FS01/FS04, NET01, AUT02/AUT03.
**Files:** `examples/modules/cap_fs/`, `examples/modules/rust_kv/`,
`lib/Compiler_cps.ml` (`__embed$2` fix), `umbrella/` project demo.

- **Done:** `examples/modules/cap_fs/` executes `CapFs.aum` through `modhost` with `rs_cap_open_root`/`rs_cap_read`/`rs_cap_write`, matching an independent `rustc` oracle under input perturbation while refusing `../outside/sentinel.txt` and `UK-4001` missing grants.
- **Remaining:**
  1. **Fix UR23:** Lower `positiveOffset`/`load`/`store` memory builtins properly in the JIT (`__embed$2` resolution) so `CapFs.aum` performs real buffer transformations rather than raw copy.
  2. Drive the full workflow through `umbrella --project` (exact Rust adapter build + `ProviderSet` + scoped callback + granted fs output + loopback net dial + perturbation).
  3. Replace `examples/modules/rust_kv/run_demo.sh` with the real positive/negative e2e workflow and update `HAR02` accordingly.
- **Accept:** One `rust_kv` / `umbrella` workflow executes real positive & negative fs/net/callback paths matching an independent Rust oracle under perturbed inputs.

### U23 — Supervise native workers and bound execution
**Status:** `pending`. **Depends:** U06b/U09/U15. **Findings:** UR09/UR11/UR20.
**Cases:** RUN01–RUN03/RUN06, API06. **Files:** **new** `umbrella/src/worker.rs`,
worker binary/IPC framing, invocation state, crash/timeout fixtures.

1. Run **one loaded native generation per supervised worker process** for the
   production profile, containing legacy global auth/symbol/loader state.
   Authenticate local IPC frames with instance identity and bound frame/output sizes.
2. Parent enforces hard deadlines, cancellation, supported `rlimit` bounds,
   and child-tree reaping (`kill(-pgid, SIGKILL)`), distinguishing failure
   before vs after an external effect.
3. Test native infinite loops, `.init_array` initializer hangs, traps, `SIGABRT`,
   `SIGSEGV`, callback deadlocks, and worker crash mid-call; parent stays
   responsive and emits structured status.

**Accept:** Zero native faults become success values, parent hangs, or orphans.
(Lifecycle containment only; `U26` establishes sandbox/escape claims.)

### U24 — Fuzz, sanitize and mutation-test the boundaries
**Status:** `pending`. **Depends:** U10b/U14/U15/U16/U19/U20/U23.
**Cases:** HAR04, CPS05 and matrix controls.
**Files:** **new** parser/descriptor/CPS/IPC fuzz targets, regression corpus,
`scripts/test-umbrella` `robustness` lane.

1. Fuzz CLI, JSON project, Cargo stream, CPS header/IR, descriptor text, status
   tags, handle words, and worker IPC under size/depth limits; save minimized
   reproducers.
2. Run pure Rust adapter/handle/codec crates under **Miri** and worker/adapter
   tests under **ASan/LSan** (recording JIT limitations explicitly rather than
   silently skipping).
3. Run all 10 required negative mutation controls from
   [the test matrix](docs/umbrella/TEST_MATRIX.md#required-negative-controls)
   in disposable copies: verify baseline green, edit applied, target case fails
   for intended reason, copy cleaned.

**Accept:** Fixed-seed fuzz corpus green, zero sanitizer errors on covered
targets, 10/10 mutation controls verified.

### U25 — Lock artifact identity and package a relocatable toolchain
**Status:** `pending`. **Depends:** U05/U06b/U07/U08/U10/U10b/U18.
**Findings:** UR13/UR15/UR22. **Cases:** PKG01–PKG03/PKG07, AUT05.
**Files:** **new** `umbrella doctor` command, bundle/install layout,
`safestos/pkg/` (`PLAN_avm_pkg.md`) & `D24` claim manifest integration.

1. Bind `Cargo.lock`/features/target, compiler/bridge/`avm-adapter-rt`/schema
   ABI versions, provider sha256, and relevant transitive native `DT_NEEDED`
   identities; close TOCTOU replace windows via immutable staging/open-fd pinning.
2. Package into a fresh relocatable prefix (removing `../austral` and
   `current_exe()/deps` assumptions); add `umbrella doctor` to diagnose missing
   or version-mismatched tools/libs without running target code. Align signed
   bundle manifests with `PLAN_avm_pkg.md` (`.avm` cells) and `PLAN_distributed_security.md` D24.
3. Test offline locked rebuilds, moved install prefix, `RUSTFLAGS`/`LD_PRELOAD`
   tampering, and mismatched bridge/`avm-adapter-rt` handshake refusals.

**Accept:** Installed demo runs outside the source checkout; wrong versions or
tampered transitive dependencies fail handshake before execution.

### U26 — Validate an untrusted-native/build isolation profile
**Status:** `pending`. **Depends:** U02/U19/U20/U23.
**Cases:** RUN02/RUN04/RUN05/RUN06. **Files:** `safestos/cranelift/src/sandbox.rs`,
`cloud_hypervisor_vm/` / [`docs/spectrum_plan.md`](docs/spectrum_plan.md),
untrusted build & runtime escape fixtures.

1. Wire explicit isolation tiers for `untrusted-native`: OS confinement via
   `safestos/cranelift/src/sandbox.rs` (user/net/ipc namespaces, `no_new_privs`,
   seccomp-bpf, Landlock) and/or hypervisor VM/unikernel isolation via
   [`docs/spectrum_plan.md`](docs/spectrum_plan.md) (Cloud-Hypervisor / Uhyve /
   Hermit). Specify mounts, network proxy, resource caps, and capability RPC.
2. Isolate **both** Cargo build scripts/proc macros (`RUN05`) and runtime native
   execution (`RUN04`); test ambient `std::fs`, raw sockets, `.init_array`
   escapes, fork bombs, and memory exhaustion.
3. Keep `untrusted-native` disabled in preflight until its escape suite passes
   on the target host.

**Accept:** Every promised escape fixture executes and has its marker blocked.

### U27 — Complete native target and platform validation
**Status:** `pending`. **Depends:** U11/U25. **Cases:** ABI07, BLD05, PKG04.
**Files:** platform target validation module, CI runner matrix, ABI vectors.
**Reference (`../rust` §3.3):** `rustc_codegen_cranelift/src/abi/{pass_mode.rs,mod.rs}`
and `rustc_abi/src/callconv.rs` for `x86_64` vs `aarch64` ABI divergence rules
(`RegKind`, `HomogeneousAggregate`, `PassMode::Cast`/`Indirect`).

1. Execute identical ABI, ownership, capability, and e2e suites natively on
   **Linux `aarch64`** and **Linux `x86_64`**.
2. Validate ELF/architecture/pointer-width/endianness/target-triple metadata
   separately for build target vs worker execution target.
3. Refuse unsupported OS/arch or cross-target binaries before `dlopen`.

**Accept:** Both Linux `aarch64` and `x86_64` pass all required lanes; any
unmeasured target is excluded from release claims.

### U28 — Wire strict CI and coverage inventory
**Status:** `pending`. **Depends:** U00b/U22/U23/U24/U25/U27 (`U26` for isolated).
**Cases:** HAR01/HAR03, AUT06, CPS07, PKG06.
**Files:** `.github/workflows/` umbrella job, `scripts/test-umbrella`,
`scripts/verify-invariants` hooks.

1. Fast PR lanes run `cli`, `build`, `abi`, `capability`, and touched `e2e`;
   `robustness` (fuzz/sanitizers/mutations) and multi-arch `package` run on
   bounded schedules with saved evidence. Never require building `../rust` on
   PRs.
2. Enforce `REQUIRED_CASES` census, feature-enabled `capstd`/TLS coverage,
   owned/borrowed API census, and descriptor golden drift; integrate `D18`
   required-verification checks.
3. Preserve `check-bridge-fresh`, `isa-census`, `check-todo-ledger`, and
   `verify-invariants`.

**Accept:** Removing any required case, disabling a required feature, or
stubbing a guard fails CI with bounded, readable logs.

### U29 — Write operator/debugger and compatibility runbooks
**Status:** `pending`. **Depends:** U28. **Cases:** PKG01/PKG02/PKG06.
**Files:** `umbrella/README.md`, **new** `docs/umbrella/TROUBLESHOOTING.md`,
`docs/umbrella/ABI_COMPAT.md`.

1. Document `build`/`check`/`run`/`doctor`, `trusted-native` vs
   `untrusted-native`, supported types/targets, build-script trust, and residual
   `unsafe` seams.
2. Publish stage-by-stage diagnostic decision tree (`parse -> preflight -> cargo
   -> artifact -> frontend -> descriptor -> load -> auth -> worker/JIT -> IO -> cleanup`).
3. Document schema/ABI migrations, provider reload, revocation, rollback, and
   effect-uncertain recovery.

**Accept:** Clean-environment operator resolves faults without checkout-internal
paths or unverified claims.

### U30 — Establish resource/performance budgets and final release gate
**Status:** `pending`. **Depends:** U18/U22/U23/U25/U29.
**Cases:** PKG05, FS08, RUN02/RUN07/RUN08.
**Files:** **new** bounded benchmark/soak script, release evidence summary.

1. Measure cold/warm build, no-change rebuild, scalar/buffer/handle/callback
   crossing latency vs direct Rust, and realistic fs/net e2e throughput.
2. Soak-test repeated worker generations/reloads/cancellations for RSS, mmap,
   FD, child, and `/tmp` growth bounds; calibrate explicit budgets per target.
3. Gate release on 100% required cases, 10/10 mutation controls, zero
   unexplained skips, clean fault handling, documented residual `unsafe`, and
   verified relocatable package (`U26`/`U32` gated separately).

**Accept:** Measured evidence supports the exact advertised profile and enforces
regression budgets.

### U31 — Optional asynchronous host integration
**Status:** `pending` (extension). **Depends:** U16/U20/U23.
**Cases:** API08. **Files:** future/task handle adapter, worker protocol, cancel tests.

1. Keep Rust futures/executor state inside the host; expose linear pending/result
   handles with explicit poll/wakeup/cancel semantics.
2. Test completion/cancel races, double completion, worker crash mid-future,
   and effect-uncertain outcomes; forbid borrowed Austral buffers across await.

**Accept:** Zero UAF, leaked tasks, or false rollback claims; until passed,
product supports synchronous adapters only.

### U32 — Optional pinned Rust type-oracle/direct-import plugin
**Status:** `pending` (extension). **Depends:** U10/U10b/U14/U17/U25/U27.
**Cases:** ORC01–ORC06. **References (strictly read-only):** `../Valen`
(`b2aa19bd`), `../rust` (`c6f757802519a0fc2619dd79e0be879601899b06`, UR24 &
§3.3); keep both reference checkouts strictly read-only and unmodified.

Split cleanly based on the 2026-11-27 audit (UR24 & §3.3):

#### U32a — Read-only `rustc_driver` descriptor & ABI oracle (preferred, stock `rustc_private`)
1. Publish a capability/gap report comparing stable adapters vs direct-import
   requirements (methods, concrete generics, `Drop`, `fn_abi_of_instance`,
   `layout_of`).
2. Build an out-of-process `rustc_driver` oracle pinned to an exact stock
   toolchain sysroot/fingerprint (no compiler fork required) using
   `callbacks.after_analysis(compiler, tcx) -> Compilation::Stop` after
   `tcx.ensure_ok().analysis(())` (`rustc_driver_impl/src/lib.rs:374-382`) to
   query `tcx.fn_abi_of_instance` and `tcx.layout_of` (`rustc_middle/src/ty/layout.rs:732,1370`),
   classify `PassMode`s (`rustc_codegen_cranelift/src/abi/pass_mode.rs`), and
   emit the same `umbrella/src/interface.rs` descriptor schema
   (`--descriptor-document`). Never embed `rustc` inside the OCaml compiler.
3. Verify cold/warm query invalidation (`ORC04`), refusal of unsupported
   pass modes/lifetimes/layouts (`ORC03`), and borrow-checked wrapper generation
   matching direct Rust oracles (`ORC02`, `ORC06`).

#### U32b — Deferred AOT-only study of `per-instance-mir` hooks (optional, no `../rust` fork)
1. `../rust` (`c6f75780251`) is treated strictly as a read-only design study
   proving `per_instance_mir` / `fill_extra_modules` is LLVM-AOT-only and unused
   by `rustc_codegen_cranelift/src/base.rs:47` (§3.3). Do not create or
   maintain a fork of `../rust`.
2. Revisit only if an explicit LLVM-AOT cross-language synthesis requirement
   cannot be served by `U32a` + stable Cranelift wrappers; stage any such
   experiment in its own pinned build/toolchain artifact, testing warm-build
   query firing, placeholder optimizer attributes (`ORC05`), and borrow checking
   (`ORC06`). Never allow `--no-borrow-check` in any production profile.

**Accept:** `U32a` (and `U32b` if ever pursued) passes `ORC01`–`ORC06` behind
its own feature/toolchain gate and routes all execution through the production
host's provider/capability boundary; ordinary projects never require it.

---

## 7. Debugging order, recommended execution queue and stop rules

### 7.1 Prioritized execution queue (from 2026-11-27 state)

1. **`U10b` (`avm-adapter-rt` extraction)** — fixes P0 link bloat (UR15: ~585 MB → <1 MB, ~20 min → <2 s) and unblocks fast iteration + U25.
2. **`U00b` (harness sync + standalone feature decoupling)** — wires all 17 `umbrella/tests/` suites and fixes stale pins (UR21, UR22).
3. **`U06b` (child supervision, pipe bounds, staging, Cargo checks, exit code)** — closes UR16, UR18 (`2` → `1`), UR19.
4. **`U15` completion (`JIT_PANIC` + `cranelift_last_error()` check in `main.rs`)** — closes UR17 (P0).
5. **`U21` + `U22` completion (`TextAdapter` wiring, `__embed$2` JIT memory fix UR23, ordinary Rust library demo, real `rust_kv` e2e)** — closes R3 capability/interop gaps.
6. **`U23` (supervised worker processes)** — contains UR20 global state and makes U24 crash/fuzz testing safe.
7. **`U24` → `U25` (with D24 + `avm-pkg` seam) → `U27` → `U28` → `U29` → `U30` (R4 release gate)**.
8. **Optional post-R4 tracks:** `U26` (`R4-isolated`), `U31` (async), `U32a` (stock `rustc_driver` descriptor oracle).

### 7.2 Debugging order and stop rules

When a case fails, first locate the last completed pipeline stage and verify
tool/target identity. Reduce to a local fixture, compare the direct Rust oracle,
then inspect generated schema/declaration, CPS signature, resolved provider, and
runtime context. Probe invalid pointers/aborts/UB only in disposable children.

Do not repair wrong native results by changing goldens, disabling optimizer
checks, extending `--allow-all`, making linear handles `Clone`, switching
borrows to raw pointers, suppressing sanitizer failures, accepting arbitrary
`.so` paths, or turning errors into success.

If two small repair attempts do not reduce the same failure, record the exact
symptom, input, competing explanations, and next discriminating experiment. Split
the task rather than expanding into a compiler rewrite.

---

## 8. Master execution record & copyable assignment

Reconciled across `PLAN_valen.md` (P0–P3), `docs/umbrella/EXECUTION.md`
(`U00`–`U22`), and `PLAN_distributed_security.md` / `PLAN_gpu_kernels.md` shared
seams as of **2026-11-27**. Allowed statuses: `pending`; `reproducing`;
`implemented with gaps`; `passed within stated profile`; `experimental/unsupported`.

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| `PLAN_valen` P0–P1 (group borrowing) | `018-group-borrow` | passed within stated profile | `ATTRIBUTION.md`, `lib/AbstractionPass.ml`, `test-programs/suites/018-group-borrow` (5 cases) | `python3 test-programs/runner.py 018-group-borrow` | Nested linear borrow desugaring pre-CPS; negative escape/alias rejections pass | Does not authorize simultaneous Rust `&mut` across FFI |
| `PLAN_valen` P2–P3 (`extern rust` + prototype) | `019-extern-rust`, `SUITE_TIER1`, `SUITE_RSCRATE` | passed within stated profile | `RS_FFI_SYMBOLS`, `safestos/rsffi`, `umbrella/modules/{Demo.aum,rscrate}`, `test-programs/suites/019-extern-rust` | `python3 test-programs/runner.py 019-extern-rust` (8/8); `tier1_extern_rust` (6/6); `rscrate` (2/2) | `Rust_Ownership` checked pre-CPS; `rs_*` manifest-gated (`UK-4001`); FNV-1a `"au!"` matches | Superseded on driver path by U02–U22 strict pipeline |
| U00 harness & baseline | HAR01, HAR03, BLD08 | implemented with gaps | `scripts/test-umbrella`, `umbrella/tests/support/`, `docs/umbrella/EXECUTION.md` | `python3 scripts/test-umbrella --lane cli --strict --json` (0); `--lane build --strict --json` | Strict selection/gap/child-fail contracts verified; 12 `SUITE_*` pins wired | 12 of 17 `umbrella/tests/` suites unwired; counts stale (`U00b`) |
| U00b harness sync & feature decoupling | HAR01/HAR03, AUT06, PKG06 | pending | — | — | — | UR21, UR22 |
| U01 demo honesty | HAR02 | implemented with gaps | `examples/modules/rust_kv/{run_demo.sh,fixtures/printed_success.txt}`, `scripts/test-umbrella` | `bash examples/modules/rust_kv/run_demo.sh` (exit 3); `scripts/test-umbrella --lane capability --strict --json` (0) | Refuses fake PASS; catches `echo PASS` mutant | Replaced by real e2e in U22 |
| U02 strict schema & preflight | AUT01, BLD06, RUN06 | implemented with gaps | `umbrella/src/{project,policy,main}.rs`, `umbrella/tests/{policy.rs,fixtures/stageprobe}` | `cargo test --offline --lib --test policy` (5 + 6 passed) | Unknown fields named; `untrusted-native` refused; stage markers ordered | Grants still passed separately via `--manifest`; blocking initializers need U23 |
| U03 CLI grammar & JSON mode | CLI01–CLI06 | implemented with gaps | `umbrella/src/{cli,main}.rs`, `umbrella/tests/cli.rs` | `cargo test --offline --lib --test cli` (20 + 9 passed as of U21) | Exact flags, non-UTF-8 paths, exit 2 on usage without spawning tools | `main.rs:761` call-denial exits 2 instead of 1 (UR18) |
| U04 strict entry & compiler transport | ABI01/ABI02, BLD05 | passed within stated profile | `safestos/cranelift/src/{cps,lib}.rs` (`compile_entry_i64`), `lib/TypingPass.ml`, `019-extern-rust/008..009` | `strict_entry_*` (7 passed); `runner.py 019` (8/8); `test/entry` | Rejects empty/missing/wrong-arity/non-i64 entries & raw invalid scalar imports | Foreign ELF arch check before `dlopen` in `U06b`/`U27` |
| U05 exact Cargo artifact selection | BLD01–BLD05, BLD09, PKG07 | implemented with gaps | `umbrella/src/{cargo,main,cli}.rs`, `umbrella/tests/cargo_select.rs`, `fixtures/cargo-workspace` | `cargo test --offline --test cargo_select` (5 passed); `SUITE_UMBRELLA_CARGO` | Proc-macros/deps ignored; verbatim `package_id` match; `RUSTC_WRAPPER` refused | `build-finished`, `target.kind`, ELF check, strict `RUSTFLAGS`/config policy (`U06b`) |
| U06 bounded child & staging | BLD07, CPS02, RUN03 | implemented with gaps | `umbrella/src/{process,staging}.rs` | `cargo test --offline --lib process::tests` (6 passed); `staging` (2 passed) | `process_group(0)` + `SIGKILL` reaps grandchildren; private `Staging::create` | Raw `Command::output()` & `/tmp/umbrella-adapters-*` remain (`U06b`) |
| U06b supervision/staging/Cargo hardening | BLD03/BLD05/BLD07, CPS02, RUN03, PKG07 | pending | — | — | — | UR16, UR18, UR19 |
| U07 compile-only CPS artifact | CPS01/CPS02/CPS05–CPS07 | implemented with gaps | `lib/{Compiler,Compiler_cps,CliParser,CliEngine}.ml`, `umbrella/src/{frontend,staging,main}.rs` | `dune build`; `019/010` `--emit-cps-only` → 182 B CPS; `cargo test --lib frontend::` (22 passed as of U17) | Emit-only never initializes JIT; drops `--allow-all`/`CPS_JIT_ADVISORY`; threads `--liquid-required` | Adapter doc still read from last module only (U17) |
| U08 explicit provider binding | LNK01–LNK04, AUT05 | implemented with gaps | `umbrella/src/{provider,adapters,policy,main}.rs`, `umbrella/tests/{entry,linking}.rs` | `cargo test --offline --test linking` (1 passed); `provider::tests` | `RTLD_LOCAL` + `dlsym(handle)` beats preloaded symbol; duplicate exports refused | Multi-provider per project; remove `RTLD_GLOBAL` fallback (`main.rs:744`, `adapters::bind`) |
| U09 per-instance identity & policy | AUT02–AUT04, LNK03 | implemented with gaps | `safestos/cranelift/src/{auth,module,cps}.rs`, `umbrella/src/{project,main}.rs` | Reduced bridge `auth::tests` (13 passed); `project::tests` | A/B snapshots isolated; narrowed rebind refuses with `UK-4001` | Retire `safestos_load_auth_manifest` call (`main.rs:378`); concurrent worker tests (U23) |
| U10 checked adapters & trust pins | ABI03, ABI06, API03 | implemented with gaps | `umbrella/src/{interface,adapters,trust,project,main}.rs`, `umbrella/tests/{interface,entry}.rs` | `cargo test --test interface` (7 passed); `trust::tests`; `--emit-trust-record` | Typed signatures & `descriptor_sha256` checked before `ProviderSet::load` | Comment-sensitive digest; no generated Austral bindings; ~585 MB link bloat (`U10b`) |
| U10b `avm-adapter-rt` extraction | ABI03–ABI08, OWN03–OWN09, PKG01/PKG05 | pending | — | — | — | UR15 (P0 link bloat & `current_exe()/deps` globbing) |
| U11 scalar conversions & JIT path | ABI04/ABI05 | passed within stated profile | `safestos/cranelift/src/adapters.rs`, `umbrella/src/interface.rs`, `lib/Compiler_cps.ml`, `umbrella/modules/AdapterLive.aum` | `cargo test --test scalar_roundtrip` (3 passed); `SUITE_ADAPTERS` (9); `SUITE_SCALAR_ABI` (6) | 8 scalars match direct Rust bit-for-bit (`-0.0`/NaN); bad widths refused | Linux `x86_64` measured; repeat natively on `aarch64` (U27) |
| U12 buffers & `abi text 1` | OWN01/OWN02 | implemented with gaps | `umbrella/src/interface.rs`, `lib/{TypingPass,Compiler_cps}.ml`, `umbrella/tests/{buffer_roundtrip,text_adapters}.rs` | `buffer_roundtrip` (3 passed); `text_adapters` (3 passed); `check-adapter-descriptors` | Exact byte extent, NUL preservation, SIGABRT on bad length; `&str` → `String` | `TextAdapter` not wired in compiler/`main.rs` (UR18); capacity/UTF-8 error status |
| U13 linear handles & destructors | OWN03–OWN05/OWN08/OWN09, LNK05 | implemented with gaps | `safestos/cranelift/src/handles.rs`, `umbrella/src/{interface,adapters}.rs`, `WidgetHandle.{aui,aum}`, `025-linear-handles` | Bridge `handles` (13); umbrella `handles` (24 as of U16); `025-linear-handles` (3/3) | Generational two-word handles; single Drop; generation bump refuses old handles | `__with` unused from modules; generation exhaustion edge test (`OWN05`) |
| U14 borrowed views & aliasing | OWN06/OWN07 | implemented with gaps | `safestos/cranelift/src/handles.rs`, `umbrella/src/interface.rs`, `lib/*`, `WidgetHandle.{aui,aum}` | Bridge `handles` (13); umbrella `handles` (21+); `WidgetHandle` compiles | Shared immutable / exclusive mutable `Borrow`; live borrow blocks `__dispose` | Chest/Gem replacement fixture, negative view fixtures, multi-word views |
| U15 checked unions, `__free`, panic ABI | ABI01/ABI06/ABI08, OWN08, CLI06 | implemented with gaps | `safestos/cranelift/src/handles.rs`, `umbrella/src/{interface,main}.rs`, `lib/{CheckedOptions,Compiler_cps,TypingPass,BuiltIn*}.ml` | `checked_adapters` (3); `either_adapters` (4); `either_allocations` (3: `5000/5000` alloc/free, `0` leaked) | `Option`/`Either`/`Result` `__checked` + `__free`; `STATUS_PANIC` in wrappers; `-C panic=abort` child | **`main.rs` omits `JIT_PANIC`/`cranelift_last_error()` check (UR17 P0)**; aggregate payloads |
| U16 scoped & retained callbacks | API04–API07, OWN07 | implemented with gaps | `safestos/cranelift/src/callbacks.rs`, `umbrella/src/{interface,adapters}.rs`, `umbrella/tests/callbacks_retained.rs` | Bridge `callbacks` (12 passed); `callbacks_retained` (3 passed) | Call/instance lifetime, thread affinity, reentrancy, `subscribe`, generation retirement | Compiler pragma emission for `callback`/`subscribe`; first-class module callbacks |
| U17 module graph & CPS bundling | CPS03/CPS04, API02 | implemented with gaps | `umbrella/src/{frontend,main}.rs`, `lib/{Compiler,CliParser,CliEngine,Vm_plugin}.ml`, `umbrella/tests/entry.rs` | `frontend::` (22 passed); `CliParserTest` (5); `entry` (11+); `either_allocations` (3) | Cycle/duplicate refusal; Kahn topological sort; single `--emit-cps-bundle` | Multi-crate qualified identity; aggregate adapter docs across all modules |
| U18 run cache & generation drain | RUN07/RUN08, LNK04–LNK06, AUT04, OWN09, API05 | implemented with gaps | `umbrella/src/{cache,adapters,main}.rs`, `safestos/cranelift/src/handles.rs`, `umbrella/tests/cache.rs` | `cache::` (6); `test cache` (7); `test entry` (12); `draining` (1) | Content-addressed front-end/provider/adapter cache; drain before callback retire | JIT caching; concurrent cache locking (`RUN08`); path deps outside crate dir |
| U19 rooted filesystem capabilities | FS01–FS08, AUT06 | implemented with gaps | `safestos/cranelift/src/{capstd_mod,module,lib}.rs`, `umbrella/src/{project,main}.rs` | Reduced bridge `fs_tests` (14 passed incl. 400-iter symlink race); `project::` (15) | `openat`/`O_NOFOLLOW` `RootDir`, per-root rights, quota, revocation, project schema | `openat2 RESOLVE_BENEATH` hardlinks; per-root quota; project binary run; `capstd_tests` offline link |
| U20 bounded network capabilities | NET01, NET02, NET04, NET06 | implemented with gaps | `safestos/cranelift/src/{netstd_mod,capstd_mod,module,lib}.rs`, `umbrella/src/{project,main}.rs`, `tests/fixtures/tls/` | Reduced bridge `net` (14 passed); `project::` (15 passed) | Endpoint/CIDR/rustls verification, zero-accept denial, redirect re-check, UDP/listen refusal | Proxy-hop fixture, socket-token cross-instance reuse, project binary dial run |
| U21 descriptor wrappers & text | API01–API03, ABI06 | implemented with gaps | `umbrella/src/{cli,main,interface}.rs`, `umbrella/tests/text_adapters.rs` | Reduced `cli` (20 + 9 passed incl. `build-wrappers` + `dlopen`); `text_adapters` (3 passed) | `--descriptor-document` `--emit-wrappers`/`--build-wrappers`; `abi text 1` | Wire `TextAdapter` in compiler/`main.rs` (UR18); methods/`Drop`/generics/collections/fs-net client demo |
| U22 capability workflow demo | FS01/FS04, HAR02 | implemented with gaps | `examples/modules/cap_fs/` (`CapFs.aum`, `module.toml`, `reference.rs`, `run.sh`), `capfsdemo.rs` | `bash examples/modules/cap_fs/run.sh` (reduced bridge features → exit 0, perturbation + `UK-4001` verified) | Compiled Austral module reads/writes granted roots matching `rustc` oracle; traversal refused | Fix `__embed$2` JIT memory builtins (UR23); `umbrella --project` fs/net/callback e2e; replace `rust_kv` |
| Shared seams: Distributed (`D08`/`D09`/`D17`/`D18`) & GPU (`G14`) | — | verified within stated scope (`D24` pending) | `umbrella/modules/{distributed_io,DistributedRequest.aum,GpuService.aum}`, `umbrella/tests/distributed_{request,tls}.rs` | `distributed_request` (1); `distributed_tls` (2); `G14` via `gpu.yml`; `D18` via `check-verification-profile` | Per-instance identity, pinned build/load, mTLS linear remote handles, `--liquid-required`, GPU service | `D24` claim/artifact manifest integrates in U25 |
| U23–U32 (excl. U32a/U32b) | RUN01–08, HAR04, PKG01–07, API08, ORC01–06 | pending | — | — | — | Remaining production & optional extension work |

Copyable assignment prompt for the next execution turn:

```text
Execute Uxx substep N from PLAN_umbrella_production.md.
Read its review finding (URxx), matrix rows, applicable AGENTS.md and dependency
evidence in docs/umbrella/EXECUTION.md. Preserve distributed/Bend plan docs and
unrelated dirty files. Keep ../Valen and ../rust strictly read-only and
unchanged (consult §3.3 as a design reference only). Create the smallest
reproducing fixture, apply a bounded patch, and run the targeted test plus
affected suite (and `make bridge` if bridge Rust changed). Native crash/UB
fixtures run only in children. Do not weaken grants, ABI/ownership contracts,
goldens, or proof statements to get green. Report exact command/status, oracle,
evidence paths and remaining gaps; update §8 and docs/umbrella/EXECUTION.md.
Stop at the next bounded patch.
```

---

## 9. References, provenance and licence discipline

- **Valen** (`../Valen`, `b2aa19bdcd01570efba6684fac9a3d08a43c7d37`,
  Apache-2.0, read-only reference): design inspiration only (group borrowing,
  `bifrost/extern_abi.rs` compiler ABI querying, `orchestrator.rs`,
  Chest/Gem borrow invalidation tests). No source copied; see
  [`ATTRIBUTION.md`](ATTRIBUTION.md) and `../ATTRIBUTION.md`.
- **Rust / Valen Rust fork** (`../rust`, `c6f757802519a0fc2619dd79e0be879601899b06`
  on branch `per-instance-mir`, MIT/Apache-2.0, strictly read-only reference,
  unbuilt): reference blueprints for Cranelift JIT symbol lookup and unwinding
  (`rustc_codegen_cranelift`), Rust ABI/layout (`rustc_abi`, `rustc_middle`),
  `rustc_driver` lifecycle hooks (`rustc_driver_impl`), and the audited
  `per-instance-mir` study (UR24, §3.3). Ideas adapted only; no fork maintained
  and zero source copied.
- **Sibling authoritative plans:**
  [`PLAN_distributed_security.md`](PLAN_distributed_security.md) (and
  [review](docs/DISTRIBUTED_SECURITY_REVIEW.md)),
  [`PLAN_gpu_kernels.md`](PLAN_gpu_kernels.md),
  [`PLAN_avm_pkg.md`](PLAN_avm_pkg.md),
  [`docs/spectrum_plan.md`](docs/spectrum_plan.md),
  [`PLAN_liquid_types.md`](PLAN_liquid_types.md),
  [`PLAN_sota_kernel_verification.md`](PLAN_sota_kernel_verification.md).
- **Paused Bend plans (excluded):**
  [`PLAN_umbrella_bend.md`](PLAN_umbrella_bend.md),
  [`PLAN_bend_austral_integration.md`](PLAN_bend_austral_integration.md),
  [`docs/LIQUID_BEND_PLAN.md`](docs/LIQUID_BEND_PLAN.md),
  [`docs/SAFESTOS_PARALLEL_PLAN.md`](docs/SAFESTOS_PARALLEL_PLAN.md).
