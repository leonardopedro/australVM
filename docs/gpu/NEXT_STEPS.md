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

**Status:** met on this host, from nixpkgs rather than a new devshell:
`llvmPackages_19.mlir` (19.1.7) and `llvmPackages_19.llvm` (19.1.7).
`scripts/gpu/mlir-probe record` runs three stages on two subjects — the
handwritten `mm0/proof_chain/mlir/smoke.mlir` (kept separate from source
compilation) and the module G20a emits for `clamp` — and writes
`mm0/proof_chain/mlir/toolchain.toml`: the three tool versions, the target
(`sm_70`), the exact pipeline string, and per subject the source digest, the PTX
file and digest, the entry name, the 10-parameter layout (one `memref<65xi32>`
→ 5 × `.u64`: alloc ptr, aligned ptr, offset, size, stride) and the
address-space conversion (`generic → global` via `cvta.to.global`).
`--gpu-lower-to-nvvm-pipeline=cubin-format=isa cubin-chip=sm_70` stops at PTX
text, so no `ptxas` or CUDA toolkit is needed. `check` re-runs every stage and
refuses a changed version, target, pipeline or PTX digest (four controls); a
host without MLIR reports a `[gap]`. The default GPU lane is untouched: nothing
in `Compiler` calls the probe, and `make all` never needs MLIR.

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

