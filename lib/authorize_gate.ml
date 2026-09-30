(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PINNED EXTRACTION — true output of:
     why3 extract -D unfer_ocaml.drv authorize_gate.mlw -o authorize_gate.ml
   (driver and source in `lib/why3_plugin/`). The WhyML program was produced
   by the unfer probability kernel (`uk_whyml_emit`); the extraction is
   semantics-preserving, so by Why3's extraction soundness the functions
   below satisfy the postconditions proved in the `.mlw`:

     authorize grants required = true  <->  required ⊆ grants
     gate_verdict grants required = 0  <->  required ⊆ grants

   Regenerate with Why3 and diff when the toolchain is present (see
   `docs/WHYML_CYCLE.md`); `authorize_gate.mli` is the contract the
   compiler plugin relies on and must not drift. The stock `ocaml64`
   driver emits Zarith `Z.t` — `unfer_ocaml.drv` maps Why3 int to native
   OCaml int so the plugin needs no zarith dependency.
*)

let rec mem (x: int) (l: (int) list) : bool =
  match l with
  | [] -> false
  | y :: r -> x = y || mem x r

let rec authorize (grants: (int) list) (required: (int) list) : bool =
  match required with
  | [] -> true
  | x :: rest -> mem x grants && authorize grants rest

let gate_verdict (grants: (int) list) (required: (int) list) : int =
  if authorize grants required then 0 else 1
