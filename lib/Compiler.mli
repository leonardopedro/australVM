(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open Identifier

type compiler

val empty_compiler : compiler

val compiler_code : compiler -> string

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

val compile_mod : compiler -> module_source -> compiler

val compile_multiple : compiler -> module_source list -> compiler

val compile_entrypoint : compiler -> module_name -> identifier -> compiler

val post_compile : compiler -> compiler

val use_cps_jit : bool ref
val jit_server_mode : bool ref

(** Hash table of compiled JIT function pointers keyed by function name. *)
val jit_functions : (string, int64) Hashtbl.t

(** Hot-swap: recompile the given module sources via CPS-JIT and replace
    the running JIT module. Returns true on success. *)
val cps_jit_swap_modules : module_source list -> bool