**Status:** the emitter exists (`lib/gpu/Gpu_mlir.ml`, `gpu_proof dump-mlir`,
`test/GpuMlirTest.ml`): signless `i32` with unsigned predicates, no wrap flags,
an explicit static extent, a dominating `ult` tail guard, and named refusals for
helper calls, non-v1 profiles, a non-positive extent and one-armed returns.
Eleven tests cover the encoding, the refusals, the structural validator's own
negative controls, and the mutation requirement (swapping `modularSubtract`
operands or reversing `clamp`'s comparison changes the body).
**Pinned MLIR verification and lowering happen in the G19a lane, not here:**
the module is captured byte-for-byte (`mm0/proof_chain/mlir/ir/clamp.mlir`,
recorded in `stage.toml` as `kernelir-to-mlir = captured`), and
`scripts/gpu/mlir-probe` verifies that exact file with LLVM 19.1.7 and lowers it
to `sm_70` PTX (`mm0/proof_chain/mlir/toolchain.toml`). What is not claimed is
preservation: no MLIR semantics exists, so the ABI header still says
`g19a-pending`, and the interface is exercised rather than proved. Widening to
the helper corpus is also unbuilt: a kernel with a helper is a named refusal,
not a silent inline.

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

**Status:** built on this host. `--target-type=mlir --entrypoint=… --output=DIR
[--gpu-static-elements=N]` builds the same checked KernelIR as the WGSL lane,
renders it (G20a), has `mlir-opt` verify it, lowers it with the pinned pipeline
to `sm_70` PTX, and publishes four chained files: `kernel.kernelir`,
`kernel.mlir`, `kernel.ptx`, `bundle.json`. The metadata records source and
builtin provenance digest, the three derivative digests with their predecessors,
the toolchain string (`LLVM version 19.1.7`), the target, the exact pipeline, the
parameter layout and the numeric contract. Publication is atomic (a staging
directory renamed only after the set checks itself) and refuses a pre-existing
destination.

`scripts/mlir-bundle` compiles the same fixture through WGSL and MLIR and
requires their KernelIR identities and bytes to agree, verifies each published
set, and fires every refusal the card names: a tampered lower artifact, a
conflicting numeric profile, a WGSL-only staged reduction, a zero/negative
static extent, a missing `mlir-opt`, and a nonexistent tool override. It also
compiles a CPU target with the toolchain absent, so the default tools are shown
not to depend on MLIR.
Without `mlir-opt` the whole check is a `[gap]`; the CPU and WGSL targets never
consult it.

**Status:** implemented. `austral compile SRC --target-type=mlir
--entrypoint=M:f --gpu-static-elements=65 --output=DIR` publishes four files —
`kernel.kernelir` (canonical bytes), `kernel.mlir` (`Gpu_mlir.render` of exactly
those bytes), `kernel.ptx` (the pinned G19a pipeline's lowering) and
`bundle.json`, whose `derivatives` array chains the three digests with their
predecessor and producer. Publication is staged: `mlir-opt` verifies the
module, lowers it, the PTX ABI is checked against the kernel's arity (five
`.u64` per memref), the set is re-read by `Gpu_mlir_bundle.verify_dir` itself,
and only then is the directory renamed into place. A failed step publishes
nothing. Refusals are named before work: a non-`u32-modular-v1` profile, a
`--gpu-operation` (WGSL-only), a non-positive or non-integer
`--gpu-static-elements`, an existing output directory, and a missing
`mlir-opt` (with `AUSTRAL_MLIR_OPT` accepted only when that path exists). The
verifier additionally requires `kernel.mlir`'s own `// kernelir-sha256:`
header to name the recorded IR, so re-hashing a forged `.kernelir` does not
repair the pair, and it re-derives the ABI from the PTX bytes rather than
trusting `bundle.json`. `test/MlirBundleTest.ml` (13 tests) pins the extraction,
the ABI expectations, the chain's tamper refusals and the CLI shape;
`scripts/mlir-bundle` runs the end to end: two MLIR publications byte-identical,
the WGSL bundle of the same fixture agreeing on `48caf04e…`, the published set
re-verified, and six controls refusing — including a CPU `--target-type=c`
fixture still compiling with no MLIR on `PATH`. Not claimed: the ABI header
remains `g19a-pending` (exercised, not proved), the bundle records host-absolute
source paths, and no MLIR semantics or preservation proof exists (PC09).

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

**Status:** genuine hardware results on this host (GTX 1060 Max-Q, capability
6.1, driver 580.119.02). `--target-type=mlir --gpu-chip=sm_61` publishes an
artifact whose `bundle.json` names the chip, and `scripts/gpu/launch check`
publishes it from `examples/gpu/pointwise`, publishes the WGSL bundle of the
same source, and the device side (`safestos/gpu/src/nvlaunch.rs`, an optional
`--features nvidia` binary that dlopens the CUDA Driver API rather than
linking it) preflights before launching: PTX `.target` against the device's
capability (PTX is not backward-compatible), the `.param .u64` count against
`input_arity` at five words per memref, extent and arity against the record,
and a block size the device accepts. `avm-nvlaunch launch` uploads the
buffers, launches blocks of the module's declared size (64, so two blocks at
extent 65), synchronises and reads back;
`scripts/gpu/launch` publishes the artifacts for the device's own chip and
compares that readback with `avm-gpu reference` on the WGSL bundle — the
KernelIR evaluator, not a retyped expectation. The launch exposed a real
emitter bug: `gpu.thread_id` is block-local, so at extent 65 a second block
rewrote 0–63 and index 64 was never written; the element index is now
`block_id * block_dim + thread_id` (re-captured and pinned by PC09/G19a).
The same run expands to the v1 corpus the card asks for: every `Pointwise`
entry that has a direct scalar body — 12 of the 14, unary and two-operand, so
the one- and two-memref ABI shapes are both measured — is published for the
device's chip, launched at extent 65, once again at 1025 (17 blocks) and once
with `--block 32` (an uneven 3-block grid), and compared with `avm-gpu
reference`. Fourteen launches, `[detail] … geometries measured: 2x64, 3x32,
17x64`. Each launch is recorded in `mm0/proof_chain/nvidia/launch.toml`
(device, geometry, ABI words, PTX/KernelIR/output digests, changed elements)
and `check` refuses a run that differs from that record, so a changed emitter,
ABI or chip is a finding rather than a silent refresh. Five refusals are
required: a wrong extent, a wrong arity, a block size no device accepts, a PTX
whose `.target` is newer than the device (the refused module gets the driver's
own verdict, `CUDA_ERROR_INVALID_PTX`, so the refusal is measured rather than
assumed), and the two helper-calling entries `addConstant`/`polynomial`,
which MLIR refuses by name (`helper call is not lowered yet`) before any
bundle is published. Every launch also agrees with the C backend's own
`expected` column in `examples/gpu/pointwise/corpus.tsv`, repeated to the
launch extent — a second oracle independent of the KernelIR evaluator; a table
cell altered by one fails at that index.
Not claimed: this is one device; the PTX
is a virtual ISA, so nothing here is a preservation proof or an instruction-
level certificate (PC09/PC10), the ABI header stays `g19a-pending`, and
hardware results are not reproduced in CI (the gate reports a `[gap]` where no
device is visible).

