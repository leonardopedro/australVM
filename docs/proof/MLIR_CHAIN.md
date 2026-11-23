# PC09 — the MLIR stage boundary, declared and pinned

[CHAIN_GATE.md](CHAIN_GATE.md) composes the chain that exists. This page records
the boundary of the stage that is only partly pinned, so that the gap is
specific instead of prose.

## What exists

`Austral → KernelIR` is real and already certified: PC04 reflects the `clamp`
fixture's KernelIR into `mm0/proof_chain/clamp_source.mm0`, PC06 composes it
into the GPU plan, and PC07 freezes the chain whose identity is
`ir_sha256 = 48caf04e…`.

`KernelIR → MLIR` is now emitted: `lib/gpu/Gpu_mlir.ml` (G20a) is the second
consumer of `Gpu_ir.kernel`, beside `Gpu_wgsl`, and `gpu_proof dump-mlir`
writes the module for the reflected `clamp` subject. `G19a`'s pinned toolchain
round trip verifies it and lowers it to `sm_70` PTX (see below), but no MLIR
*semantics* is declared, so the module is captured and exercised, not
certified: `G20b` remains unbuilt. The record therefore stays a **boundary
record** that (a) pins the pre-MLIR subject to the identity the chain already
trusts, (b) names every pass of the route, (c) records the captured pass's
pre/post bytes, (d) counts the uncaptured ones, and (e) refuses stale evidence.

## The record

`mm0/proof_chain/mlir/stage.toml`:

- `[subject]` — module, entry, comparison predicate and `kernel_ir_sha256`, plus
  the dialect profile (`none — no MLIR dialect semantics is declared`) and the
  captured pass count.
- `[[option]]` — the identity inputs PC09 requires: numeric profile, static
  extents, `fast_math`, `fusion`, `shared_memory`, target. These are pinned in
  the checker (`DECLARED_OPTIONS`), not just restated in the record, so editing a
  record cell is a refusal.
- `[[pass]]` — `kernelir-to-mlir` is `state = "captured"` with
  `pre_file = "clamp.kernelir"`, `post_file = "clamp.mlir"` and both sha256
  digests under `mm0/proof_chain/mlir/ir/`; `mlir-canonicalize` and
  `convert-gpu-to-nvvm` stay `uncaptured` with the concrete reason.

The pass list is pinned in `DECLARED_PASSES`, so an uncaptured optimization
cannot fall out of the chain by deleting its row — the failure PC09's own text
names.

## What `check` verifies

`scripts/mlir-chain check`:

1. the record's `kernel_ir_sha256`, `comparison` and `entry` equal the
   `-- reflect:` line and `-- ir_sha256=` of `clamp_source.mm0` (the artifact
   PC04/PC07 already trust, re-read on every run);
2. the options are exactly `DECLARED_OPTIONS`;
3. the pass list is exactly `DECLARED_PASSES`, once each, and each row is
   `captured` (both IR files exist and hash to their own recorded digest) or
   `uncaptured` (with a reason) — a forged `captured` row fails because its IR
   files do not exist;
4. the subject's `captured_passes` cell equals the number of captured rows.

Capturing a pass moves it to `captured` but never to *proved*: while a pass is
captured the checker prints `[gap] PC09: N pass(es) captured, but capture is not
preservation: no MLIR semantics is available`, and every uncaptured pass is
counted in a second `[gap]`. Both are true at once: the *record* is current, the
*preservation claim* is not made.

## The emitted MLIR (G20a)

`gpu_proof dump-mlir <aui> <aum> clamp --extent 65` renders the same KernelIR
the WGSL path consumes:

```mlir
module attributes {austral.extent = 65 : i64, austral.numeric_profile =
  "u32-modular-v1", austral.abi = "g19a-pending"} {
  gpu.module @clamp {
    gpu.func @main(%arg0: memref<65xi32>, %arg1: memref<65xi32>) kernel
        attributes {gpu.known_block_size = array<i32: 64, 1, 1>} {
      %c_extent = arith.constant 65 : index
      %tid = gpu.thread_id x
      %in_bounds = arith.cmpi ult, %tid, %c_extent : index
      scf.if %in_bounds {
        %arg0_v = memref.load %arg0[%tid] : memref<65xi32>
        %v0 = arith.constant 100 : i32
        %v1 = arith.cmpi ugt, %arg0_v, %v0 : i32
        %v3 = scf.if %v1 -> (i32) { ... scf.yield %v2 : i32 }
               else { scf.yield %arg0_v : i32 }
        memref.store %v3, %arg1[%tid] : memref<65xi32>
      }
      gpu.return
    }
  }
}
```

