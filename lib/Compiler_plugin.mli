(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(** The compiler-plugin seam (S36): the australVM plugin system extended to
    the compiler itself. A plugin is a registered check that runs on every
    typed module; the Why3-extracted OCaml modules produced by the unfer
    probability kernel (`uk_whyml_emit`) are the canonical plugins — their
    properties are machine-checked by Why3 before extraction, so the pass
    carries a verified guarantee (see `unfer/docs/WHYML_CYCLE.md`). The
    deltanet UNF gate (`Deltanet_plugin`) is the DeltaNets analogue: it
    recomputes top-level constants through the kernel's `uk_austral_unf`
    unique-normal-form reducer.

    Two pass kinds share one ordered registry. A [gate_pass] sees only the
    module name, the kernel foreign externals, and the top-level constants —
    the WHYML_CYCLE §3 surface, unchanged. A [typed_pass] sees the whole
    typed module, which is what PLAN_liquid_types.md L1 needs: the liquid
    contract pass must read per-declaration pragmas, and the gate signature
    cannot see them. [run_on_typed] runs both kinds in registration order and
    the first rejection wins. *)

type verdict =
  | VerdictOk
  | VerdictReject of string

type gate_pass =
  module_name:string ->
  foreign_externals:string list ->
  constants:(Identifier.identifier * Stages.Tast.texpr) list ->
  verdict

type typed_pass = Stages.Tast.typed_module -> verdict

(** Register a gate pass, replacing any existing pass of the same name. *)
val register : name:string -> gate_pass -> unit

(** Register a typed pass, replacing any existing pass of the same name. *)
val register_typed : name:string -> typed_pass -> unit

(** Run only the gate passes against facts supplied by the caller. *)
val run :
  module_name:string ->
  foreign_externals:string list ->
  constants:(Identifier.identifier * Stages.Tast.texpr) list ->
  verdict

(** Run every registered pass (gate and typed alike) over a typed module, in
    registration order; the first rejection wins. Called by
    `Compiler.compile_mod` after typing, before codegen. *)
val run_on_typed : Stages.Tast.typed_module -> verdict

(** The foreign externals a typed module imports (`uk_*` / `uz_*`). *)
val foreign_externals_of : Stages.Tast.typed_module -> string list

(** The top-level constants of a typed module, as `(name, initializer)`. *)
val constants_of :
  Stages.Tast.typed_module -> (Identifier.identifier * Stages.Tast.texpr) list

(** The pragmas carried by each function-like declaration — the surface the
    gate signature cannot reach, and the reason [typed_pass] exists. *)
val decl_pragmas_of :
  Stages.Tast.typed_module -> (Identifier.identifier * Common.pragma list) list

(** Drop every registered pass (test/QA reset). *)
val reset : unit -> unit

(** Drop one registered pass by name. *)
val unregister : string -> unit

(** Names of every registered pass, gate and typed, in registration order. *)
val names : unit -> string list

(** Names of the registered gate passes only. *)
val list_registered : unit -> string list