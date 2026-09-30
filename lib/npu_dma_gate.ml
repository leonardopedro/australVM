(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PINNED EXTRACTION — true output of:
     why3 extract -D unfer_ocaml.drv npu_dma_gate.mlw -o npu_dma_gate.ml
   (driver and source in `lib/why3_plugin/`). The WhyML program was produced
   by the unfer probability kernel (`uk_whyml_emit`, WhymlProgram::NpuDmaGate);
   the extraction is semantics-preserving, so by Why3's extraction soundness
   the functions below satisfy the postconditions proved in the `.mlw`:

     dma_ok (size, offset) bytes = true  <->  offset + bytes <= MAX_NPU_SRAM
     dma_verdict offset bytes   = 0     <->  offset + bytes <= MAX_NPU_SRAM

   with MAX_NPU_SRAM = 262144 (256 KiB — the GPU.md hardware invariant: an
   async DMA load never overflows the NPU SRAM).

   The `npu_buffer` record extracts to the `(size, offset)` pair (Why3's
   default record encoding for the `int`-mapped driver). The compiler plugin
   only consumes `dma_verdict`.

   Regenerate with Why3 and diff when the toolchain is present (see
   `docs/WHYML_CYCLE.md`); `npu_dma_gate.mli` is the contract the compiler
   plugin relies on and must not drift. The stock `ocaml64` driver emits
   Zarith `Z.t` — `unfer_ocaml.drv` maps Why3 int to native OCaml int so the
   plugin needs no zarith dependency.
*)

let max_npu_sram : int = 262144

let dma_ok (buf: (int * int)) (bytes: int) : bool =
  let (_size, offset) = buf in
  offset + bytes <= max_npu_sram

let dma_verdict (offset: int) (bytes: int) : int =
  if offset + bytes <= max_npu_sram then 0 else 1
