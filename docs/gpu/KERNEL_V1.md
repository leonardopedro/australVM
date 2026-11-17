# Proposed GPU kernel v1 contract

**Status: proposed, 2026-11-17; not implemented.** This is the implementation
contract for G01–G12 in [PLAN_gpu_kernels.md](../../PLAN_gpu_kernels.md).
[Research and prior work](../GPU_KERNEL_REVIEW.md) explain the backend choice.
Later features must version this contract instead of silently weakening it.

## 1. Source and observable behavior

The source is ordinary `.aui`/`.aum` Austral. Select one function explicitly;
no new parser grammar or pragma is necessary for v1. For example:

```austral
module body Pointwise is
    function add(x: Nat32, y: Nat32): Nat32 is
        return modularAdd(x, y);
    end;
end module body.
```

Supply a matching interface for this example:

```austral
module Pointwise is
    function add(x: Nat32, y: Nat32): Nat32;
end module.
```

G01 must parse/typecheck these as real fixtures with the existing compiler;
do not work around an ordinary frontend error in the GPU emitter.

The proposed command is:

```sh
./austral compile examples/gpu/pointwise/Pointwise.aui,examples/gpu/pointwise/Pointwise.aum \
  --target-type=wgsl --entrypoint=Pointwise:add --output=/tmp/pointwise.avkg
```

`/tmp/pointwise.avkg` is a **bundle directory** containing `kernel.json` and
`kernel.wgsl`, not a shared library or native executable. Reject JIT, hot-swap,
`--emit-cps`, `--no-entrypoint`, package-export/totality and executable-output
options combined with this target. CPU targets keep their existing meaning.
There is no automatic CPU fallback when this command fails.

At launch, each entry parameter is a separate equal-length read-only array.
For one or two parameters and an input length N:

```text
output[i] = entry(input0[i], input1[i])  for 0 <= i < N
```

The unary case has only input0. No scalar/uniform parameters in v1. Constants
can be expressed in the source. The entry returns one `Nat32`; Boolean values
are internal only. Inputs and output are separate allocations; in-place writes,
overlapping output and host-pointer kernel arguments are unsupported.

## 2. Complete admission rules

Typing, normal return checking, compiler plugins and linearity still run.
The GPU pass is an additional **closed whitelist** applied to the whole reachable
function/constant closure, including nested expressions. Unchecked constructs
must be refused even when hidden in a helper, argument or conditional arm.
Unrelated unreachable declarations do not authorize a reachable foreign call.
The module itself may not opt into `Unsafe_Module` for a GPU target.

| Form | v1 decision |
|---|---|
| `Nat32`, `Boolean`, typed literals and immutable locals | Accept |
| Entry signature | One/two `Nat32` inputs, one `Nat32` result |
| Local helpers | Monomorphic, same user module, `Nat32`/Boolean values only; acyclic resolved call graph |
| Constants | Reachable same-module `Nat32`/Boolean constants evaluated/lowered under the same rules |
| Expressions | Variables, supported constants, comparisons, Boolean operations, conditionals, admitted calls |
| Statements | Return, immutable bindings and their compiler-generated one-time initializations, blocks, structured conditionals |
| Builtins | Exactly the resolved standard `Nat32` instances of modular add/subtract/multiply and bitwise and/or/xor/not |
| `trapping*`, divide/remainder, shifts | Reject in v1; do not change their contracts to fit WGSL |
| `while`, `for`, recursive calls, fold | Reject in v1; a later bounded iteration profile is separate |
| Mutation, mutable borrows, pointers, spans, array access, allocation/free | Reject; the generated map wrapper owns all storage accesses |
| Arbitrary `@embed`, foreign calls, function pointers, dynamic dispatch | Reject, including transitive uses |
| Records/unions/generics, 64-bit values, casts and floats | Reject in v1 |
| Atomics, shared memory, barriers, subgroups, device-side launch | Reject in v1 |

Recognize builtins using the canonical compiler-created module/typeclass/
instance/method identity and exact signature. User aliases that resolve to that
identity are fine. A user-defined `modularAdd` or an alternate module claiming
to be `Austral.Pervasive` must not gain intrinsic privileges. Maintain an
explicit intrinsic table with conformance tests against the ordinary C backend.
The standard builtin implementation's internal `@embed` is an audited leaf;
that exception does not admit `@embed` from user code or arbitrary helpers.

The intended scalar semantics are `Nat32` modulo 2^32 arithmetic and ordinary
Boolean comparison/branching. Signedness and width must remain explicit in IR.
Do not use OCaml's native `int` for arithmetic evaluation: use Zarith/range checks
and serialize u32 literals as decimal strings. Do not add host signed-overflow
behavior to the CPU reference evaluator. The `*_trapping*` failure cases must
remain negative tests even after modular arithmetic works.