### PC10b — Relate the NVVM/PTX output to the instructions the device ran

**Depends on** PC10 as landed (`safestos/gpu/src/target.rs`,
`avm-target capture`, `scripts/gpu/boundary`, `mm0/proof_chain/nvidia/native.toml`)
and on G21a's launch record. **Files:** a SASS listing/attribution artifact, a
decoder input pinned like the MLIR toolchain, and its own gate in
`scripts/gpu/boundary`.

**Landed 2026-11-23 (the listing and the localised control, not the decoder).**
`safestos/gpu/src/target.rs` gains `listing`/`word_diff` and `avm-target` gains
`words` and `diff`: every `.text.*` section is split into its 16-byte encoding
words (the width every target this repo links for uses), a section that is not a
whole number of words is refused by name, and the listing's digest is a function
of the words and their offsets. `scripts/gpu/boundary` records, per capture,
`sass_words`, `word_bytes` and `listing_sha256`, and requires
`words × word_bytes == sass_bytes` — so the count is measured against the
section table rather than taken from it. Measured on the GTX 1060: each cubin's
`.text.main` is 24 words covering all 384 bytes, and both mutations are now
localised: the operand-order edit moves **exactly one** word, `.text.main@272`
(index 17), and the extent-66 memory bound moves exactly one word,
`.text.main@144` — different words, both stored in the record and both compared
by `check`, so a tampered offset or listing digest fails. What this does
not do is name the opcode or claim the PTX line wrote that word: it localises the
*mutation*, not the instruction. `cudaPackages.cuda_nvdisasm` is unfree here, so
step 1 below is still open, and steps 3's attribution to named PTX instructions
waits for it.

PC10 stops at identification: the record says these bytes are an
ELF64/`EM_CUDA` container with 384 bytes of `.text` and a digest, and refuses
an operand-order edit because it links to *different bytes* — not because
anything knows what those bytes compute. This card is the next step PC10's own
text asks for, and only this far: turn the recorded `.text` into a **listing**
whose coverage is checked, and attribute listing entries to the PTX the driver
consumed, so "the device ran these instructions" is a claim about decoded
bytes rather than a size.

1. **A decoder, pinned.** `nvdisasm`/`cuobjdump` are not on this host (no CUDA
toolkit), so either a flake input pins a toolkit version and records it beside
`mlir-opt` in the toolchain record, or an SM 6.1 decoder for the fragment's
opcode subset is written in-repo with its own tests. The choice and its version
belong in the record: a listing from an unrecorded disassembler is not evidence.
2. **Coverage, not prose.** For each captured cubin, the listing must account
for every byte of every `.text.*` section: instruction offsets contiguous from
the section start, the last instruction's end equal to the section size, and a
digest of the listing itself. A listing that skips bytes is refused, because a
decoder that silently drops an instruction is the failure mode here.
3. **Attribution.** For the small fragment `difference` already links
(`sub.s32 %r6, %r4, %r5; st.global` at extent 65), record which listing entry
came from which PTX instruction, and extend the existing mutations: the
operand-order edit must change the *attributed* SASS entry's operands (not just
the blob digest), and the extent-66 capture must show its memory bound at the
attributed load/store. An attribution that survives the mutation control is
what makes it a relation rather than a comment.
4. **A named semantics gap, not a semantics proof.** No public SASS model exists
the way `riscv-lean`/Sail models RV64D, and PC10's text forbids using
disassembly text as a semantics proof. So the card's exit is a decode-fidelity
record: *this listing is the recorded bytes, and this entry is the instruction
that this PTX line produced*, with the PTX→SASS semantic relation left a named
`[gap]` and pointed at whatever ISA model a later card builds.

