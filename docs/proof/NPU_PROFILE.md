# The first NPU branch: one target, one operator, one opaque cut (PC11)

PC11 asks for an NPU proof branch to be *defined*: one actual target/runtime, one
supported tensor operation, an inventory of which stages expose semantics, MM0
obligations for the inspectable stages, and — where the vendor compiler emits an
opaque artifact — its identity plus an explicit unproved
operator→binary/firmware edge. This document is the profile; the data is
`mm0/proof_chain/npu/{target,quantdot}.toml`, the statements are
`mm0/proof_chain/npu_operator.mm0`, the admitted side is
`examples/npu/QuantDot.{aui,aum}`, and the gate is `scripts/npu-profile check`.

Nothing here was run on an NPU. `target.toml` records `host_measured = false`:
this host has no `/dev/accel`, no `xrt-smi`, no Peano and no IRON. Every row
below marked *declared* is therefore a `[gap]`, not evidence, and the profile
would say the same on a host that did have the hardware — the difference would be
artifacts to check, not a different claim.

## Why this target

A GPU branch always has a virtual ISA to fall back on: PTX is portable text the
driver translates, so a PTX capture is a real artifact even without a device. An
NPU branch has no such fallback, and the card exists to make that visible rather
than paper over it. AMD XDNA (Ryzen AI) is chosen because its stack splits
cleanly along that line:

- **Open compilers.** IRON on `mlir-aie` produces the array program and its
  instruction stream; Peano on `llvm-aie` compiles one AIE compute tile. Their IR
  is inspectable *in principle* — no artifact of either exists here.
- **Documented binaries.** An application ships an overlay (xclbin identity) and
  a ctrlcode stream of `XAie_TxnOpcode` commands, including
  `XAIE_IO_BLOCKWRITE` for DMA and the ERT/MERT split that runs it.
- **Opaque instructions.** The compute tiles are AIE-ML VLIW processors whose
  instruction semantics exist only as an architecture manual (AM020). There is no
  machine-checkable model of them in this repository, or publicly.
- **Signed firmware.** The NPU firmware is vendor-signed and loaded through the
  PSP; the driver does not interpret ctrlcode. Its scheduling and completion
  semantics are not a public specification.

The citations in `target.toml` are references, not evidence: nothing in this
repository fetched them, and `scripts/npu-profile` does not check them. They are
recorded so a reviewer can check the inventory's sentences against their sources.

## The operator

`A[4x4 i8] x B[4x4 i8] -> C[4x4 s32]`, mapped from one admitted Austral step:

| admitted fact | value |
|---|---|
| module / step | `QuantDot` / `dot4` |
| arity and types | 8 parameters, all `Int32`; result `Int32` |
| body | 4 products, 3 additions (counted in the `.aum`, not retyped) |
| compiled | `austral compile … --target-type=c`, the repository's own compiler |

The step takes eight `Int32` scalars because the admitted computation is the
*inner* step, not the tensor: the operator's job in this branch is to say what
the hardware is asked to compute, and the loop order around it is the command
list's business. The checker reads this out of the interface and body text and
re-derives the counts, so a fixture that drifts from the source is refused.

## The numeric profile

`int8_sym_exact_s32`: zero point 0, unit scales, exact widening, no
requantization.

| quantity | value | how |
|---|---|---|
| element range | `-128 … 127` | the i8 range |
| product range | `-16256 … 16384` | `(-128)(127)` … `(-128)(-128)` |
| accumulator range | `-65024 … 65536` | re-derived as `k = 4` terms, exhaustively |

This is a claim a reviewer can state: the i8 products sign-extend exactly and
accumulate in `s32` without overflow. Approximation, saturation and
requantization are *not* in this profile — they are PC12's subject — so nothing
here has to justify an error bound.

## The inventory

Nine stages, one state each, in `mm0/proof_chain/npu/target.toml`:

