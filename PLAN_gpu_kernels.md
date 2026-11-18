# Austral-authored GPU kernels: implementation plan

**Status, 2026-11-18.** G00–G09 are recorded as implemented. The compiler emits
validated WGSL bundles, and the device runner has a recorded Vulkan adapter
probe. Start with G10 for dispatch and readback. G11–G23 remain pending,
including the optional MLIR backend. See the [execution record](docs/gpu/EXECUTION.md).

Read [the research review](docs/GPU_KERNEL_REVIEW.md), then the proposed
[v1 contract](docs/gpu/KERNEL_V1.md). The original programme is
[unfer/GPU.md](../unfer/docs/GPU.md) and its partially implemented follow-up,
[timepiece/GPU_FEDERATION_PLAN.md](../timepiece/GPU_FEDERATION_PLAN.md).
This plan adds the kernel compiler that those documents deferred. It preserves
their implemented DMA, layout, diagnostics and proof-verification work.

## 1. Outcome and architectural decisions

The first useful release takes an ordinary Austral function over `Nat32`,
checks a small pure/total GPU subset, emits a real compute shader, validates it
without a GPU, then executes that shader over arrays on an identified hardware
adapter. The scalar computation comes from the Austral source body. A runner
that merely calls an existing handwritten CUDA/WGSL kernel does not satisfy it.

The chosen pipeline is:

```text
.aui/.aum → existing typed frontend and linearity
          → closed GPU subset check → typed KernelIR + memory plan
          → WGSL + versioned metadata
          → Naga validation → wgpu dispatch → completion/readback
```

Keep C and CPS/Cranelift unchanged for existing CPU targets. Do not lower CLIF
into GPU code, reinterpret CPU pointers as GPU pointers, or fall back to C when
the GPU target fails. GPU admissibility is additional to existing checks.

Use WGSL/wgpu first because a small emitter and validator can deliver a real
kernel without introducing an LLVM build or NVIDIA-only runtime. Existing
federation WGSL examples provide a reference. The typed KernelIR is also the
boundary for a later **MLIR backend**, retaining the old programme's direction.
That extension gets explicit tasks G19–G21 rather than an undocumented promise.

Proof-producing compilation is a first-class companion workstream:
[PLAN_proof_chain.md](PLAN_proof_chain.md) and its
[design contract](docs/PROOF_CHAIN.md) specify reusable MM0 application theorems,
representation semantics, lowering certificates and typed residual obligations
across CPU/GPU/NPU branches. Reserve these interfaces in G02 instead of attaching
an unstructured proof-status label after code generation. Native instruction
coverage remains the long-term goal; an IR proof is an explicitly earlier cut.

This is a compiler/backend project. NPU-specific lowering, distributed
scheduling, FreeToken, automatic tensor partitioning, arbitrary Rust imports,
full GPU-program verification and tensor-core tuning are not first-release
requirements. The [distributed plan](PLAN_distributed_security.md) and
[umbrella plan](PLAN_umbrella_production.md) retain their separate scopes.

### Release boundaries

| Milestone | Tasks | Exit evidence |
|---|---|---|
| M0: frontend and semantics | G00–G04 | Real source fixtures typecheck; supported closure has an explicit typed IR; forbidden constructs refuse |
| M1: generated device artifact | G05–G08 | Austral source → WGSL bundle → Naga validation and C/IR differential checks on CPU-only CI |
| M2: first hardware release | G09–G12 | Generated shaders run with bit-exact results on a recorded physical adapter; required tests do not fall back/skip |
| M3: stronger host/evidence | G13–G15 | Wrapper obligations scoped honestly; authorized module calls and real pending/completion ownership |
| M4: numerical usefulness | G16–G18 | Explicit Float32 profile, bounded numerical kernels and one optional unfer integration |
| M5: native backend option | G19–G21 | Pinned MLIR path and measured NVIDIA launch ABI/hardware execution |
| M6: parallel kernel extensions | G22–G23 | Explicit address/layout/barrier model and measured multiple-device dispatch |

M0–M2 are the default execution scope. M3–M6 are ordered extensions with their
own acceptance boundaries; do not start all backends and optimizations together.
If hardware is unavailable, finish CPU-validatable work, record the hardware gap,
and keep M2 incomplete. An empty dispatch or software adapter does not close it.

M2 is an execution-tested kernel release, not completion of the compositional
proof goal. Coordinate PC00/PC01 with initial interface work; PC02–PC07 can build
the first reused source/IR/WGSL proof without physical hardware. Later PC tasks
close actual CPU/GPU/NPU boundaries individually. No missing preservation proof
may be filled by a new unchecked axiom or an encoded statement-of-record.

## 2. Instructions for the executing LLM

The intended executor is a lower-cost coding model. Work one task card at a
time; do not infer missing GPU semantics from a success message.

1. Read root `AGENTS.md`, and `safestos/AGENTS.md` before changing that subtree.
   Before a `[SYNC]` task, read the destination's instructions. Modify australVM
   for G00–G17/G19–G23; G18 explicitly permits the listed unfer paths. Other
   sibling checkouts are references, not mutation targets.
2. Inspect `git status` before edits. Existing proof-script and numerical work
   is unrelated; preserve it. Do not reset, clean, stage or commit other work.
3. Read the current card plus its named sources and the relevant contract
   section. Implement its stated scope and its smallest meaningful tests.
   Prefer at most one compiler concern and one test concern per patch. Split
   a card if necessary, recording subtasks without changing its acceptance.