**Exit.** A pinned decoder and its version recorded; a listing per captured
cubin whose coverage is checked *(landed: word-level, digest-pinned)*; an
attribution table for `difference` that the operand-order and memory-bound
mutations move *(landed: one word each, `.text.main@272` for the operand edit
and `.text.main@144` for the extent-66 bound)*; and a `[gap]` naming the missing
PTX→SASS semantics. Without a decoder the card is a `[gap]` everywhere, never a
pass — the same convention as G21a's hardware line.

### PC11 — Define the first NPU proof branch and its opaque cut

**Depends on** PC01/PC02 (word and map semantics) and PC07 (the whole-chain
fixture). **Files:** `docs/proof/NPU_PROFILE.md`, `mm0/proof_chain/npu/`
(target and operator/command inventories), `mm0/proof_chain/npu_operator.mm0`,
`examples/npu/`, and one checker (`scripts/npu-profile`) — an isolated branch,
not a generic NPU backend.

A GPU branch has a virtual ISA to fall back on; an NPU branch does not. The
whole point of this card is to pick a target where *that* is visible and priced,
rather than to claim inference results as proof.

**Target, named once.** AMD XDNA (Ryzen AI): AIE-ML compute tiles, ERT runtime,
`amdxdna` kernel driver. It is chosen because its stack splits cleanly into what
is inspectable and what is not: the compilers are open (IRON on `mlir-aie` for
the array, Peano on `llvm-aie` for one tile), the application binaries are
documented (an overlay plus a ctrlcode of `XAie_TxnOpcode`, including
`XAIE_IO_BLOCKWRITE` for DMA), and the compute tiles are VLIW processors whose
instruction semantics exist only as an architecture manual (AM020) with no
machine-checkable model. Nothing on this host is an NPU (`host_measured =
false`): no `/dev/accel`, no `xrt-smi`, no Peano, no IRON, so every `declared`
row below is a `[gap]`, never evidence.

**One operator.** A signed 8-bit quantised dot product, `A[4x4 i8] x B[4x4 i8]
-> C[4x4 s32]`, mapped from one admitted Austral step (`dot4`, eight `Int32`
parameters, four products and three adds read out of the `.aui`/`.aum` text, not
retyped). The numeric profile is `int8_sym_exact_s32`: zero point 0, unit scales,
exact widening, no requantization — so the arithmetic claim is “the i8 products
sign-extend exactly and accumulate in s32”, which is a property a reviewer can
state, not an approximation excuse.

**The inventory, as four inspectable stages and two declared plus two opaque:**

| stage | state | subject |
|---|---|---|
| operator | inspectable | the admitted step and the tensor descriptor it maps to |
| quantization | inspectable | `int8_sym_exact_s32`: zero point, scales, widening, no requantization |
| layout | inspectable | row-major operands and output, per-element and accumulator widths |
| dma-command | inspectable | the declared command list (`XAIE_IO_BLOCKWRITE`, host→L2 and L2→host) |
| compiler-ir | declared | IRON/Peano IR: open compilers, no artifact on this host |
| overlay | declared | the xclbin identity: a format, not a semantics |
| compute-tile ISA | **opaque** | AIE-ML VLIW instructions, AM020 only |
| firmware | **opaque** | vendor-signed firmware and ERT scheduling |

