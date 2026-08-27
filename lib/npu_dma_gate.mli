(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   Contract for the pinned Why3 extraction of `why3_plugin/npu_dma_gate.mlw`
   (GPU.md): the compiler plugin relies on `dma_verdict` deciding the SRAM
   bound. By the WhyML postcondition (proved by Why3, extraction
   semantics-preserving):

     dma_verdict offset bytes = 0  <->  offset + bytes <= MAX_NPU_SRAM
*)

(** Physical safety of an async DMA load: true iff the transfer stays inside
    the NPU SRAM. `buf` is the `(size, offset)` pair (the linear NPU buffer). *)
val dma_ok : int * int -> int -> bool

(** The compiler-pass decision: 0 = allow (offset + bytes <= MAX_NPU_SRAM),
    1 = reject. Why3-verified postcondition. *)
val dma_verdict : int -> int -> int