4. Use resolved IDs/types, typed ASTs and structured data. Never parse source
   again with regex, render arbitrary C as WGSL, or identify trusted operations
   by bare names. Enumerate unsupported cases; no silent constant/default.
5. Do not change language semantics to pass the GPU test. In particular,
   `modularAdd(Nat32)` is not `trappingAdd(Nat32)`, Float32 is not Float64,
   a bank relabelling is not an arbitrary address transformation, and
   enqueue is not device completion.
6. Add a failing regression before fixing a claimed defect. Differential tests
   need hand-computed controls and an independent source/C path; comparing two
   consumers of the same incorrect IR is insufficient.
7. Record commands, exit codes, evidence paths, adapter identity, limitations and
   relevant source/tool digests. A missing compiler, shader error or timeout
   must not become “no GPU, skipped.” Only an actual optional adapter probe may
   report an unavailable adapter.
8. Never mark a task verified because code compiles or another task passed.
   Use `pending`, `in progress`, `implemented with gaps`, `verified within stated
   scope`, or `blocked: <specific prerequisite>`. Failure to obtain hardware is
   a hardware gap; it does not erase passing CPU evidence.
9. Keep progress logs small. Preserve one compact summary per task and paths to
   detailed temporary/log artifacts. Do not paste an entire build transcript or
   historical status table into the next prompt.
10. No general refactor, physics-suite sweep, new theorem approval, plugin system,
    Nix rebuild of every environment, or second GPU backend unless the card
    requires it. Preserve license/attribution if adapting reference runtime code.

### Build and command profiles

Run from the australVM root. These are **execution instructions**, not commands
already run by this planning change.

```sh
# OCaml/C compiler work: every dune command stays inside this devshell.
nix develop "git+file://$PWD#ocaml" -c make all
nix develop "git+file://$PWD#ocaml" -c dune exec test/GpuSubsetTest.exe

# Isolated Rust shader/runtime work; this crate is created by G07.
cargo test --locked --manifest-path safestos/gpu/Cargo.toml
cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features device

# Whole compiler integration after relevant compiler changes.
nix develop "git+file://$PWD#ocaml" -c make test
```

`GpuSubsetTest.exe` and other GPU test executables are proposed names. Add them
to the explicit dune module lists before using them. Before G07 creates the
crate/lock, its cargo commands are not available. G00/G07 may resolve and lock
new dependencies once; thereafter use `--locked`. Offline mode is appropriate
when the needed crates are cached, not a reason to silently change versions.

Use `git+file://$PWD` to avoid copying the large worktree into Nix. Newly needed
flake input files must belong to Nix's source snapshot; stage only the intended
new files if the invocation requires that. Do not run bare `nix-shell ... dune`.
If the default proof shell is unbuildable, `.#ocaml` can build but does not
prove. G13 must record missing/unknown Why3 as a gap. It cannot call an OCaml-only
build a successful proof run.

The existing release bridge must be fresh for compiler links. Touching bridge
sources requires the documented `make bridge` under the same shell. The initial
isolated GPU crate should avoid bridge edits and new default GPU dependencies.
Do not rebuild the bridge after every WGSL-only edit.

### Planned gate interface

G12 implements `scripts/check-gpu` with these modes:

```text
scripts/check-gpu source       # frontend subset, CPU/C oracle and generated shader validation
scripts/check-gpu optional     # source lane plus device probe; may report hardware gap
scripts/check-gpu required     # source lane plus actual physical-device corpus; no skip/fallback
```

Optional mode may exit 0 with a structured `hardware=unavailable` result after
passing source checks, but it must print `[gap]` and cannot generate a hardware
pass record. Required mode converts that condition into exit 1. Gate output must
contain separate `source`, `shader-validation`, `device-kind`, `execution` and
`proof` results. Do not collapse them into one `verified` value.

## 3. Fixed first-release contract

[KERNEL_V1.md](docs/gpu/KERNEL_V1.md) is authoritative. The important decisions
are already made so the executor does not need to redesign them:

- Entry: one or two `Nat32` parameters, one `Nat32` result, mapped across
  equal-length arrays; no arbitrary storage accesses from author code.
- Immutable scalar computation, conditionals, acyclic local helpers and exact
  `Nat32` modular/bitwise intrinsic identities. No traps, casts, recursion,
  loops, foreign IO, pointers, shared memory or barriers in v1.
- Explicit KernelIR widths, a compiler-generated identity-index memory plan,
  64 threads/workgroup and a release limit of 1048576 elements/job.
- `kernel.json` plus `kernel.wgsl`; deterministic serialization, SHA-256 byte
  bindings and bounded parsing. These provide provenance/staleness checks,
  not adversarial authentication or a proof of compiler correctness.
- Native Linux is the first tested platform. Other OS/backend support is
  advertised only after measurement. A software Vulkan adapter is useful for
  tests but does not count as physical GPU execution.
- Naga/wgpu 29.0.4 initially match the existing federation lock. The isolated
  crate's default feature set does not acquire a device.

## 4. Task cards

### G00 — Record the baseline and toolchain constraints

**Depends:** none. **Size:** small. **Files:** new `docs/gpu/EXECUTION.md`;
initial `safestos/gpu/Cargo.toml`/lock preparation only if necessary for the probe.

Inspect the named source paths in the research review. Record revisions, dirty
paths, host architecture, Rust version, compiler/bridge availability, and the
actual Nix shells used. Confirm the current wgpu/Naga and CUDA versions from
locks; do not upgrade the workspace. Record whether crates are cached or need
resolution. Read the local `delta_algebra` example as a reference only.

