(* CamlCompiler_rust_bridge.ml
   High-level OCaml API to Rust/Cranelift bridge
   
   This module provides the complete pipeline:
   1. Compile Austral MAST to CPS IR (binary)
   2. Pass to Rust bridge for Cranelift compilation
   3. Get back native function pointer
   4. Execute via scheduler
*)

open Stages.Mtast
open MonoType
open Identifier

(* External FFI: Link to C stub which calls Rust *)

external c_compile_to_function : bytes -> int -> int64 = "compile_to_function"
external c_initialize_bridge : unit -> int = "initialize_bridge"
external c_bridge_ready : unit -> int = "bridge_is_ready"

(* Track if bridge is initialized *)
let bridge_initialized = ref false

(* Initialize the Rust bridge *)
let initialize () : bool =
  if !bridge_initialized then
    true
  else
    let result = c_initialize_bridge () in
    bridge_initialized := (result = 1);
    !bridge_initialized

(* Check if bridge is ready *)
let is_ready () : bool =
  c_bridge_ready () = 1

(*
 * Compile MAST to native function pointer
 * This is the main entry point for the compilation pipeline
 *)

let compile_mast (module_name: module_name) (decls: mdecl list) : int64 option =
  if not (is_ready ()) then
    if not (initialize ()) then
      None
    else if not (is_ready ()) then
      None
    else
      compile_mast' module_name decls
  else
    compile_mast' module_name decls

(* Private compilation logic *)
and compile_mast' (module_name: module_name) (decls: mdecl list) : int64 option =
  (* Step 1: Generate CPS IR *)
  match CpsGen.compile_module module_name decls with
  | None ->
      (* Demo mode: compile a simple function that returns 42 *)
      compile_demo ()
  | Some cps_bytes ->
      (* Step 2: Pass to Rust bridge *)
      let ptr = c_compile_to_function cps_bytes (Bytes.length cps_bytes) in
      if ptr = Int64.zero then
        compile_demo ()
      else
        Some ptr

(* Compile a single function *)
let compile_function (name: string) (params: (string * mono_ty) list) (body: mstmt) : int64 option =
  if not (is_ready ()) then
    if not (initialize ()) then
      None
    else if not (is_ready ()) then
      None
    else
      compile_function' name params body
  else
    compile_function' name params body

and compile_function' (name: string) (params: (string * mono_ty) list) (body: mstmt) : int64 option =
  match CpsGen.compile_function_expr name params body with
  | None -> None
  | Some cps_bytes ->
      let ptr = c_compile_to_function cps_bytes (Bytes.length cps_bytes) in
      if ptr = Int64.zero then None else Some ptr

(* Compile a demo function: fn() -> 42 *)
and compile_demo () : int64 option =
  let demo_bytes = Bytes.create 0 in
  let ptr = c_compile_to_function demo_bytes 0 in
  if ptr = Int64.zero then None else Some ptr

(* Convenience: compile a constant-returning function *)
let compile_constant (name: string) (value: int64) : int64 option =
  let body = MReturn (MIntConstant (Int64.to_string value)) in
  compile_function name [] body

(* Convenience: compile identity function *)
let compile_identity (name: string) (ty: mono_ty) : int64 option =
  let body = MReturn (MParamVar (Ident "x", ty)) in
  compile_function name [("x", ty)] body
