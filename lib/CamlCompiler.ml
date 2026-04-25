(*
   C FFI Wrapper for Austral Compiler
   Exposes compiler functions to C code for runtime compilation
*)

open Compiler
open Sexplib.Std

(* Convert string option to C-friendly result *)
external value : 'a -> 'b = "caml_copy_string"

(* Main entry point for typed_eval: compile source to C code *)
let compile_to_c (source : string) : string option =
  parse_and_compile_c source

(* Compile a full cell module *)
let compile_cell (source : string) : (string * string) option =
  compile_cell_module source

(* Export functions for C *)
external stub_compile_to_c : string -> string option = "austral_compile_to_c"
external stub_compile_cell : string -> (string * string) option = "austral_compile_cell"
