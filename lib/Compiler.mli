(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open Identifier

type compiler

val empty_compiler : compiler

val compiler_code : compiler -> string

(** The environment after the builtins and any compiled modules. *)
val cenv : compiler -> Env.env

type module_source =
  | TwoFileModuleSource of {
      int_filename: string;
      int_code: string;
      body_filename: string;
      body_code: string
    }
  | BodyModuleSource of {
      body_filename: string;
      body_code: string
    }

(** A module that passed the ordinary checked frontend (parse/combine, return
    check, desugaring, extraction, typing, plugins, linearity, body extraction
    and monomorphization). Used by non-C backends so they consume the same
    checked program as the C/CPS paths. *)
type prepared_module = {
  prepared_env : Env.env;
  prepared_name : module_name;
  prepared_typed : Stages.Tast.typed_module;
  prepared_mono : Stages.Mtast.mono_module;
}

(** Run the shared checked-frontend stages once. [after_typing] runs between
    linearity checking and body extraction (opt-in passes only). *)
val prepare_module :
  ?after_typing:(Stages.Tast.typed_module -> unit) ->
  Env.env -> module_source ->
  Env.env * prepared_module

val compile_mod : compiler -> module_source -> compiler

val compile_multiple : compiler -> module_source list -> compiler

val compile_entrypoint : compiler -> module_name -> identifier -> compiler

val post_compile : compiler -> compiler

val use_cps_jit : bool ref
val jit_server_mode : bool ref
val emit_cps_path : string option ref

(** U11: where to write the canonical adapter descriptors the CPS lowering
    requires, in the form `umbrella/src/interface.rs` parses. *)
val emit_adapters_path : string option ref

(** avm-pkg: run the package totality pass on every compiled module. *)
val pkg_totality : bool ref

(** avm-pkg: export each compiled module's `Derivation` constants to this
    plan JSON path (merging with an existing file). *)
val emit_derivations_path : string option ref

(** memsafe: export the linearity ledger — the checker's linear-resource and
    borrow decisions — to this JSON path, merging with an existing file
    (docs/memsafe.md §3). *)
val emit_linearity_ledger_path : string option ref

(** Hash table of compiled JIT function pointers keyed by function name. *)
val jit_functions : (string, int64) Hashtbl.t

(** Hot-swap: recompile the given module sources via CPS-JIT and replace
    the running JIT module. Returns true on success. *)
val cps_jit_swap_modules : module_source list -> bool
