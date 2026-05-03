(* CamlCompiler_rust_bridge.ml
   High-level OCaml API to Rust/Cranelift bridge
   
   This module provides the complete pipeline:
   1. Compile Austral MAST to CPS IR (binary)
   2. Pass to Rust bridge for Cranelift compilation
   3. Get back native function pointer
*)

open Stages.Mtast
open MonoType
open Identifier

external c_compile_to_function : bytes -> int -> int64 = "ocaml_compile_to_function"
external c_initialize_bridge : unit -> int64 = "ocaml_initialize_bridge"
external c_bridge_ready : unit -> int64 = "ocaml_bridge_ready"
external c_execute_function : int64 -> int64 = "ocaml_execute_function"
external c_execute_function_1 : int64 -> int64 -> int64 = "ocaml_execute_function_1"
external c_execute_function_2 : int64 -> int64 -> int64 -> int64 = "ocaml_execute_function_2"
external c_last_error : unit -> string option = "ocaml_cranelift_last_error"
external c_cedar_load_policy : string -> int64 = "ocaml_cedar_load_policy"
external c_cedar_check_runtime : string -> string -> string -> int64 = "ocaml_cedar_check_runtime"
external c_set_cell_jit_ptr : int64 -> int64 -> unit = "ocaml_set_cell_jit_ptr"
external c_cell_swap : int64 -> int64 -> int = "ocaml_cell_swap"
external c_scheduler_dispatch : unit -> unit = "ocaml_scheduler_dispatch"
external c_au_alloc : int64 -> int64 = "ocaml_au_alloc"
external c_load : int64 -> int64 = "ocaml_load"
external c_store : int64 -> int64 -> unit = "ocaml_store"

let bridge_initialized = ref false

let initialize () : bool =
  if !bridge_initialized then
    true
  else
    let result = c_initialize_bridge () in
    bridge_initialized := (result = 1L);
    !bridge_initialized

let is_ready () : bool =
  c_bridge_ready () = 1L

let compile_demo () : int64 option =
  let demo_bytes = Bytes.create 0 in
  let ptr = c_compile_to_function demo_bytes 0 in
  if ptr = Int64.zero then None else Some ptr

let last_jit_error () : string option =
  c_last_error ()

let compile_binary (binary: string) : (int64 * string option) =
  if not (is_ready ()) && not (initialize ()) then
    (Int64.zero, Some "JIT bridge not initialized")
  else
    let binary_bytes = Bytes.of_string binary in
    let ptr = c_compile_to_function binary_bytes (Bytes.length binary_bytes) in
    if ptr = Int64.zero then
      (Int64.zero, last_jit_error ())
    else
      (ptr, None)

let execute_function (ptr: int64) : int64 =
  c_execute_function ptr

let execute_function_1 (ptr: int64) (arg1: int64) : int64 =
  c_execute_function_1 ptr arg1

let execute_function_2 (ptr: int64) (arg1: int64) (arg2: int64) : int64 =
  c_execute_function_2 ptr arg1 arg2

let compile_mast (_module_name: module_name) (decls: mdecl list) : int64 option =
  if not (is_ready ()) && not (initialize ()) then None
  else
    match CpsGen.compile_module _module_name decls with
    | None -> compile_demo ()
    | Some cps_bytes ->
        let ptr = c_compile_to_function cps_bytes (Bytes.length cps_bytes) in
        if ptr = Int64.zero then compile_demo ()
        else Some ptr

let compile_function (name: string) (params: (string * mono_ty) list) (body: mstmt) : int64 option =
  if not (is_ready ()) && not (initialize ()) then None
  else
    match CpsGen.compile_function_expr name params body with
    | None -> None
    | Some cps_bytes ->
        let ptr = c_compile_to_function cps_bytes (Bytes.length cps_bytes) in
        if ptr = Int64.zero then None else Some ptr

let compile_constant (name: string) (value: int64) : int64 option =
  let body = MReturn (MIntConstant (Int64.to_string value)) in
  compile_function name [] body

let cedar_load_policy (policy: string) : bool =
  c_cedar_load_policy policy = 1L

let cedar_check (p: string) (a: string) (r: string) : bool =
  c_cedar_check_runtime p a r = 1L

let set_cell_jit_ptr (desc: int64) (jit: int64) : unit =
  c_set_cell_jit_ptr desc jit

let cell_swap (old_id: int64) (new_desc: int64) : int =
  c_cell_swap old_id new_desc

let scheduler_dispatch () : unit =
  c_scheduler_dispatch ()

let au_alloc (size: int64) : int64 =
  c_au_alloc size

let load (ptr: int64) : int64 =
  c_load ptr

let store (ptr: int64) (value: int64) : unit =
  c_store ptr value