The validator must enumerate every `Tast` expression and statement constructor:
accept, reject with a named reason, or handle a specific compiler-generated
administrative form. No wildcard success, `IntLit(0)` default or silently erased
node. Diagnostics identify the root and helper path, construct and available
source span; synthesized nodes can point at the containing function.

## 3. Resource bounds and wrapper

Initial limits are explicit release limits, not claims about every device:

- 1 MiB source per supplied file; 100000 KernelIR nodes across the reachable closure.
- 128 reachable helpers; call depth at most 128. Bound traversal before recursing.
- N at most 1048576 elements; at most two inputs and one output, each four bytes/element.
- A fixed local workgroup size of 64, one-dimensional dispatch.
- One outstanding job per context until the async extension has its own capacity contract.
- 32 MiB per input JSON file and 32 MiB of runtime-owned payload/staging buffers per job; driver caches/overheads are reported separately rather than claimed within this bound.

Validate source-size bounds before parsing in this target. Also enforce bundle
metadata ≤16 MiB and WGSL ≤16 MiB before reading/parsing it. Device and session
limits can be lower. Reject oversized artifacts; never truncate them.

Bind group 0 uses input bindings 0..arity-1 and output binding arity. Input
storage is `read`; output is `read_write`. Every buffer element is a standalone
u32 word. Buffer size is exactly 4*N bytes, computed in checked host u64/usize
arithmetic. Runtime checks input arity, equal lengths, resource budgets, binding
size, workgroup dimensions/invocations and dispatch limits **before allocation
and submission**.

The generated entry has a `global_invocation_id` and a guard before any load:

```wgsl
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&input0)) { return; }
    // For an arity-two entry; arity-one omits input1.
    output0[i] = scalar_entry(input0[i], input1[i]);
}
```

Compute groups as `N / 64 + (N % 64 != 0)`, using checked host arithmetic.
Validate against the actual device's maximum workgroups/dimension. N=0 returns
an empty result without zero-sized bindings or a dispatch. Record zero dispatches;
a zero-length test alone cannot establish GPU execution. In required-device
mode adapter acquisition still must succeed.

For v1, every invocation writes only `output0[i]`, input buffers are read-only,
and output is fresh. These are deliberate structural constraints that simplify
bounds and noninterference reasoning. Their enforcement depends on the
validator, wrapper renderer and runtime binding checks; frontend linearity alone
does not prove them. Never count a robustness-driven out-of-bounds zero read or
discarded write as evidence that the intended bounds checks worked.

## 4. KernelIR and bundle format

Use a small typed structured IR, not a second Austral parser. Preserve source
locations as diagnostic data. Expression variants are parameter/local, u32/bool
literal, typed intrinsic, comparison, Boolean operation, conditional and resolved
helper call. Statements are immutable let, sequence, conditional and return.
Represent the generated memory plan explicitly: `map1`/`map2`, identity output
index, binding roles, guard and workgroup size. The renderer consumes that plan;
proof tooling must not maintain another handwritten wrapper specification.

Use JSON schema identifier `austral.gpu.kernel.v1`. `kernel.json` contains:

| Field | Required content |
|---|---|
| `schema` | Exact identifier above |
| `backend` / `numeric_profile` | `wgsl` / `u32-modular-v1` |
| `entry` / `input_arity` | Qualified source name / 1 or 2 |
| `compiler` | Compiler version, build/source identity when available, dirty/unknown explicitly recorded |
| `sources` | Deterministically ordered logical paths and SHA-256 byte digests for the admitted source closure and canonical builtin source |
| `kernel_ir` | Typed IR plus memory plan, with deterministic local names/order |
| `ir_sha256` | SHA-256 of specified canonical semantic-IR bytes |
| `wgsl_sha256` | SHA-256 of the exact `kernel.wgsl` bytes |
| `requirements` | Binding/element layout, fixed local size, limits and required backend features |
| `evidence` | Per-claim statuses; `not-run`, `checked`, `proved-under-assumptions`, `gap`; never one unqualified `verified` flag |
| `proof_manifest` | Null, or a reference to fixed-relative `proof-manifest.json` with SHA-256, using `austral.proof-chain.v1` |

The optional proof manifest follows [the shared proof-chain contract](../PROOF_CHAIN.md).
It binds exact semantic subjects, imported theorems, lowering edges and residual
obligations. Absence means no composed MM0 claim. A proof-required policy must
run the proof-chain checker and inspect actual transitive premises; successful
shader validation, a Lean/Why3 verdict or a manifest hash cannot satisfy it.
The first execution-tested release can omit proofs with that absence visible.

Freeze canonical IR serialization with byte fixtures before adding caches:
UTF-8, no insignificant whitespace, stable declared field order, decimal-string
u32 values and explicitly ordered arrays. Exclude source locations/absolute
paths from semantic IR bytes; preserve their separate source-file digests.
There are no duplicate keys, floating JSON numbers or hash-map iteration order.
The Rust reader must reproduce the semantic IR digest from the parsed value.
Reject unknown fields/variants in this schema and references to nonexistent
locals/functions. The Rust reference evaluator uses only validated IR, with a
bounded instruction/call budget.