**Landed 2026-11-23.** `scripts/npu-profile check` now makes the inventory a
claim instead of data: it re-derives every byte count from dtype and shape,
every offset from the declared layout, the sixteen stores' partition of `C`,
the cap from `lib/npu_dma_gate.ml`, and the accumulation bound over the i8
range; it reads `dot4`'s arity, types and product count out of the
`.aui`/`.aum` and compiles the module with `--target-type=c`; it holds each of
the nine stages to exactly one stated obligation in
`mm0/proof_chain/npu_operator.mm0` (replayed by `mm0-rs`/`mm0-c` when present,
a `[gap]` otherwise); and ten mutations of those facts must be refused.
`docs/proof/NPU_PROFILE.md` is the profile document. The cut below is unchanged
by any of that: the checker inspects this repository's description, not the
tile.

**The cut, stated so it cannot be softened.** `operator → binary/firmware` is
unproved and stays unproved: the operator's meaning is a statement about integers
this repository can check, the ctrlcode and overlay are formats it can hash, and
between them sits a vendor compiler and a VLIW ISA with no model. Consequently:

- a DMA certificate, a well-formed command list, or a successful inference run
  **cannot** satisfy native-instruction-required policy;
- the SRAM gate's 256 KiB cap contributes its narrow size property and nothing
  more — it does not discharge the edge;
- the inspectable stages may be checked exhaustively (every byte count re-derived
  from dtype and shape, every offset from the declared layout, the numeric bound
  by exhausting the i8 range, the sixteen stores disjoint and covering `C`), and
  that is a claim about *this repository's description*, not about the hardware.

`mm0/proof_chain/npu_operator.mm0` states one obligation per stage over an
uninterpreted vocabulary, with `-- certificate: [gap]` and no `axiom`/`theorem`,
and `scripts/npu-profile check` is the gate that holds it to the inventory both
ways: every stage has exactly one obligation and no obligation names an
undeclared stage; an `opaque` row must name its prerequisite; an `inspectable`
row must be one of the four subjects above. Until that checker and
`docs/proof/NPU_PROFILE.md` are in the tree *and the gate runs them*, the
inventory is declared data that nothing reads — a file that no gate opens is not
evidence.

**Exit.** An honest target-specific proof inventory, at least one concrete
inspectable claim (the command list and the numeric bound are the candidates),
and a named opaque prerequisite for each unproved edge — with every
machine-derived figure reproducible from the files rather than copied. If the
target cannot expose the required semantics, this branch's native proof stays
blocked on that prerequisite while the higher claims stay reusable elsewhere in
the chain; that outcome is acceptable and must be written down, not worked
around.

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

**Status:** landed. `TrappingArithmetic(Float32)`'s add/subtract/multiply are
resolved in `lib/gpu/Gpu_intrinsics.ml` by the instance declaration and its
method IDs, never by a method's text, and the C backend executes
`test-programs/suites/025-gpu-f32/001-affine-f32` with the golden
`program-stdout.txt` (`python3 test-programs/runner.py 025-gpu-f32` → 1 PASS):
the Float32 rows read `16777216.000000` and `0.000000` where the Float64 rows of
the *same source* read `16777217.000000` and `1.000000`, so a pass that widened
to binary64 would fail the case instead of rounding quietly. Intrinsic identity
is pinned for f32 in both directions by `test/GpuSubsetTest.ml`: a user
definition *named* `trappingAdd` does not capture the call — the frontend
resolves it to the canonical method, the local definition stays unreachable (no
helper is emitted), and the entry lowers to `F32Add`; a user operation under
its own name (`scale`) stays a helper `Call` and never gains intrinsic status.
`test_refuses_trapping_arithmetic` pins the last exit clause: the default
`u32-modular-v1` profile still refuses `trappingAdd` on `Nat32`, and the
f32 profile refuses Float64, division and non-Float32 results (G16b's
checker). Not claimed: the C fixture is the oracle for *source* arithmetic
only; nothing here relates f32 to a device.

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