The rules G20a fixes are applied in the emitter, not deferred to a pass:
`Nat32` is signless `i32` with **unsigned** predicates everywhere (`ugt` above);
modular arithmetic carries no `nuw`/`nsw`; indices are `index` with an explicit
`ult` guard that dominates the load and the store; the extent is a parameter and
must be positive. `Gpu_mlir.validate` is a *structural* check (balanced regions,
every `%value` defined before use, one `gpu.func`, no signed predicate, no wrap
flag) — it is not `mlir-opt`, and the header says `abi: g19a-pending` rather
than claiming a verified interface.

Refusals are named: a helper call, a non-`u32-modular-v1` profile, a
non-positive extent, and a body that returns from only one branch of an `if` are
all `Invalid_argument`s carrying the reason. Eleven tests in
`test/GpuMlirTest.ml` hold the encoding, the refusals, the validator's own
negative controls, and G20a's mutation requirement — swapping subtraction
operands and reversing the comparison each change the lowered body.

## Controls

`scripts/mlir-chain selftest` — six, each mutating one evidence cell:

| control | refused by |
|---|---|
| comparison predicate `GreaterThan` → `LessThan` in the KernelIR | the record must match the reflected predicate |
| a pass option `forbidden` → `allowed` | the options are pinned in the checker |
| an uncaptured pass row dropped | the route is pinned in the checker |
| an uncaptured pass forged to `captured` | its IR file must exist and hash |
| a stale `kernel_ir_sha256` | it must equal the reflected digest |
| a restated `captured_passes` count | it must equal the captured rows |
| a captured row whose IR is missing or does not hash | both sides are re-read and hashed (the forged-row control exercises it) |

The first two are PC09's own acceptance test: "Mutate a comparison predicate or
pass option and stale/invalid evidence is refused."

## The toolchain round trip (G19a)

`scripts/gpu/mlir-probe` pins what a real toolchain does with these bytes, from
nixpkgs `llvmPackages_19.mlir` / `llvmPackages_19.llvm` (19.1.7):

| stage | command | result |
|---|---|---|
| verify | `mlir-opt <module>` | exit 0 for the handwritten `smoke.mlir` and for `clamp.mlir` |
| lower | `mlir-opt <module> "--gpu-lower-to-nvvm-pipeline=cubin-format=isa cubin-chip=sm_70"` | kernel outlining, memref/arith/scf → LLVM, gpu → NVVM, NVPTX backend |
| extract | the `gpu.binary` assembly string | `mm0/proof_chain/mlir/ir/{smoke,clamp}.ptx` |

The PTX is a real target: `.target sm_70`, `.address_size 64`, `.visible .entry
main`, ten `.u64` parameters (one `memref<65xi32>` lowers to alloc ptr, aligned
ptr, offset, size and stride), `cvta.to.global` for the address space, and the
tail guard visible as `setp.gt.u32 %p1, %r1, 64; @%p1 bra`. Because the pipeline
uses `cubin-format=isa` it stops at PTX text — no `ptxas`, no CUDA toolkit, and
no hardware is touched.

`mm0/proof_chain/mlir/toolchain.toml` records the three tool versions, the
target, the exact pipeline string, and per subject the source and PTX digests,
entry name, parameter layout and address-space conversion. `check` re-runs all
three stages and refuses a changed version, target, pipeline, source digest or
PTX digest (four controls in `selftest`). `scripts/mlir-chain check` uses the
same record to attest that `clamp.mlir` — the module PC09 marks `captured` — is
the file the pinned toolchain verified and lowered.

## What is not claimed, and what comes next

- **No preservation proof.** One pass is captured, so nothing here says the
  emitted `MLIR` computes the same function as the `KernelIR` it came from.
  `check` fails rather than passes if every row claims `captured` while the pass
  proofs are still absent.
- **Verified and lowered, not executed.** `mlir-opt` accepts the module and
the pinned pipeline emits `sm_70` PTX for it (G19a), but nothing is dispatched
and no GPU claim is made; the emitter's header still says `abi: g19a-pending`
because the *interface* is not proved, only exercised.
- **No dialect semantics.** The record says the dialect profile is absent; it
  does not describe one.
- **Next, in order:** `G20b` (the versioned MLIR artifact); then move
  `mlir-canonicalize` and `convert-gpu-to-nvvm` to `captured`, and state and (if
  a reviewed MLIR semantics is available) prove the pass-local relations —
  reusing PC03's composition and PC04's algorithm theorem, which is what PC09
  asks for.