Write bundles to a sibling staging directory on the destination filesystem,
validate all files, then rename to a previously absent final directory. Refuse
an existing destination. On failure, do not publish a partial/new valid-looking
bundle or destroy an earlier artifact. Staging cleanup is bounded to the
compiler-created directory.

Before publication, the compiler invokes the CPU-only `avm-gpu validate` checker
on the staging directory. Locate it through `AUSTRAL_GPU_CHECKER` (an executable
path override), then PATH. Missing checker, timeout, nonzero exit or an invalid
structured success response refuses compilation. Invoke with an argument vector,
never by interpolating paths/data into shell code. Its schema and returned
digests must match the requested staged bundle. There is no v1 flag that publishes
an unvalidated bundle as a successful GPU compilation.

The checker also admits only the canonical v1 memory wrapper in parsed Naga IR:
the exact resource roles, identity invocation index, bounds guard dominating
every storage read, and one identity-index output write. Scalar helpers may not
access storage globals. Refuse extra accesses, loops, atomics and barriers. This
is a small fixed-pattern audit with tests, not a general shader verifier; use
Naga's typed representation rather than searching WGSL text for substrings.

Hashes detect stale/mixed artifact bytes. They are not signatures or proofs of
source equivalence. V1 is a **trusted developer artifact** tool; deployment of
untrusted bundles is unsupported. Naga validation, binding reflection and the
memory-wrapper audit do not establish that scalar shader computations implement
`kernel_ir`. No shader passthrough,
unsafe validation bypass or “all capabilities” shortcut belongs in this path.

## 5. Runtime and commands

Create an isolated crate at `safestos/gpu`, without adding it to the CPU JIT's
default dependencies. One library plus `avm-gpu` binary is sufficient. Default
features cover metadata, IR reference evaluation and Naga WGSL validation;
`device` enables wgpu dispatch. Pin Naga/wgpu 29.0.4 with a separate lock initially.
Use the pinned APIs, not remembered examples from another version.

Proposed commands:

```text
avm-gpu validate BUNDLE
avm-gpu reference BUNDLE --inputs FILE
avm-gpu probe --require-hardware
avm-gpu run BUNDLE --inputs FILE --require-hardware
```

Input JSON is `{"inputs":[[0,1,4294967295],[1,2,1]]}` for map2. Require
integer tokens in [0,4294967295], exact arity and equal lengths. Output JSON
contains `status`, `output`, backend/adapter identity and type, profile,
artifact digests, dispatch count and timing categories. Bound the JSON input
file before parsing, then enforce N; no u64→u32 truncation.

Use exit 0 for completed validation/reference/device work, 1 for a refused or
failed operation, 2 for usage errors, and 77 only for an optional probe/check
whose adapter is unavailable. `--require-hardware` changes unavailability into
exit 1. Explicitly classified software adapters may be used in a separately
labelled software test; they never satisfy required hardware acceptance. An
unknown adapter type is unverified, rather than assumed physical. Initialization,
shader/pipeline validation, submission, mapping and device-loss errors must be
distinct; do not convert every error into “no GPU.”

A synchronous runner on its own process/worker is enough for first release.
Use a 30-second default job wait and a separately bounded checker subprocess
wait; configurable lower limits are allowed. Timeout is an explicit outcome,
not evidence of device preemption or permission to reclaim active buffers.
It retains buffers until actual submission completion/readback, drops mapped
views before unmapping, and never reports enqueue as completion. Host timeouts
must not cause premature reuse of in-flight allocations. Async controller
integration is a later task and uses an owned PendingJob/completion state.

## 6. Required evidence and release claim

Three oracles serve different purposes:

1. Hand-computed fixtures, including boundaries and overflow, catch agreement on
   a common compiler bug.
2. The ordinary C backend plus an Austral driver executing the same source
   functions checks source-to-KernelIR correspondence independently of WGSL.
3. A Rust IR evaluator and generated shader compare at array scale. Test N=1,
   63,64,65,127,128,129,1024, the release maximum where feasible, and N=0.

Change the source body, swap arguments in a subtraction, reverse a comparison,
remove the guard, and replace the write index with a colliding expression in
scratch controls. Corresponding tests/checks must fail; a timed-out scratch
process is a failed control, not a passing GPU test. Do not submit deliberately
unbounded/malformed kernels to hardware to test a compiler refusal.

The first completed release can claim: **the admitted Austral integer map/zip
subset compiles to valid WGSL and executes on the recorded hardware adapter,
with bit-exact differential results for the tested corpus**. Pure validation,
a software adapter, an unavailable GPU or only an empty dispatch cannot satisfy
that claim. Quantified semantic preservation, arbitrary GPU memory safety,
warp efficiency, floating numerical error bounds and NPU portability remain
separate evidence claims.