| stage | state | subject |
|---|---|---|
| operator | inspectable | the admitted step and the tensor descriptor it maps to |
| quantization | inspectable | `int8_sym_exact_s32`: zero point, scales, widening, no requantization |
| layout | inspectable | row-major operands and output; per-element and accumulator widths |
| dma-command | inspectable | the declared command list (`XAIE_IO_BLOCKWRITE`, host→L2, L2→host) |
| compiler-ir | declared | IRON/Peano IR: open compilers, no artifact on this host |
| overlay | declared | the xclbin identity: a format, not a semantics |
| ctrlcode | declared | `XAie_TxnOpcode` bytes: documented commands, no encoder here |
| compute-tile ISA | **opaque** | AIE-ML VLIW instructions; AM020 only |
| firmware | **opaque** | vendor-signed firmware and ERT scheduling |

An `opaque` row must name the prerequisite it is blocked on (`a machine-checkable
AIE-ML ISA model`, `vendor firmware documentation with a stated semantics`), so
"opaque" cannot be a word used to skip a question. An `inspectable` row must be
one of the card's four subjects.

## The command list, as checked

`mm0/proof_chain/npu/quantdot.toml` declares three commands moving 96 bytes of
L2 — A (16 B) and B (16 B) in, C (64 B) out — inside the SRAM gate's 256 KiB cap
(`MAX_NPU_SRAM`, read out of `lib/npu_dma_gate.ml` rather than restated). The
checker re-derives, from dtype and shape and the declared row-major layout:

- every buffer's byte count (`rows × cols × element_bytes`), and C's as
  `4 × 4 × 4`;
- each L2 offset from the buffers before it;
- all sixteen outputs' operand and store offsets, checking the stores are
  disjoint and cover `C` exactly (a partition, not merely inside it);
- every command's extent against its own buffer and against the cap;
- the direction of each transfer (an input must not be read back, an output must
  not be written outward);
- the accumulation bound by exhausting the i8 range.

## What the checker establishes, and what it does not

`scripts/npu-profile check` passes with these lines (measured on this host):

```
[pass] PC11 operator: QuantDot:dot4 admitted (8 Int32 -> Int32, 4 product(s)) and compiled
[pass] PC11 layout: 16 of 16 output(s) and their operands lie inside 64-byte C, row-major, as a partition
[pass] PC11 dma-command: 3 XAIE_IO_BLOCKWRITE command(s), 96 byte(s) of L2, inside the 262144-byte SRAM gate
[pass] PC11 quantization: bound -65024..65536 re-derived from the i8 range and k=4
[pass] PC11 obligations: 9 stage(s) have one stated obligation each, and every cite is current
[pass] PC11 opaque cut: compute-tile-isa and firmware are named, with their prerequisites; the operator -> binary/firmware edge is unproved
[pass] PC11: the operator/layout/DMA-command stages are inspectable and checked against the admitted computation; the native edge is named, not claimed
[pass] PC11 controls: 10 mutations, each refused
```

With `mm0-rs`/`mm0-c` installed it also replays the artifact
(`[pass] PC11 replay: mm0-rs and mm0-c accept mm0/proof_chain/npu_operator.mm0`);
without them that line is a `[gap]`, never a pass.

Ten mutations must be refused, so none of the above is a check that cannot fail:
a 5-row shape the 4-row tile cannot cover, a column-major layout reused, a DMA
command that overruns its buffer, an accumulation bound off by one, an admitted
arity the interface does not have, an opaque stage relabelled inspectable, an
opaque stage with no prerequisite, an output moved the wrong way, a stale
citation of the fixture, and a stage with no stated obligation.

**Not established, and not to be softened:**

- the operator→binary/firmware edge — AIE-ML instructions have no model here, the
  ctrlcode has no encoder here, and the firmware is signed vendor code;
- that the command list is what a real overlay would execute: it is this
  repository's *description* of the work, checked against the admitted step;
- any timing, scheduling or completion property — ERT/MERT behaviour is the
  firmware's, and is opaque;
- anything from a DMA certificate, a well-formed command list or a successful
  inference run, none of which can satisfy native-instruction-required policy.

The SRAM gate's 256 KiB cap contributes its narrow size property and nothing
more; the higher claims (the step's arithmetic, its bound, the layout partition)
stay reusable if this branch's native edge never closes, which is exactly the
outcome the card allows: blocked on a named external prerequisite, not worked
around.