Do not infer compute capability from `/dev/dri`. The proper adapter probe is
G09. The baseline can record it as not run. Inspect the existing build/test
baseline with bounded relevant commands; if a pre-existing failure prevents
building, preserve its smallest reproduction and continue independent design/
Rust checks. Do not fix unrelated physics or proof work.

**Accept:** a reproducible source/tool inventory and a documented command profile;
no claim of a GPU run. **Handoff:** first pending card is G01; baseline failure
paths are explicit rather than hidden in inherited README counts.

### G01 — Expose the checked typed frontend without changing CPU codegen

**Depends:** G00. **Size:** medium, split refactor/tests if needed.
**Files:** `lib/Compiler.{ml,mli}`; a small new frontend helper only if needed;
`test/GpuFrontendTest.ml`, `test/dune`; real source fixture directory.

Factor the existing parse/combine, return check, desugaring, extraction, typing,
plugin execution and linearity stages into one reusable preparation path. It
returns the typed module, environment and source identity needed by a backend.
Keep body extraction/environment updates correct for following modules. Run
normal checks exactly once per module; do not register an artifact-emitting
plugin that fires on every builtin module.

The future GPU target uses that preparation path and captures only the selected
user module. Normal initialization still uses canonical builtin sources. Keep
C, typecheck, CPS and hot-swap paths behaviorally unchanged; test their use of
the shared stages rather than mechanically replacing every pipeline at once.

Add real `.aui/.aum` fixtures for identity, a conditional clamp and two-argument
modular arithmetic. Typecheck their actual signatures and public visibility.
The contract's example must become runnable source, not only a string in a test.

**Check:** targeted frontend/CLI tests in `.#ocaml`, then `make test` once after
the refactor. **Accept:** typed Nat32 parameters and resolved arithmetic method
IDs are observable; a malformed/linearity-invalid fixture still fails normally;
no JIT function is compiled/executed by the new frontend entry.

### G02 — Define typed KernelIR and deterministic metadata

**Depends:** G01. **Size:** small/medium. **Files:** new
`lib/gpu/{Gpu_ir.ml,Gpu_ir.mli,Gpu_bundle.ml,Gpu_bundle.mli,dune}`;
`docs/gpu/` schema notes/fixtures; `test/GpuIrTest.ml`, dune wiring;
`flake.nix` and `dune-project` for the hashing dependency.

Coordinate PC01 before freezing this schema: reserve the optional proof-manifest
reference and exact semantic subject identities from `docs/PROOF_CHAIN.md`.
Coordinate the constructor meanings with PC02; code generation and proof
emitters must not maintain incompatible copies of the numeric/memory model.

Keep the dune dependency graph acyclic: the GPU library depends on
`austral_core` and pure support libraries; `austral_lib` depends on it. The GPU
library must not import `Compiler`/`CliEngine` back from `austral_lib`.

Implement the exact structured v1 IR and memory-plan variants. Separate semantic
IR from diagnostic/source locations. Freeze JSON keys/encoding with small byte
fixtures and a literal outside signed 32-bit range, including `4294967295`.
Canonical bytes must not depend on OCaml hash iteration, absolute checkout path
or a process-local declaration ID. Renumber local/helper IDs deterministically.

Use `Digestif.SHA256` for OCaml byte hashing and Rust `sha2` for the independent
reader. Add the OCaml dependency to the GPU dune library, the authoritative
`dune-project` package declaration and both applicable OCaml/proof shell inputs;
regenerate the derived opam file rather than hand-editing it. Do not implement
a new SHA algorithm, substitute `Digest`/FNV, or introduce a JIT call to hash IR.
Specify SHA-256 domains and cross-language test vectors. Source closure and
builtin byte digests are separate from semantic IR identity. Include actual
compiler identity or `unknown`, never a guessed git revision. Implement resource
limits and bounded graph traversal. Do not add a shader cache yet.

**Accept:** deterministic serialization on rerun and equivalent checkout paths;
round trips retain types/values; unknown IR variants and out-of-range literals
refuse. **Control:** semantic literal edit changes semantic bytes; source-only
comment edit changes the source digest while leaving semantic bytes unchanged.

### G03 — Admit the scalar AST whitelist, with complete refusals

**Depends:** G02. **Size:** medium. **Files:** new `lib/gpu/Gpu_subset.{ml,mli}`;
`test/GpuSubsetTest.ml` and source fixtures; relevant dune lists.

Implement every expression/statement constructor's accept/refuse decision.
Handle immutable `TLet` and compiler-generated initialization temporaries without
accidentally admitting reassignment. Preserve ordinary type/linearity failures;
do not attempt to prove GPU safety by calling `TotalityCheck` alone.

Start with identity, literal/parameter/local, comparison, Boolean and conditional
forms. Reject unsupported types before serialization. Keep a table/test covering
all current `Tast` constructors so a newly added constructor cannot gain a
wildcard pass. Include forbidden forms nested inside call arguments and branches.

**Accept:** positive source fixtures lower into typed IR. Negative fixtures cover
while/for, mutation/borrow, pointers/spans/indexing, arbitrary embed/foreign,
64-bit/floats/casts, function pointers/dynamic dispatch, records/unions and unsafe
user modules. Diagnostics name the rejected form and containing function/span.
No bundle is published by a refused source.

### G04 — Resolve helper closure and trusted integer intrinsics

**Depends:** G03. **Size:** medium, split helper/intrinsic patches.
**Files:** new `lib/gpu/Gpu_intrinsics.{ml,mli}`; `Gpu_subset`; relevant `Env`
read-only accessors if necessary; frontend/IR tests.