**Status:** the CPU/profile half is landed and gated; the device half is a
`[gap]` on this host. `scripts/gpu-f32-profile` compiles `axpy` and `cancel` of
`examples/gpu/f32-affine` under `--gpu-numeric-profile=wgsl-f32-v1`, validates
each bundle (`profile=wgsl-f32-v1`, `element=f32`, `ir=e7b526eeb585` /
`35dbf7e74c38`), compares the KernelIR reference bit for bit against an
independent Python binary32 oracle that rounds after every operation (so a
widening to f64 cannot pass: at 2^24 the two disagree) on 4 `axpy` + 3 `cancel`
bounded normal inputs including the cancellation-sensitive `2^24 → 0`, and
requires six refusals to name what they refused: Float64, `trappingDivide`,
a Nat32 result under the profile, Float32 under `u32-modular-v1`, an unknown
profile, and a scratch bundle whose `numeric_profile` disagrees with its
KernelIR. Measured now: ten `[pass]` lines and one `[gap]` (device), exit 0;
`scripts/verify-invariants` prints `[pass] G16b: the wgsl-f32-v1 profile is
checked end to end`. `--require-hardware` exits 77 with this checkout's
`avm-gpu` because that binary was built without `--features device`; the Intel
UHD Graphics 630 run recorded in `EXECUTION.md` (max 1 ULP of the oracle,
budget 2) came from a device-enabled build and was not re-measured here.
Not claimed: no per-bundle MM0/Why3 obligation for f32; contraction is
permitted rather than eliminated (the profile records it and the ULP budget
covers it); float comparisons, division, float→int conversion and Float64 stay
refused.

### G18 — Integrate one unfer scale operation

Select one scale operation already used in `TensorState`. Read the destination
instructions before editing and add the opt-in example and feature listed in
the [parent card](../../PLAN_gpu_kernels.md#g18--exercise-one-existing-unfer-workload-sync).
Use a named CPU oracle and the same unfer values for GPU comparison. Time
explicit copies and cold/warm execution separately. Unfer's default Cargo build
and solver stay on their existing paths during this experiment.

**Status:** landed on this host as an optional integration; see
[UNFER_SCALE.md](UNFER_SCALE.md). The operation is the amplitude scale behind
`QuantumState::scale_and_add`, the kernel is `examples/gpu/unfer-scale` (one
`trappingMultiply(x, 2.5)` under `wgsl-f32-v1`), and the host is
`fock_sirk/examples/austral_pointwise.rs` behind that crate's optional
`austral-gpu` feature with an optional path dependency on `avm-gpu` (no unfer or
JIT dependency, so no cycle). `python3 scripts/gpu/unfer-scale` compiles the
bundle, builds the feature-off and feature-on configurations, runs the example
on a physical adapter, and requires the GPU result to equal unfer's own f64
arithmetic cast to binary32 bit for bit: `[pass] G18 oracle: 6 value(s) of 3
complex amplitude(s) match … on Intel(R) UHD Graphics 630 (CFL GT2)` (cold
`prepare`/`upload`/`submit`/`readback` and the warm submission total are printed
separately). Two controls refuse: a `u32-modular-v1` bundle (the compiler refuses
Float32 for that profile, and the example refuses a non-f32 bundle) and a
3.0-scaled kernel against the 2.5 oracle (mismatch count 5, so the comparison is
live). Measured: submission dominates (≈ 98 ms over 8 submissions versus
0.0049 ms of f64 oracle for six values), so the integration *hurts* end-to-end
time at this size and no solver replacement is proposed; narrowing the f64
amplitudes to f32 before the multiply changed 1 of 6 rounded results relative to
scaling in f64 and casting once, and that difference is reported rather than
hidden. The transfer is an explicit copy each way (not zero-copy). The default
`cargo build -p fock_sirk` is unchanged, and `scripts/verify-invariants` pins the
`[pass] G18` line, reporting `[gap]` when the sibling `../unfer`, cargo or a
physical adapter is absent.

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
[proof plan](../lean4mm0.md#15-compositional-proof-producing-compilation-in-mm0-pc00pc15).

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
