(*
   C FFI Wrapper for Austral Compiler
   Exposes compiler functions to C code for runtime compilation
*)

let compile_to_c (_source : string) : string option =
  None

let compile_cell (_source : string) : (string * string) option =
  None

let () =
  Callback.register "compile_to_c" compile_to_c;
  Callback.register "compile_cell" compile_cell