Walk the complete reachable closure, detect direct/mutual cycles, and reject
cross-user-module helpers in v1. Check constant initializers transitively too.
Admit only canonical `Nat32` modular add/subtract/multiply and bitwise instances
from `lib/builtin/Pervasive.*`, using resolved owner/typeclass/type/method IDs.
Treat each intrinsic as an audited semantic leaf. Keep its signature check and
source identity; never recurse into arbitrary `@embed` and then whitelist it.

Test alias imports, a same-named user function, a fake builtin module, a helper
that calls a foreign function, a helper-hidden loop, a function value, and a
mutual call cycle. Calls to `trappingAdd`/divide/remainder remain refused even
when the runtime inputs would happen to be safe.

**Accept:** `modularSubtract(1,2)` has u32 modular semantics in IR; helper graphs
are finite/bounded; no textual-name spoof gains intrinsic status. The full
M0 corpus passes its positive/negative expectations.

### G05 — Add the WGSL bundle compilation target

**Depends:** G04. **Size:** medium. **Files:** `lib/CliParser.{ml,mli}`,
`CliEngine.ml`, `Compiler.{ml,mli}`, `lib/dune`; CLI tests and help.

Add a typed GPU target variant carrying source entry and output directory. The
command in the contract selects it explicitly. Reject incompatible flags before
compilation. Route to checked frontend → GPU closure → IR; do not enter the
CPS try/catch/fallback path or the host executable linker.

Implement staged bundle publication and “destination exists” refusal. At this
stage it may publish a clearly incomplete IR-only scratch result in tests;
public successful WGSL compilation waits for G06/G07. Do not accept an empty
`kernel.wgsl` to make a CLI test green.

**Accept:** help and parser agree; unknown/missing entry and incompatible flags
have actionable errors; a deliberately unsupported source never emits C or runs
an entrypoint. Existing C/TC/JIT command fixtures retain their old behavior.

### G06 — Render actual WGSL from KernelIR

**Depends:** G05. **Size:** medium. **Files:** new `lib/gpu/Gpu_wgsl.{ml,mli}`;
`Gpu_bundle`; `test/GpuWgslTest.ml`, examples/goldens.

Generate scalar helper bodies, deterministic safe names and the map wrapper
from the memory plan. Reserve WGSL identifiers rather than emitting user names
verbatim. Emit concrete u32 literals; fold modular constant operations using
u32 semantics where WGSL constant-expression rules would otherwise differ.
Preserve branch structure and argument order. Emit read-only inputs/fresh output
bindings, the pre-load index guard and fixed workgroup size.

Keep goldens small: identity, noncommutative subtraction, conditional and a
helper-based modular polynomial. Goldens check rendering structure; G07/G08
check validity/meaning. Bundle digests cover the resulting exact bytes.

**Accept:** the rendered arithmetic is visibly derived from the source IR;
a source-body mutation changes the appropriate shader/semantic digests; no
handwritten shader substitutes for a fixture's computation.

### G07 — Add the independent Rust bundle/Naga checker and IR evaluator

**Depends:** G06. **Size:** medium, split crate/schema/evaluator patches.
**Files:** `safestos/gpu/{Cargo.toml,Cargo.lock,src/lib.rs,src/schema.rs,src/artifact.rs,src/validate.rs,src/wrapper.rs,src/reference.rs,src/bin/avm-gpu.rs}`;
crate tests; `Gpu_bundle` and a small argument-vector tool-invocation helper.

Use one isolated crate with default CPU-only validation/reference features.
Implement bounded reads, strict schema parsing, duplicate-key rejection,
canonical IR digest reproduction and WGSL byte digest verification. Add Naga
29.0.4 with WGSL input. Parse and validate with capabilities required by this
subset; do not disable checks. Reflect entry stage, workgroup size and declared
binding roles/types against metadata.

Implement the contract's fixed-pattern Naga memory-wrapper audit as a separate
small patch: index provenance, dominating guard, exact input reads and one
identity-index output store. Scalar helpers cannot reach storage globals.
Reject unsupported shader memory/control forms instead of trying to prove
arbitrary alias/control-flow equivalences. Test a removed guard, colliding or
off-by-one write index, extra store, helper storage access and wrong input extent
before any of these scratch shaders reaches hardware.

Evaluate validated IR with Rust `wrapping_*` u32 operations, ordinary Boolean
semantics, and a bounded call/instruction budget. Enforce type/scope validity
before evaluation. Implement `validate` and `reference` CLI commands from the
contract with stable structured errors; device subcommands may still be clearly
unavailable until G09.

Recognize the reserved proof-manifest reference from PC01. It cannot grant a
proof status by itself: a proof-required policy delegates to the proof-chain
checker, which checks theorem applications, exact subjects and residual
premises. Ordinary validation remains a narrower evidence class.

Wire the compiler's staging-directory validation to this checker using
`AUSTRAL_GPU_CHECKER`, then PATH. Require the expected structured schema/digests
and exit 0 before rename/publication; missing tool, timeout or malformed response
refuses the GPU target. Use argument-vector execution with bounded output and
child cleanup, not shell interpolation. Finish the public successful compilation
path left incomplete in G05. Document building/installing this checker before
using `--target-type=wgsl`.

**Accept:** cross-language canonical bytes/hashes agree; all compiler bundles
pass Naga. Truncated JSON, duplicate/unknown fields, invalid references, oversized
files, mismatched shader/IR digests, wrong bindings and malformed WGSL refuse.
This task's success is shader validation, not a GPU execution pass.

