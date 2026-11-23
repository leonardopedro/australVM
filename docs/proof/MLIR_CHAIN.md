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
certified. G20b publishes the versioned artifact (below), but publication is
not preservation either. The record therefore stays a **boundary record** that (a) pins the pre-MLIR subject to the identity the chain already
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
  digests under `mm0/proof_chain/mlir/ir/`; so is `mlir-canonicalize`, whose
  pre side is exactly the previous pass's post artifact and whose post side is
  `clamp.canonical.mlir` from `mlir-opt --canonicalize` (the rewrite is
  visible: `scf.if` becomes `arith.select`); `convert-gpu-to-nvvm` stays
  `uncaptured` with the concrete reason.

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
4. the subject's `captured_passes` cell equals the number of captured rows;
5. adjacent captured rows chain: a captured pass's `pre_file`/`pre_sha256` must
   be its predecessor's `post_file`/`post_sha256`, so two blocks that each hash
   correctly cannot describe different routes.

Capturing a pass moves it to `captured` but never to *proved*: while a pass is
captured the checker prints `[gap] PC09: N pass(es) captured, but capture is not
preservation: no MLIR semantics is available`, and every uncaptured pass is
counted in a second `[gap]`. Both are true at once: the *record* is current, the
*preservation claim* is not made.

`mlir-canonicalize` is re-captured with
`python3 scripts/mlir-chain capture-canonicalize`, which resolves `mlir-opt`
from the G19a record (or `PATH`), refuses to start unless the pre side is the
recorded `kernelir-to-mlir` output, and rewrites the row from the produced
bytes. Without `mlir-opt` it exits 3 and the row stays `uncaptured`.

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

`scripts/mlir-chain selftest` — seven, each mutating one evidence cell:

| control | refused by |
|---|---|
| comparison predicate `GreaterThan` → `LessThan` in the KernelIR | the record must match the reflected predicate |
| a pass option `forbidden` → `allowed` | the options are pinned in the checker |
| an uncaptured pass row dropped | the route is pinned in the checker |
| an uncaptured pass forged to `captured` | its IR file must exist and hash |
| a stale `kernel_ir_sha256` | it must equal the reflected digest |
| a restated `captured_passes` count | it must equal the captured rows |
| a captured row whose IR is missing or does not hash | both sides are re-read and hashed (the forged-row control exercises it) |
| a captured pass whose pre side is not its predecessor's post | the digest chain compares the two rows' `pre`/`post` fields |

The first two are PC09's own acceptance test: "Mutate a comparison predicate or
pass option and stale/invalid evidence is refused."

## The published bundle (G20b)

`austral compile <aui>,<aum> --target-type=mlir --entrypoint=M:f
--gpu-static-elements=65 --output=DIR` publishes a versioned artifact whose
files chain by digest: `kernel.kernelir` (the canonical `Gpu_ir` bytes),
`kernel.mlir` (the same KernelIR rendered), `kernel.ptx` (the G19a pipeline's
lowering of that module) and `bundle.json` (the `derivatives` array with each
stage's `sha256`, `predecessor` and `produced_by`, plus the source captures,
the numeric contract, the lowering identity and the measured PTX ABI).
Publication stages under `DIR.staging-<pid>`, runs `mlir-opt`, extracts the PTX,
checks the ABI (five `.u64` per memref operand), verifies the set with
`Gpu_mlir_bundle.verify_dir`, and only then renames it into place.

`gpu_proof verify-mlir-bundle DIR` re-hashes each file, requires the MLIR
module's own `// kernelir-sha256:` header to name the recorded IR (so a forged
`kernel.kernelir` with a recomputed JSON digest still fails), re-derives the
ABI from the PTX, and refuses any pipeline other than the pinned one.
`scripts/mlir-bundle` exercises the end to end: two MLIR publications
byte-identical, the WGSL bundle of the same fixture agreeing on
`48caf04e…`, and the named refusals (tampered PTX, conflicting options, invalid
extent, missing toolchain). The ABI header stays `g19a-pending`: publication
says the bytes are consistent, never that the interface is proved.

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

## The published artifact (G20b)

`austral compile --target-type=mlir` publishes the same KernelIR through the
compiler's own frontend, so the WGSL and MLIR lanes cannot disagree about their
input:

| file | content | bound to |
|---|---|---|
| `kernel.kernelir` | `Gpu_ir.canonical_bytes` | the source + builtin digests in `bundle.json` |
| `kernel.mlir` | `Gpu_mlir.render`, carrying `// kernelir-sha256:` | `kernel.kernelir` |
| `kernel.ptx` | the pinned pipeline's `sm_70` output | `kernel.mlir` |
| `bundle.json` | digests, toolchain, target, pipeline, ABI, numeric contract | all three |

The check is not a hash table: `gpu_proof verify-mlir-bundle` re-hashes each
file **and** requires the module's own header digest to equal the recorded IR
digest, so re-hashing a forged `kernel.kernelir` in `bundle.json` does not
rescue the pair. It also re-derives the ABI from the PTX (one `memref` per
operand → five `.u64`, so ten for `clamp`) and refuses a different target, width,
entry or missing `cvta.to.global`.

`scripts/mlir-bundle` compiles the fixture through WGSL and MLIR, requires the
published KernelIR identity and bytes to equal the WGSL bundle's, publishes the
MLIR set twice for byte identity, and fires every refusal the card names: a
tampered lower artifact, a conflicting numeric profile, a reduction on this
target, a non-positive static extent, a missing `mlir-opt`, a nonexistent tool
override — plus the positive control that a CPU target still compiles with no
MLIR installed.

## What is not claimed, and what comes next

- **No preservation proof.** Two passes are captured, so nothing here says the
  emitted or canonicalized `MLIR` computes the same function as the `KernelIR`
  it came from. `check` fails rather than passes if every row claims `captured`
  while the pass proofs are still absent.
- **Verified and lowered, not executed.** `mlir-opt` accepts the module and
the pinned pipeline emits `sm_70` PTX for it (G19a), but nothing is dispatched
and no GPU claim is made; the emitter's header still says `abi: g19a-pending`
because the *interface* is not proved, only exercised.
- **No dialect semantics.** The record says the dialect profile is absent; it
  does not describe one.
- **Next, in order:** capture `convert-gpu-to-nvvm` as a pre/post pair, and
  state and (if a reviewed MLIR semantics is available) prove the pass-local
  relations — reusing PC03's composition and PC04's algorithm theorem, which is
  what PC09 asks for.
