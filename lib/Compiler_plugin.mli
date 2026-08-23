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
    unique-normal-form reducer. *)

type verdict =
  | VerdictOk
  | VerdictReject of string

(** Register a pass. `check` receives the module name, the `uk_*`/`uz_*`
    foreign externals the module imports, and the module's top-level
    constants as `(name, initializer)` pairs, and returns a verdict. *)
val register :
  name:string ->
  (module_name:string ->
  foreign_externals:string list ->
  constants:(Identifier.identifier * Stages.Tast.texpr) list ->
  verdict) ->
  unit

(** Run every registered pass on the given externals and constants; the
    first reject wins. *)
val run :
  module_name:string ->
  foreign_externals:string list ->
  constants:(Identifier.identifier * Stages.Tast.texpr) list ->
  verdict

(** Extract the `uk_*`/`uz_*` foreign externals and the top-level constants
    from a typed module and run every registered pass on it. Called by
    `Compiler.compile_mod` after typing, before codegen. *)
val run_on_typed : Stages.Tast.typed_module -> verdict

(** Drop every registered pass (test/QA reset). *)
val reset : unit -> unit

(** Names of the registered passes. *)
val list_registered : unit -> string list