### G08 — Establish a source/C/IR differential corpus

**Depends:** G07. **Size:** medium. **Files:** `examples/gpu/pointwise/`;
`test-programs/suites/023-gpu/` (confirm the next free suite ID at execution);
new `scripts/gpu-source-check` or helpers under `scripts/gpu/`; fixture data.

Run the same authored source through the ordinary C backend with an Austral
CPU driver, and through GPU IR emission plus the Rust reference evaluator.
Use hand-computed expected results for simple cases. Do not use the existing
CPS path as the sole oracle for Nat32/bitwise semantics; it has a different
lowering history.

The required scalar corpus includes identity, min/clamp, asymmetric subtraction,
bitwise and/or/xor/not, add/multiply wrapping at boundaries, local helpers,
constant references and branch selection. Required boundary values include 0,
1, 2147483647, 2147483648, 4294967294 and 4294967295. Add a fixed-seed bounded
random corpus with its generator/seed recorded.

**Accept:** ordinary source execution, expected fixtures and IR results agree.
Mutation controls swap subtraction operands and reverse a comparison; each must
fail. A source helper silently omitted by lowering must also fail. Compiler
refusals are checked without a GPU or shell-dependent output scraping.

### G09 — Probe the actual device and classify errors

**Depends:** G07; can follow G08 independently. **Size:** small.
**Files:** `safestos/gpu/src/device.rs`, CLI/feature wiring; device tests.

Add optional wgpu 29.0.4 with the native backend features deliberately selected.
Implement `probe`, including backend/name/vendor/device/type and relevant limits.
Use actual adapter enumeration/selection; do not infer physical hardware from
OS device-node existence. A user selector must be explicit and validated.

A software CPU adapter or unknown device type cannot satisfy `--require-hardware`.
Adapter absence has its specified optional/required exit semantics; request-device
failure, validation failure and device loss are different errors. Test result
classification with injected probe outcomes without pretending mocks ran a GPU.

**Accept:** the device-independent crate still builds/tests without optional
GPU dependencies enabled. The actual host probe produces either recorded
adapter evidence or a precise unavailable/error record. No hardware claim yet.

### G10 — Execute a validated bundle with correct buffers/completion

**Depends:** G08,G09. **Size:** medium, split preflight/dispatch/readback patches.
**Files:** `safestos/gpu/src/{device,run}.rs`, CLI; runtime tests.

Implement checked input parsing and preflight limits before creating buffers.
Bind input STORAGE buffers, a fresh STORAGE|COPY_SRC output, and a COPY_DST|
MAP_READ staging buffer, with exact element sizes and the contract's binding
order. N=0 needs no zero-sized allocation/dispatch. Use checked sizes and group
counts, request the necessary limits, and refuse unsupported requirements.

Capture shader/pipeline validation errors through the pinned wgpu mechanisms.
Submit once, retain live allocations until actual completion, map/read/copy the
result, drop mapped views, then unmap. A worker-owned synchronous path is fine;
there is no async service responsiveness claim here. Use bounded waits/explicit
device-loss errors; a caller timeout does not authorize early buffer reuse.

**Accept:** one real nonempty generated identity/add/subtract shader round trip
on a recorded hardware adapter. On an unavailable host, code and CPU preflight
checks can pass but this acceptance remains `implemented with gaps`.
No CPU recomputation may be returned as a GPU result.

### G11 — Prove the hardware tests detect mistakes

**Depends:** G10. **Size:** medium. **Files:** crate integration tests;
`scripts/gpu/` test orchestration; source/vector fixtures.

Run the exact source-generated corpus against the C and IR oracles for array
sizes 0,1,63,64,65,127,128,129,1024, and the supported maximum in a bounded stress
case. Include unequal lengths, oversized dispatch, wrong bundle binding/profile,
and malformed input refusal before GPU submission.

In scratch artifacts, remove the guard, swap subtraction inputs, reverse a
comparison and create a colliding output-index plan. Compiler/structural checks
must catch unsafe wrapper mutations before hardware submission; semantic
mutations can be safely run and must disagree with expected vectors. Do not
launch a known unsafe kernel merely to observe wgpu robustness. Record that each
mutation actually applied and was caught by the intended check.

**Accept:** nonempty required hardware corpus passes; every relevant control
fails as designed. Tests report actual backend/device, not just a test count.
A software or skipped run cannot produce this acceptance record.

### G12 — Wire bounded gates, CI and the first release documentation

**Depends:** G11 for hardware release; G08 for CPU-only CI wiring.
**Size:** small/medium. **Files:** `scripts/check-gpu`, `.github/workflows/`;
root README, `examples/gpu/pointwise/README.md`, execution log.

Implement the gate modes in §2. CPU CI compiles source fixtures, checks refusal,
validates actual shaders with Naga and runs C/IR oracles. A separate explicitly
configured GPU runner uses required mode. A CPU CI pass never presents as GPU
coverage. Do not add wgpu/CUDA to all default compiler/bridge builds.

Document installation/locked dependency steps, authoring commands, the admitted
subset, diagnostics and exact not-covered claims. Measure baseline latency/
throughput against the same generated WGSL with a direct Rust host; report
compilation, upload, execution/readback and total timings separately. No required
speedup threshold for v1, and no comparison against unrelated physics workloads.

**Accept:** repeatable source gate plus at least one truthful hardware evidence
record. Update README test counts only from actual output. M2 is the first
release boundary; all optional tasks below can remain pending without disguising
that boundary's limitations.

