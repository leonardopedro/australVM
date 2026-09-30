(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   Interface of the Why3-extracted authorization gate (S36). The
   implementation `authorize_gate.ml` is the pinned output of
   `why3 extract -D ocaml64 authorize_gate.mlw` — a semantics-preserving
   translation of the WhyML program produced by the unfer probability kernel
   (`uk_whyml_emit`). By Why3's extraction soundness both functions satisfy
   the postcondition proved in the `.mlw`:

     authorize grants required = true  <->  required ⊆ grants
     gate_verdict grants required = 0  <->  required ⊆ grants

   This interface is the contract the compiler plugin relies on; the `.ml` is
   regenerated (and diffed) whenever Why3 is available.
*)

val authorize : int list -> int list -> bool
val gate_verdict : int list -> int list -> int
