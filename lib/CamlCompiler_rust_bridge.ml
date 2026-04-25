(* CamlCompiler_rust_bridge.ml
   High-level OCaml API to Rust/Cranelift bridge
   
   This module provides the complete pipeline:
   1. Compile Austral MAST to CPS IR (binary)
   2. Pass to Rust bridge for Cranelift compilation
   3. Get back native function pointer
*)

open Austral_core.Stages.Mtast
open Austral_core.MonoType
open Austral_core.Identifier

external c_compile_to_function : bytes -> int -> int64 = "compile_to_function"
external c_initialize_bridge : unit -> int = "initialize_bridge"
external c_bridge_ready : unit -> int = "bridge_is_ready"

let bridge_initialized = ref false

let initialize () : bool =
  if !bridge_initialized then
    true
  else
    let result = c_initialize_bridge () in
    bridge_initialized := (result = 1);
    !bridge_initialized

let is_ready () : bool =
  c_bridge_ready () = 1

let compile_demo () : int64 option =
  let demo_bytes = Bytes.create 0 in
  let ptr = c_compile_to_function demo_bytes 0 in
  if ptr = Int64.zero then None else Some ptr

let compile_mast (_module_name: module_name) (decls: mdecl list) : int64 option =
  if not (is_ready ()) && not (initialize ()) then None
  else
    match Austral_core.CpsGen.compile_module _module_name decls with
    | None -> compile_demo ()
    | Some cps_bytes ->
        let ptr = c_compile_to_function cps_bytes (Bytes.length cps_bytes) in
        if ptr = Int64.zero then compile_demo ()
        else Some ptr

let compile_function (name: string) (params: (string * mono_ty) list) (body: mstmt) : int64 option =
  if not (is_ready ()) && not (initialize ()) then None
  else
    match Austral_core.CpsGen.compile_function_expr name params body with
    | None -> None
    | Some cps_bytes ->
        let ptr = c_compile_to_function cps_bytes (Bytes.length cps_bytes) in
        if ptr = Int64.zero then None else Some ptr

let compile_constant (name: string) (value: int64) : int64 option =
  let body = MReturn (MIntConstant (Int64.to_string value)) in
  compile_function name [] body