### G13 — Generate and check narrow memory-plan obligations

**Depends:** G12; shader-validation work can precede hardware availability.
**Size:** medium. **Files:** new `lib/gpu/Gpu_obligations.{ml,mli}`,
`docs/gpu/` obligation specification; new proof fixtures/check script.

Use the actual memory plan to emit bounds, non-overflow and unique-write claims.
Use the shared semantic domains and obligation interfaces from PC02/PC05/PC06;
the objective is to reuse source/KernelIR properties and prove the new wrapper
conditions. Do not create another disconnected memory theory or treat cited
high-level digests as theorem applications.
For fixed workgroup size 64 and the admitted N bound, prove that the generated
flat index is representable; the guard implies index<N; two active distinct
invocations write distinct indices; inputs have sufficient extents; fresh output
cannot alias an input. State which facts the runtime validates and which hardware
invocation semantics are trusted.

Reuse Why3 as a subprocess with explicit unbounded-integer/machine-integer
bridges. Do not extract unbounded integers into native ints without bounds.
Source/bundle/plan digests accompany the obligation set. If the prover is missing,
optional mode reports a gap; a required proof profile refuses.

Why3 evidence and an independently checked MM0 certificate are separate outputs.
The MM0 path uses the proof-chain tasks to produce actual derivations with
explicit imported theorem applications and remaining premises. Why3 success,
Lean statement export or a finite witness does not satisfy an MM0-required claim.

**Accept:** real emitted obligations are discharged in the supported proof shell,
with exact assumptions recorded. Mutating the memory plan to wrap writes,
removing the guard or changing the extent must invalidate evidence or fail a
claim. Renderer/model correspondence remains trusted unless separately proved.
Do not repurpose fixed `Layout.lean` examples or auto-approve new W3 statements.

### G14 — Add an authorized module-facing GPU adapter

**Depends:** G12 and the relevant umbrella interop prerequisites.
**Size:** medium, split ABI/capability/integration patches.
**Files:** a small adapter under `safestos/gpu/`; `safestos/rsffi` or an explicit
umbrella adapter, manifest example and targeted tests; bridge only if unavoidable.

Expose a narrow versioned GPU service contract through the existing Rust interop
route. Prefer opaque context/job identities, explicit byte lengths and typed
errors; no wgpu Rust layout or device pointers cross the FFI. Define limits and
authorization for device selection, allocation and launch. Use the normal
manifest-gated foreign namespace; do not add privileged `au_*` names to bypass
checks. No new unfer kernel symbols are needed for a local adapter.

If current umbrella cannot carry the required owned types, implement its stated
narrow prerequisite or use an explicit checked ABI; do not claim the whole
umbrella production plan is complete. Rebuild the release bridge if touched.

**Accept:** an Austral host module launches a compiled Austral GPU artifact;
grant denial occurs before the selected operation; lengths/ownership/status
errors refuse; normal CPU/JIT module behavior is unchanged. Native trusted
adapters are still trusted code, not an OS sandbox.

### G15 — Connect linear jobs to actual asynchronous completion

**Depends:** G14. **Size:** medium, split state machine/wrappers/device tests.
**Files:** new `stdlib/gpu/Jobs.{aui,aum}`; adapter-owned job state;
compile-time and runtime fixtures.

Define abstract linear OwnedBuffer/InFlight/PendingJob/CompletedResult wrappers
with privileged constructors. Submission consumes the owned buffer/job; polling
consumes and returns either the pending token or its completed receipt. Runtime
identity includes context and generation. Enforce one outstanding job initially;
later capacity changes need explicit bounds.

`complete` must depend on an actual matched runtime event, unlike the current
pure Virtio record transition. Cancel-wait is distinct from cancel-device-work.
Retain/quarantine allocations until completion, acknowledged cancellation or a
reset contract makes release safe. Device loss cannot manufacture a successful
receipt. Integrate via a worker rather than blocking the controller executor.

**Accept:** use-after-submit/double-receipt source fixtures fail; stale/forged
runtime identities fail; a second controller request advances while the first
job runs; reordered/repeated completion and timeout retain correct ownership.
Mock lifecycle tests and real device evidence are reported separately.

### G16 — Add a deliberate Float32 numerical profile

**Depends:** G12; does not require G14/G15 for standalone kernels.
**Size:** medium. **Files:** source `lib/builtin/Pervasive.{aui,aum}` if adding
Float32 arithmetic instances; GPU intrinsic/IR/schema/emitter code; C/IR/GPU tests.

The current standard arithmetic class lists Float64 but not Float32. Add genuine
Float32 source support with ordinary CPU conformance tests before mapping it to
WGSL. Never map an existing Float64 program to f32 silently or edit the generated
`BuiltInModules.ml` directly. Initially admit add/subtract/multiply only and
reject float-to-int conversions, division and transcendental functions.

Version the numeric profile as an explicit opt-in (`wgsl-f32-v1`). Specify the
admitted finite domain, treatment of overflow/subnormals, permissible rounding/
contraction differences and per-operation test tolerances. Bound test inputs
and intermediates; unsupported exceptional behavior must not be used to justify
numerical correctness. A hardware result is compared against a controlled f32
reference, not only f64 with a generous unexplained epsilon.

**Accept:** a source-authored Float32 axpy/affine kernel executes on hardware;
normal, boundary, cancellation-sensitive and deliberately refused exceptional
cases follow the documented profile. No bit-exact cross-device claim and no
physical error certificate unless separately established.

