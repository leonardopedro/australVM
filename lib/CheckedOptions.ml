(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(** U15: the checked-wrapper decision, made where the union declaration is
    visible.

    A foreign signature may name a union monomorph whose definition was
    emitted in the module that exports the interface (the Pervasive `Option`
    is imported, not redeclared here), so the CPS pass's own
    [MUnionMonomorph] table cannot describe it: the signature carries an id
    that no local declaration holds. The typing pass resolves the declaration
    through the environment, so it is the pass that knows the payload type and
    the union's case order.

    The table is keyed by the external symbol: that is what the CPS lowering
    has when it decides whether to name [`<symbol>__checked`], and an imported
    Rust function has one signature. Both passes run in one process, so nothing
    is serialized or re-derived. *)

type import = {
  (* The union's own indices, from its declared case order. *)
  failure_tag: int;
  success_tag: int;
  (* An adapter scalar: [Boolean], [Int32]/[Nat32], [Int64]/[Nat64],
     [Float32] or [Float64]. *)
  payload: Type.ty;
  (* The checkable union's own name and case names. [`Option`] is the
     payload-less-failure shape; [`Either`] carries a payload in both cases,
     which is why [failure_payload] is present for it and absent for
     [`Option`]. The names are what the descriptor writes, so the tag test can
     compare names to indices instead of assuming them. *)
  union_name: string;
  failure_case: string;
  success_case: string;
  failure_payload: Type.ty option;
}

let imports: (string, import) Hashtbl.t = Hashtbl.create 8

let record ~external_name (import: import): unit =
  Hashtbl.replace imports external_name import

let find (external_name: string): import option =
  Hashtbl.find_opt imports external_name
