# Next implementation steps for Austral GPU kernels

**Plan, 2026-11-19.** Start from G00–G15 as recorded in [EXECUTION.md](EXECUTION.md).
The first new source feature is a bounded integer fold. The first new device
schedule is an exact map-and-sum reduction. MLIR is an independent second
backend for the existing integer map corpus. This document breaks the remaining
[G-series cards](../../PLAN_gpu_kernels.md#4-task-cards) into smaller patches.

## Evidence and design choices

The compiler already checks a closed typed Austral subset, produces KernelIR,
emits a versioned WGSL bundle and validates it through Naga. The device runner
has recorded dispatch and readback on Vulkan. Its G11 hardware corpus covers
14 entries and 22,414 element results against C and KernelIR. The G15 service
test used two contexts on one selected physical adapter per run. G13 checks
Why3 memory-plan goals, while PC04 and PC06 have narrower MM0 artifacts with
explicit trusted or open premises. The next work can reuse these paths and
their source mutation controls. [Recorded work](EXECUTION.md),
[source/IR proof](../proof/SOURCE_IR.md), [memory-plan proof](../proof/GPU_PLAN.md).

Unfer's earlier programme proposed MLIR and FreeToken and implemented narrower
layout and DMA work. Its numerical code already runs Candle tensors on a
selected device. Use that as a workload and comparison point after the kernel
pipeline has a matching numerical profile. The Candle and wgpu storage
representations need an explicit transfer path for the initial integration.
[Original GPU programme](../../../unfer/docs/GPU.md),
[federation execution and layout work](../../../timepiece/GPU_FEDERATION_PLAN.md),
[unfer tensor implementation](../../../unfer/fock_sirk/src/tensor_state.rs).

| Finding from research | Consequence for AustralVM |
|---|---|
| Bend 2 lowers a pure fork into tasks and a join. Fixed placement depends on roughly balanced subproblems, and leaf work benefits from flat loops. | Start with a pure scalar body and statically bounded work per lane. Give each workgroup a defined output slot, then explicitly join the group results in later dispatches. [BendRT paper](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/bend2/docs/BendRT/main.typ), [shader guide](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/guide/SHADERS.md). |
| Bend hands a complete phase to one GPU. It documents one event loop, one GPU per program, and a deadline that can leave work running. | Keep Austral's host-owned pending token and completion receipt. Build bounded host placement on that protocol. Varying device speed and lifecycle must be handled at the host. [Bend limitations](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/README.md#limitations), [Bend IO guide](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/guide/GUIDE.md). |
| WGSL checks uniformity of collective operations. Adjacent lanes reading adjacent elements can reduce memory transactions on CUDA. | Tail lanes contribute identity and reach every barrier. Tune elements per lane only after measuring a fixed baseline on the same device. [WGSL uniformity](https://www.w3.org/TR/2026/CRD-WGSL-20261106/#uniformity), [CUDA memory guidance](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/index.html#coalesced-access-to-global-memory). |
| MLIR's GPU dialect provides a GPU-to-NVVM lowering route. The memref calling convention determines the actual launch parameters. | Feed typed KernelIR to MLIR directly. Start with static extents and verify emitted parameter types and layout from the pinned pipeline. [MLIR GPU dialect](https://mlir.llvm.org/docs/Dialects/GPU/), [MLIR LLVM ABI](https://mlir.llvm.org/docs/TargetLLVMIR/#calling-conventions). |
| CubeCL and Rust-GPU already let Rust authors write kernels. | Use a Rust/wgpu control with the same algorithm first to isolate host and emitter costs. Add a CubeCL comparison when an equivalent target and schedule are available. [CubeCL](https://github.com/tracel-ai/cubecl), [Rust-GPU](https://github.com/Rust-GPU/rust-gpu). |

Review pins are australVM `d430c13e5ccbf19faeb79ba469129a0cd40f205b`, unfer
`eb8c2a70f9baf78ed45b1d5ad7577dedfe08a65f` and Bend
`9c586f094f4fc88bef440dc8b4d77a2f92c257cc`. This plan uses original Austral
fixtures and repository-native implementation. The cited Bend revision carries
Apache-2.0. Check the provenance and license of any code actually imported in a
later task, including the integration obligations of any AGPL-covered reuse.
[Pinned Bend license](https://github.com/bendlang/bend/blob/9c586f094f4fc88bef440dc8b4d77a2f92c257cc/LICENSE).

## Pick one card at a time

| Workstream | Order | First deliverable |
|---|---|---|
| More useful WGSL kernels | G17a → G17b → G22a → G22b → G22c | One exact source-authored map-and-sum and a measured portable GPU run |
| MLIR | G19a → G20a → G20b → G21a | One exact source-generated NVVM/PTX launch from the same KernelIR, then corpus coverage |
| Proofs | PC05a → PC05b → existing PC06/PC07 | An MM0-checked transfer for the actual clamp shader with its trust boundary recorded |
| Numerical integration | G16a → G16b → G18 | One explicit Float32 map and a feature-gated unfer example |
| Local multiple GPUs | G15a → G23a → G23b | Bounded jobs on independently selected physical adapters |

For a single executor, begin with **G22a** (G17a/G17b landed 2026-11-19; see
[BOUNDED_FOLD.md](BOUNDED_FOLD.md)). G19a and PC05a can make progress on
the existing v1 corpus when the WGSL hardware lane is unavailable. G22a can
also start from the original v1 scalar mapper while G17b is in flight. G17 no
longer depends on Float32. G18 is the only card here that changes unfer.

Use the existing compiler and test entry points. Candidate new paths, flags
and profile names below are proposed and should first appear with their tests.
Read the repository instructions and `safestos/AGENTS.md` before editing the
Rust subtree. Start cards with `git status --short` and the execution record.

```sh
nix develop "git+file://$PWD#ocaml" -c make all
cargo build --locked --manifest-path safestos/gpu/Cargo.toml
cargo test --locked --manifest-path safestos/gpu/Cargo.toml --features service
python3 scripts/check-gpu source
```

Run targeted Dune commands inside `nix develop "git+file://$PWD#ocaml"`.
Run a physical lane using a `--features "device service"` checker and
`python3 scripts/check-gpu required`. Select and record the physical adapter
for new profile tests. Use the repository's configured proving environment for
proof-required checks. After each card record the exact command and exit code,
source/artifact hashes, target and profile, and the separate source, validation,
execution and proof verdicts. Record unavailable toolchains/hardware as a gap
and continue with an independent card. Keep the ordinary v1 source gate green.

## WGSL authoring and one reduction

### G17a — Establish loop semantics and the narrow frontend profile

**Files.** `lib/gpu/Gpu_subset.ml`, `test/GpuSubsetTest.ml`, a new paired
`.aui`/`.aum` example and a short profile contract under `docs/gpu/`.

Use an opt-in `--gpu-source-profile=bounded-fold-v1`. Admit one entry-body
`for i from 0 to K` with a compile-time literal `K` in `0..31`. The body
assigns one already initialized private `Nat32` accumulator from a v1 scalar
expression. The index is unused in the first fixture. Reject nested loops,
other writes, early return, effects, unknown bounds and loops in helpers.
Keep bound and binder decisions on the typed monomorphic AST. Write a probe
that records the actual resolved initializer, loop and assignment nodes.

Austral's specification and the C renderer use the inclusive endpoint. The
local CPS lowering used `CmpLt`; it now emits `CmpLte` (reconciled
2026-11-19, pinned by `test/CpsLoopLoweringTest.ml` and measured through the
JIT server), so C, KernelIR/WGSL and the CPS JIT agree. The fixture records
the resolved counts with the C executable and a hand computation as oracles.
[Austral loop specification](https://austral-lang.org/spec/spec.html#for-loop),
[C loop renderer](../../lib/CRenderer.ml), [CPS loop lowering](../../lib/Compiler_cps.ml).

**Exit.** Real source with `K=0` means one update and `K=31` means 32. Add
refusal cases for `K=32`, dynamic bounds, a second mutable binding and an
assignment through an address. The default WGSL profile still refuses `for`.
If the typed AST differs from the assumed shape, record it and update the
match before attempting the emitter. Keep this patch limited to admission and
the source oracle fixture.

### G17b — Expand the bounded fold into existing KernelIR

**Files.** `Gpu_subset.ml`, CLI profile plumbing in `CliParser.ml`/`CliEngine.ml`,
and the GPU source corpus. Reuse `Gpu_ir.ml`'s existing immutable locals.

Translate each body update using the accumulator binding for that iteration.
Create one fresh IR local for the next accumulator. Redirect subsequent reads
to the final binding. Check the projected expansion and helper evaluation
budget before allocating the expanded body. Count repeated helper calls as
work, even where the helper definitions are shared. Maintain the ordinary
compiler's scoping and exact `Nat32` arithmetic. The compiler's opt-in changes
source admission. It can still emit the existing bundle format because the
result uses only existing KernelIR with the same memory plan. Record the
source-profile selection in the build evidence.

**Exit.** Compile and run a finite integer transform with 1, several and 32
iterations. Check hand results, C, IR and device at a ragged length. Mutate
the source literal count and one update operand and show that each changes
results. A bound/work-budget violation refuses before bundle publication.
Existing v1 bundles and the Rust reader remain compatible.

### G22a — Define map-and-sum and check its CPU schedule

**Landed 2026-11-19** in [REDUCE_U32.md](REDUCE_U32.md): the operation, pass
schedule, plan identities, the C oracle and the Rust schedule all agree at the
named sizes. G22b (the staged WGSL emitter) is next.

**Files.** Proposed `docs/gpu/REDUCE_U32.md`, profile plan in `lib/gpu/`, and
a matching Rust evaluator/schedule under `safestos/gpu/`.

Propose `--gpu-operation=map-sum-u32` with one `Nat32 → Nat32` mapper and
an explicit modular sum modulo 2³². The result is one word, with identity 0
for empty input. A CPU oracle maps the original scalar function and folds
with wrapping add. The group-schedule oracle independently partitions ranges
into tiles of 64. Each pass takes an input count `n > 0` to `ceil(n/64)` output
elements. Always run pass 0 for nonempty input, including a singleton. Pass 0
uses the Austral mapper. Later passes read partial sums until one remains.
At the v1 maximum of 1,048,576 inputs, at most four passes are needed. The host
calculates counts and byte capacities with checked arithmetic. Use distinct
storage for each pass's inputs and outputs. Empty input returns `[0]` without
creating a zero-size GPU binding.

Use a versioned reduction artifact with separate program and launch-plan
identities. Record mapper IR, reduction operator and identity, group size,
resource requirements, backend entries and their digests. The host derives
the dynamic counts from actual validated inputs. Reject v1 map-only consumers
with a profile mismatch. Preserve ordinary v1 bundle bytes and validation.
The partition follows `i = 64 * group + lane` with `0 <= lane < 64`. Quotient
and remainder give a unique assignment for each valid index. A CPU arithmetic
model of this schedule and the four-elements-per-lane variant agreed with a
serial wrapping sum over 12 sizes from empty to the release maximum. This
checks the proposed counting and padding, with device execution and barrier
semantics still to be tested.

**Exit.** A C mapper plus independent serial sum, hand vectors and the Rust
schedule agree bit-for-bit for `0,1,63,64,65,4095,4096,4097` and the maximum
legal length. Include wraparound and a mapper with `f(0) != 0` so that
applying the mapper to padding fails. Wrong counts, extents, identity, roles
and an oversized input refuse. The partition covers each valid index once.

### G22b — Emit and validate the fixed WGSL reduction

**Landed 2026-11-20** in [REDUCE_U32.md](REDUCE_U32.md):
`lib/gpu/Gpu_reduce_wgsl.ml` emits `reduce.json`/`reduce.wgsl` (two entries,
guard + identity, six strides, one barrier per step, single lane-0 write),
`--gpu-operation=map-sum-u32` selects it, and
`safestos/gpu/src/{reduce_artifact,reduce_audit}.rs` load and audit it.
Measured: emitter suite 4 OK, Rust 41 OK (7 audit mutations, each refused by
name), publish + `validate-reduce` agree on the program digest. A general
shader/model correspondence is still not claimed.

**Files.** `Gpu_wgsl.ml` or one adjacent reduction emitter, `Gpu_bundle.ml`,
Rust `schema.rs`/`artifact.rs`/`validate.rs`/`wrapper.rs`, and focused tests.

Generate a mapper pass and a partial-sum pass. Each invocation in a 64-lane
group computes a guarded input value or the identity, stores it in its own
workgroup slot, then reaches a barrier. For strides `32,16,8,4,2,1`, lanes
below the stride combine two disjoint slots, and every lane reaches the
barrier after the step. Lane 0 writes that group's unique partial. Guard every
input load. Tail lanes contribute identity before the first barrier and never
skip a collective. Keep workgroup scratch to 64 `u32` words. Use the actual
Naga validation and add a separate structural audit for this profile. Check
scratch roles, indices, guards, all barriers and the single group write. Keep
the v1 wrapper audit unchanged.

**Exit.** Compile source to staged WGSL/metadata, validate both entries and
check byte determinism. In scratch fixtures that recompute artifact digests,
refuse a removed load guard, skipped/divergent barrier, wrong stride, group
write to a nonunique slot and a wrong binding role. Report shader validation
and structural audit separately from semantic preservation.

### G22c — Dispatch, read back and measure the reduction

**Landed 2026-11-20** in [REDUCE_U32.md](REDUCE_U32.md):
`safestos/gpu/src/reduce_run.rs` preflights the derived schedule against
adapter limits, dispatches one ordered pass per level with the previous
pass's output bound as the next input, and reads the final word once
(`avm-gpu run-reduce`, `reference-reduce`). Measured on an NVIDIA GTX 1060
(Vulkan, physical): all nine G22a sizes equal the C oracle bit for bit,
including `[16384,256,4,1]` at 1 048 576.

The four-elements-per-lane variant landed as `map-sum-u32-x4`
(`reduce-u32-x4-v1`, `i = group*256 + j*64 + lane`), not as a replacement:
1 048 576 needs three passes instead of four, and 12 sizes (including
255/256/257) agree with the reference on hardware. `avm-gpu compare-reduce`
now compares either reduction with map + full readback + CPU fold and reports
bytes moved and warm timings; at N = 1 048 576 the x4 reduction moved
4 210 760 bytes versus 12 582 912 for map+fold and read back 4 bytes versus
4 MiB. `scripts/gpu-reduce-hardware` is the physical lane, run by
`scripts/check-gpu required` and grepped by the GPU workflow; the source lane
validates both artifacts with the CPU checker.

**Files.** `safestos/gpu/src/run.rs` or an adjacent reduction runner, CLI run
and reference tests, and isolated hardware cases in the GPU scripts.

Reuse preflight, adapter selection, error scopes and readback. Run one ordered
compute pass per reduction level on a queue, retaining intermediate buffers on
the device and using separate input/output bindings for each pass. Request
preflight limits for per-dispatch group count, workgroup size, scratch bytes,
binding capacity and total allocations including staged output. Read back the
final word once. Observe completion or report failure through the runner.

**Exit.** A named physical adapter agrees with the G22a oracles. Instrument
exact pass counts, guarded tails, bytes moved, and cold and warm timings.
Compare with map → full readback → CPU fold, and with equal-schedule Rust/WGSL
where available. A later single tuning patch can try four elements per lane at
`i = group * 256 + lane + 64 * j`, `0 <= j < 4`. Run coverage and tail checks
again, then keep that variant only if measurements justify it. General
shared-memory kernels, user barriers and floating-point reductions remain
outside this fixed operation.

## MLIR as a second consumer of KernelIR

### G19a — Pin and record one toolchain round trip

**Files.** Optional MLIR devshell, `scripts/gpu/mlir-probe` and a handwritten
smoke fixture with two `memref<65xi32>` parameters and a guarded copy kernel.

Use the lock's compatible LLVM/MLIR tooling where available. Capture versions
and pass availability, then pin a working `gpu`/`arith`/memory-to-NVVM
pipeline and PTX or binary serialization for one named NVIDIA target. Choose
the supported static bare-pointer convention if the emitted ABI confirms it.
Capture lowered signatures and the device entry declaration either way. Keep
the smoke fixture explicitly separate from source compilation. Refuse missing
tools or unsupported target options with the failed stage named.

**Exit.** Two runs reproduce the recorded artifact or an explained lowering
record if the tool emits environment-dependent metadata. Record commands,
target, address-space conversion, data layout, output file and parameter
layout. No changes to the OCaml-only or default GPU lane.

### G20a — Generate verified MLIR for existing scalar semantics

**Files.** Proposed `lib/gpu/Gpu_mlir.ml` and focused emitter tests.

Start with `Pointwise:difference` and `Pointwise:clamp`. Lower typed values,
bindings and branches from KernelIR. Use signless `i32` with explicit unsigned
comparisons for `Nat32`, modular arithmetic without no-wrap flags, and checked
conversion for launch IDs and indices. Use the v1 memory plan with an explicit
static extent of 65 and a dominating tail guard. Use the existing G19a
interface/ABI. Widen successively to the current v1 helper and operation corpus.

**Exit.** Pinned MLIR verification and lowering accept the real source-derived
fixtures. Swapping subtraction operands and reversing a comparison produce
different lowered bodies. Unknown IR cases produce a named refusal.

### G20b — Publish a separate versioned MLIR artifact

**Files.** `Compiler.ml`, GPU CLI and staged bundle publication, a schema and
toolchain checks for an MLIR target. The proposed user route is
`--target-type=mlir --gpu-static-elements=65` with the existing entry selector.

Stage canonical KernelIR, source provenance, emitted MLIR, lowered bytes,
target/pipeline identity and checked ABI metadata. Bind each derivative by
digest to its predecessor. Record which tool produces each artifact and what
validation ran. Reject a missing pipeline, mismatched ABI or unsupported
profile before publication. Keep v1 WGSL bundles and fallback policy intact.

**Exit.** Repeatably compile the same Austral fixture through WGSL and MLIR
and reproduce its KernelIR identity. Refuse a tampered lower artifact,
conflicting target options and an invalid static extent. The default CPU tools
still operate without MLIR installed.

### G21a — Launch an actual source-generated NVIDIA artifact

**Files.** Optional CUDA runner in the isolated GPU crate, device tests and
one benchmark input. Reuse the recorded ABI before considering dynamic memrefs.

Load the actual emitted PTX or binary. Check parameter order, size, pointer
roles and exact allocation extent. Launch identity, clamp, asymmetric
subtraction and wrapping arithmetic at length 65 and compare against C, IR
and WGSL on the same device where available. Record selected hardware and
driver, completion and readback. Test a wrong extent/arity and a mismatched
ABI before calling the driver. Then expand to the v1 corpus. PTX is a virtual
ISA. PC09/PC10 track MLIR pass preservation and native instructions separately.
[NVIDIA PTX specification](https://docs.nvidia.com/cuda/parallel-thread-execution/).

**Exit.** Genuine hardware results and ABI record, or lower-artifact and
preflight evidence with hardware execution still open. Compare only matching
workload and transfer paths. Keep the new toolchain optional.

## Proof reuse for the first scalar shader

### PC05a — Reflect actual parsed WGSL for one clamp

**Landed 2026-11-22.** `safestos/gpu/src/wgsl_reflect.rs` reflects the parsed
Naga module of the bundle `austral compile --target-type=wgsl` publishes for
`examples/gpu/pointwise` — not a retyped model: the compute entry, the one
scalar helper it calls, that helper's parameter name/position/width, the
comparison direction, the literal, and which operand each branch returns,
plus the storage roles and the identity output index. The wrapper's role,
guard and freshness assumptions stay a separate audit (`wrapper.rs`), reported
as its own field. The reflection must agree with the bundle's KernelIR, and
the emitted `mm0/proof_chain/clamp_wgsl.mm0` records both subjects' digests and
defines `wgsl_clamp` with `wgsl_clamp_matches_ir` / `wgsl_inherits_source_bound`.
Seven Rust tests cover the accepted shader (its `REAL_WGSL` digest is the
bundle's own `wgsl_sha256`), a reversed comparator, a changed literal or
operand, a changed storage role or width, unsupported parsed nodes, and an
artifact whose recomputed digest does not rescue a wrong correspondence.
`scripts/pc05-reflect` re-emits from a fresh bundle (byte identity), runs
`check-reflect`, and applies the three parsed-shader mutations with recomputed
hashes, requiring a refusal for each. The MM0 replay runs green under the
flake's `mm0` package — `nix build "git+file://$PWD#mm0"`, put its `bin/` on
`PATH`, and the script reports `word+clamp_source+clamp_wgsl assembled and
accepted` plus `a reflected ujoin does not check the umeet proof` — and remains
a `[gap]` on a host without `mm0-rs`/`mm0-c`, never a pass. See
[PARSED_WGSL.md](../proof/PARSED_WGSL.md).

**Files.** Small adapter near `validate.rs` and proof fixtures. Read the actual
parsed Naga entry and scalar helper. Translate the admitted `u32` parameter,
constant, ordered comparison and case/return into the existing word model.
Match IDs, widths, comparison direction and observable result. Check the
wrapper's role/guard/freshness assumptions separately. Refuse unsupported
parsed nodes. Bind the emitted witness to the real WGSL and IR subjects.

**Exit.** An accepted real clamp and refusals after independently mutating
parsed shader operands, comparator or storage role with hashes recomputed.
Record parser-to-model reflection as trusted until a checked decoder exists.

### PC05b — Transport the clamp theorem through the reflected edge

**Landed 2026-11-22.** `mm0/proof_chain/clamp_transport.mm0` (generated by
`scripts/pc05b-transport --write` from the committed `clamp_wgsl.mm0`) carries
the shader bound to the device's value in two applications of PC03's
`transfer_through_eq`: the first over PC05a's `wgsl_inherits_source_bound`, the
second over `native_agrees`. The earlier theorem is in the proof term, so
removing its proof refuses the transport (control `drop-import`). The four
residual premises are a register in the header — `wrapper`, `extents`,
`bindings`, `native` — and `native` is the one the proof consumes; the other
three are where PC06's bundle claim will consume them. A strict profile refuses
an artifact that shrinks the register (missing/re-labelled row, premise dropped
from the statement, a premise "proved" by a theorem, a third theorem, an
`axiom`), and pins `-- subject:` and `-- bound:` so a changed proof subject or
literal is refused structurally rather than by luck. The replay checks with
`mm0-rs`/`mm0-c` under the flake's `mm0` package; without them it is a `[gap]`.
A changed shader moves `edge_sha256`/`wgsl_sha256` and leaves the theorem
statements byte-identical. See [PARSED_WGSL.md](../proof/PARSED_WGSL.md).

**Files.** A narrow MM0 emitter/replay fixture and statement/dependency test.
Use the existing PC04 scalar bound and PC03 transfer lemmas. Apply them to
the reflected scalar edge. Record distinct outstanding premises for the
wrapper, runtime extents, bindings and native device execution. Where a Why3
arithmetic goal has a corresponding MM0 premise, require an actual MM0
discharge to turn that premise green. Continue with PC06 and PC07 for the
composed bundle gate. Owner approval uses the existing statement workflow.

**Exit.** Actual `mm0-rs`/`mm0-c` replay with an earlier theorem application
visible in its dependencies. A changed lower shader requires new edge evidence
and leaves the high-level theorem stable. Removing a required premise or
changing a proof subject refuses the strict profile. The reduction follows
later by proving the modular sum law once and adding schedule-specific
coverage, disjointness, barrier and memory relations.

## Numeric and host follow-ups

### G16a — Establish Float32 source operations with a CPU oracle

**Files.** `lib/builtin/Pervasive.{aui,aum}` if required by the operation
family, `Gpu_intrinsics.ml` for the resolved IDs, and a small C-target fixture.
Start with add/subtract/multiply and explicit `Float32` literals for an affine
map. Resolve canonical operation instances by identity. Preserve Float64 and
named foreign-call behavior. Confirm `f32` values reach the GPU subset pass as
Float32 and reproduce typed literals without widening.

**Exit.** C executes the real affine source fixture. The fixture distinguishes
f32 from f64 rounding. A user-defined operation of the same name remains
outside the intrinsic list. The existing v1 profile still denies Float32.

### G16b — Add the opt-in Float32 GPU profile

**Files.** One profile contract, `Gpu_ir.ml`, `Gpu_wgsl.ml`, Rust schema and
reference, and device fixtures. Add real f32 IR/storage and a CPU reference
for the admitted arithmetic. State input/intermediate range and rounding,
contraction, subnormal, signed-zero and overflow behavior. Enforce the finite
domain where validation depends on it. Record any caller premises in the
profile. Use the specification's numeric rules to justify the comparison.
[WGSL floating-point rules](https://www.w3.org/TR/2026/CRD-WGSL-20261106/#floating-point-evaluation).

**Exit.** Test the affine fixture on device with an operation-specific
reference and include cancellation-sensitive, boundary and exceptional inputs.
Refuse Float64, wrong element representation and unsupported operations. Use
the original v1 suite to guard against numeric reinterpretation.

### G18 — Integrate one unfer scale operation

Select one scale operation already used in `TensorState`. Read the destination
instructions before editing and add the opt-in example and feature listed in
the [parent card](../../PLAN_gpu_kernels.md#g18--exercise-one-existing-unfer-workload-sync).
Use a named CPU oracle and the same unfer values for GPU comparison. Time
explicit copies and cold/warm execution separately. Unfer's default Cargo build
and solver stay on their existing paths during this experiment.

### G15a — Close host-state hazards before widening dispatch

**Landed 2026-11-22.** `service.rs` publishes a completion and its words in one
critical section under the job lock, claims a receipt inside that same lock
(too-short capacity still retryable), and serializes close against admission
with a per-context `closed` transition taken before the handle leaves the
registry — spawning and registering the worker under that same lock. The lock
order is documented in the module header and in [G15_DESIGN.md](G15_DESIGN.md).
Four tests force the interleavings through test-only hooks
(`service_test_set_hook`); against the unfixed ordering they fail with stale
bytes, two claims of one receipt and the wrong refusal code respectively. No
ABI or kernel change. The device path was re-measured: two physical contexts
and three child-process teardown cycles on the Intel UHD 630. TSan/loom and a
machine-checked lock order remain gaps.

**Files.** `service.rs`, its lifecycle tests, and `run.rs` only where completion
classification needs it. Keep the kernel and the ABI stable for this card.

First reproduce one race with a deterministic handoff hook in a test. Static
inspection shows `publish` sets the terminal job state before copying its bytes,
and `begin_job` takes a context reference before `rs_gpu_close` removes that
context. `rs_gpu_result` also reads `result_taken` and later sets it after
copying. Check these interleavings against buffer ownership and worker
registration. Publish success only once the relevant bytes and ownership state
are ready. Make claiming a receipt atomic with its lifecycle, with too-short
capacity still retryable. Serialize close against admission using one coordinated
state transition and a documented lock order. Distinguish a definitive device
failure from an inconclusive wait timeout. Keep allocations quarantined until
the device contract permits release. A caller's wait deadline is independent of
device cancellation. [Service code](../../safestos/gpu/src/service.rs),
[wgpu 29.0.4 completion API](https://docs.rs/wgpu/29.0.4/wgpu/struct.Queue.html#method.on_submitted_work_done).

**Exit.** A controlled result/read interleaving cannot observe stale bytes.
Two simultaneous receipt attempts give one success and one refusal.
Submit versus close either leaves a live context with a matched job or refuses
without a new worker. Reversed/duplicate callbacks, wrong result size, timeout
and late completion leave consistent ownership. Use synchronization primitives
in the test to force order. Run the existing compile-time linearity and mock
service suites and retain the physical callback test.

### G23a — Bound placement and progress with mock devices

**Landed 2026-11-22.** `safestos/gpu/src/coordinator.rs` (feature `service`)
owns one reusable worker per device and a per-device `Budget` (in-flight jobs
and reserved bytes). Admission reserves capacity, the worker's terminal
`Outcome` returns it, and a device whose worker is gone admits nothing until
`teardown` joins it and releases the reservation. A context is bound to one
device and a generation; `poll`/`result` refuse another context's job
(`UNKNOWN_HANDLE`) and a stale generation (`STALE`). `Identity::is_physical`
keeps a mock or software device out of a hardware claim. Seven mock tests in
`safestos/gpu/tests/coordinator.rs` cover the card's exit cases, and seven
mutation controls (byte budget, queue budget, context and generation binding,
capacity release, one shared worker, both lost-worker admission paths) each
fail the test that claims them; the unmutated source is sha256-restored. G23b
replaces the mock identity and queue with selected adapters; nothing here is a
device record.

**Files.** A local coordinator beside `service.rs`, the existing adapter
selection code, and a mock lifecycle fixture. Hold explicit per-device queue
and byte limits, reserve capacity before admitting work, and return capacity
on terminal outcomes. Use one reusable worker per device, with caller-supplied
immutable inputs. Bind buffer and receipt identities to their context and
generation. Keep driver waits and teardown off the coordinator.

**Exit.** The second worker completes while the first is held by the test.
Refuse an over-budget submission, a cross-context receipt and a stale
generation. A lost worker's reserved budget is accounted for until teardown.

### G23b — Check two physical adapters in one process

**Landed 2026-11-22.** `safestos/gpu/tests/dual_device.rs` (features
`device` + `service`) enumerates adapters, collapses views of one card to one
selection (PCI vendor/device when the backend reports it, the name otherwise —
the two views of this host's NVIDIA card spell their names differently), and
requires two distinct physical cards, else a `[gap]` with
`AVM_GPU_ALLOW_ONE_DEVICE=1` or a failure. It then places one kernel per card
through the G23a coordinator — so the hardware lane uses the same
one-worker-per-device admission path the mock tests pin — submits both before
awaiting either, checks each device against the KernelIR reference on its own
tagged inputs, and records per-device prepare/upload/submit/readback timings.
Overlap is host-side correlation made comparable: the two submit→terminal
windows must intersect, and the concurrent wall clock is reported against a
serial phase of the same work (598 ms vs 1185 ms here, 1.98x). Device loss is
not triggerable on either adapter and is recorded as this card's open gap; the
mock slow/lost cases stay G23a's. Three controls (serialized phase, swapped
expected vectors, no card collapse) each fail the assertion that owns the
property.

Select two distinct adapters and record how identity is established, including
duplicate backend views of one card. Submit independent generated pointwise
kernels with different tagged inputs. Check both results and phase timings.
Use the mock slow/lost device cases to establish coordinator behavior and
exercise device loss on hardware where supported. Record host-side correlation
of submit/completion and use device timing to substantiate hardware overlap.
With one adapter, record the gap for two-device execution. Remote workers and
NPU support are separately scoped in the
[distributed plan](../../PLAN_distributed_security.md) and
[proof plan](../../PLAN_proof_chain.md).

## Copyable assignment

> Read `AGENTS.md`, `PLAN_gpu_kernels.md`, `docs/gpu/NEXT_STEPS.md` and the
> execution record. Start with G17a. Read its named sources and check git
> status. Implement only that card and its acceptance cases. Keep C/CPS and
> current v1 artifacts compatible. Run the targeted fixture and the existing
> source gate in the documented environments. Record exact evidence and any
> unverified boundary. Then take the next eligible card. When a hardware or
> toolchain prerequisite is absent, finish the applicable CPU work, record
> the gap, and take an independent eligible card. Keep proof approvals in the
> existing owner workflow.