### G17 — Add one bounded iteration/composite extension

**Depends:** G16. **Size:** medium; finish one subtask before another.
**Files:** versioned `docs/gpu/` extension contract; GPU IR/validator/emitter;
source/CPU/device fixtures.

First add fixed-size, explicitly laid-out numeric records/arrays only if the
chosen example needs them. Do not reuse host OCaml/Rust/C layouts: define offsets,
alignment, padding and WGSL address space, then test serialized byte fixtures.
After that, admit one static bounded scalar iteration with at most 1024 steps
per invocation and a total work budget; reject dynamic/unproved bounds.

This is a separate kernel profile, not a weakening of Liquid's total fragment
or permission for arbitrary `while`. Keep all outputs independent, without
shared memory or barriers. Demonstrate a real finite fold (e.g. a fixed small
polynomial/complex pointwise transform) authored in Austral.

**Accept:** representation/iteration source cases agree with CPU and GPU oracles;
changing a field offset or iteration count is caught; oversized/unproved loops
refuse. This does not yet implement a multi-workgroup reduction.

### G18 — Exercise one existing unfer workload [SYNC]

**Depends:** G16, and G17 only if the workload requires records.
**Size:** medium. **Files permitted in unfer:** `fock_sirk/Cargo.toml`,
`fock_sirk/examples/austral_pointwise.rs`, a small optional adapter module,
`docs/` recipe; generator/fixture sources in australVM `examples/gpu/`.

Use a simple amplitude/complex scale or bounded pointwise operation already
represented in `TensorState`/the Hermite reference. Keep a named independent
numerical oracle. This is an integration example first; do not replace the full
SIRK solver, sparse basis construction, whitening or physics acceptance suites.

Use an optional path dependency on the isolated GPU crate only if it has no
unfer/JIT dependency, avoiding a cycle (`unfer → GPU crate → unfer`). Compile
bundles explicitly with the toolchain in the recipe, not by launching an OCaml
compiler from an opaque cargo build script. Package the resulting exact artifacts.
Candle CUDA tensors and wgpu buffers are not interchangeable; initially use
explicit transfers and report their cost. Do not claim zero-copy.

**Accept:** source-authored kernel, Rust host and existing numerical oracle agree
for the stated finite corpus on hardware; feature-off unfer behavior stays the
same. CPU fallback/optional absence is visible. Record whether integration helps
or hurts end-to-end time before proposing solver replacement.

### G19 — Pin and smoke-test the optional MLIR toolchain

**Depends:** G12; no need to wait for numerical extensions.
**Size:** medium. **Files:** optional flake shell/tooling; `scripts/gpu/mlir-probe`;
small known-input fixtures; an exact version/pipeline record in `docs/gpu/`.

Add an optional MLIR/LLVM environment separate from `.#ocaml` and default CPU
builds. Use a pinned package with `mlir-opt`, `mlir-translate` and the necessary
NVPTX serialization/toolchain support. Inspect its help and supported passes;
do not paste an upstream latest-version pipeline blindly into this checkout.

First compile one tiny known module through `gpu`/`arith` to NVVM/device bytes.
Record tool/package digests, exact pass options, target chip/PTX version and the
emitted kernel parameter ABI. This plumbing fixture is handwritten MLIR and
must be labelled as such; it is not an Austral-to-MLIR acceptance.

**Accept:** repeatable tooling pipeline with actual output and missing-tool
failures. If its build cannot run, record the smallest blocker and preserve the
working WGSL backend; do not switch compilers or upgrade the whole workspace.

### G20 — Emit MLIR from the existing typed KernelIR

**Depends:** G19,G08. **Size:** medium, split scalar/memory rendering.
**Files:** new `lib/gpu/Gpu_mlir.{ml,mli}`; GPU target/metadata extension;
validator/golden/differential tests.

Use the same admitted u32 semantics and memory plan as v1. Emit typed `gpu`
kernel structure plus `arith` and required structured control/memory operations.
Do not round-trip through C, CLIF or a reconstructed WGSL AST. Distinguish signed
and unsigned comparisons, shifts/extensions, and index width. Start without
Float32, shared memory, barriers or fusion.

Run the pinned verifier/lowering pipeline on every corpus module. Serialize
backend identity, target, pipeline and tool digests. Prohibit the old C fallback.
A successful verifier run establishes MLIR well-formedness, not execution.

**Accept:** real Austral source emits valid/lowerable MLIR and device artifacts;
the source mutations change emitted operations; unknown targets fail. The
handwritten G19 fixture is not used in place of generated source output.

For composed proof evidence, PC09 additionally records every selected lowering
boundary and checks its semantic edge. MLIR verifier success alone does not
discharge preservation.

### G21 — Establish the native launch ABI and hardware oracle

**Depends:** G20. **Size:** medium; isolate ABI fixture from runtime integration.
**Files:** optional CUDA adapter in the isolated GPU crate; artifact metadata;
device tests and benchmark recipe.

Derive the actual lowered parameter ABI from emitted PTX/LLVM/device metadata,
then lock it with a minimal execution test. Do not guess that a memref is one
pointer: lowering may expand descriptors or require a pinned bare-pointer
calling convention. Choose one verified convention and serialize it explicitly.
Use the workspace-compatible CUDA/cudarc version or an isolated locked adapter,
without changing unfer's default CUDA stack.

Launch source-generated identity, asymmetric subtraction and wrapping arithmetic
on a named NVIDIA device. Verify counts, widths, pointer/address-space roles,
completion and readback. Compare outputs with the same C/IR oracles; compare
performance with WGSL only on the same device and equivalent data movement.

**Accept:** a second genuine source-to-GPU backend, with tested ABI and hardware
identity. No NVIDIA hardware means the hardware claim remains a gap even if PTX
was produced. Backend-specific evidence cannot certify the other backend.

PC10 separately tracks PTX-to-native lowering and the actual device instruction
artifact. These execution tests do not supply that MM0 proof; a PTX proof or a
native blob digest cannot satisfy a native-proof-required profile.

### G22 — Add a constrained tiled/shared-memory profile

**Depends:** G13,G17 and a working selected backend. **Size:** multiple small
patches, each with its own versioned contract and negative cases.
**Files:** new `docs/gpu/TILED_PROFILE.md`; typed address/memory-plan extension;
backend lowering; layout/obligation/test modules.

Begin with one fixed tile operation; no arbitrary barrier intrinsic in otherwise
unrestricted control flow. Model global/workgroup/private storage, indices,
active lanes and barrier participation. Padded inactive lanes must participate
in required barriers rather than take the v1 early return before them. Validate
workgroup storage and alignment from actual device limits.

Analyse races independently of performance. A bank-conflict claim is scoped to
bank count/width, lane access schedule, access size and any broadcast rule. Reuse
the existing finite bank examples as regressions and specify exactly which
address/padding transformations the search permits. Do not claim their fixed
bank relabelling theorem forbids all swizzles. Prove or runtime-check new bounds;
Naga uniformity/validation remains mandatory on the WGSL path.

**Accept:** one generated tiled kernel with safe edge tiles and non-power-of-two
sizes; a divergent barrier, overlapping write and bad padding/extent are refused
before launch. Numerical and performance measurements are separate results.
No generalized optimal-layout or deadlock-freedom claim without its own proof.

### G23 — Add bounded independent dispatch across multiple devices

**Depends:** G15 and one working device backend; no distributed-system expansion.
**Size:** medium. **Files:** runtime context selection, per-device queues/budgets;
module/source integration fixtures; device tests.

Keep kernels unchanged. Give each context an explicit backend/adapter identity,
independent capacity and memory budget. Submit independent jobs to two physical
devices and retain buffers in their own context. Reject cross-context receipts
or buffers; report one-device availability honestly. CPU parallel reference
execution is not a second GPU.

**Accept:** two physical devices make progress on generated kernels and match
oracles; one slow device does not occupy the controller's event loop; device loss
in one context does not manufacture success in another. Transfers, admission
wait and end-to-end time are measured. Multi-host scheduling/collectives remain
with the distributed plan and a separate numerical partitioning contract.

## 5. Test oracle matrix

| Case | Source/IR gate | Shader/runtime check | Expected result |
|---|---|---|---|
| Pure identity/map2 modular polynomial | Accept | Naga + source/C/IR/GPU comparison | Exact u32 equality |
| Overflow `MAX+1`, `0-1`, large multiply | Accept only modular intrinsics | C and wrapping Rust reference | Exact modular result |
| Trapping arithmetic or divide | Refuse | No dispatch | Named unsupported-semantics diagnostic |
| Foreign call/loop hidden in helper | Refuse closure | No bundle | Path to the forbidden construct |
| Spoofed builtin name/module | Refuse or ordinary helper rules | No intrinsic privilege | No unsafe/foreign bypass |
| Signed/64-bit/Float64 input in v1 | Refuse | No silent truncation | Type/profile diagnostic |
| Same source/relocated checkout | Stable semantic IR/WGSL | Digest agreement | Stable artifact bytes where specified |
| Stale/tampered metadata/WGSL | Schema/hash refusal | No dispatch | Explicit artifact error |
| N=0 | Accept input | No zero allocation/dispatch | Empty result, dispatch_count=0 |
| N=63/64/65 and ragged tail | Accept | Guard before load; exact extents | Exact results, no robustness dependence |
| Unequal inputs or insufficient device limits | Input/requirement refusal | Before allocation/submission | Capacity/layout error |
| Missing adapter | Source lane still runs | Optional gap / required failure | Never hardware success |
| Software adapter | Source/software lane allowed | Physical-required refusal | Labelled software evidence |
| Missing prover in proof-required mode | Compilation/evidence refusal | No “proved” record | Proof-unavailable error |
| Timeout/cancel-wait | Retain pending ownership | Completion/reset before reclamation | Unknown/pending, not success |

## 6. Evidence record and next-task prompt

Update the [execution record](docs/gpu/EXECUTION.md) after each task. Record the
task and status, source or contract identity, commands and evidence, and any
remaining gaps. A milestone remains open until its required evidence exists.
Preserve the recorded historical measurements and follow the existing
owner-comparison workflow for proof statement approvals.

Suggested prompt for the cheap executor:

> Read AGENTS.md, PLAN_gpu_kernels.md, docs/GPU_KERNEL_REVIEW.md and the relevant
> part of docs/gpu/KERNEL_V1.md. Inspect git status and the execution record.
> Start at G10 if it is still pending. Use the execution record to pick up the
> first pending task in M0–M2 whose dependencies are satisfied.
> Keep the patch limited to the card, run its meaningful positive and refusal
> tests, and update the evidence record with exact commands and limitations.
> Continue with the next eligible task. If a tool/hardware prerequisite is absent,
> finish independent CPU-validatable work and report that precise remaining gap;
> do not claim GPU execution, change numeric semantics or add a fallback to pass.
> Preserve unrelated work and do not auto-approve proof statements.
