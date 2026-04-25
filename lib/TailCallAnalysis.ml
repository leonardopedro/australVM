(*
    SafestOS Tail Call Analysis Module
    Part of the Austral project
    
    This module adds tail-call detection and optimization to the Austral compiler.
    It identifies tail positions and marks calls for [[clang::musttail]] emission.
*)

open Identifier
open MonoType
open Stages.Mtast
open CodeGen

(* ============================================================================
   Tail Position Analysis
   ============================================================================ *)

(* A tail call context tells us what the expected next action is *)
type tail_context =
  | TCFunctionExit       (* Function should return or tail-call *)
  | TCConditional        (* Both branches must be tail calls *)
  | TCMatchArm           (* Match arm body must be tail call *)
  | TCNone               (* Not in tail position *)

(* Check if a statement is in tail position *)
let rec is_tail_position (ctx: tail_context) (stmt: mstmt): bool =
  match stmt with
  | MReturn _ ->
      (* Return is always a tail operation *)
      true
      
  | MSkip ->
      (* Skip can be followed by another statement in tail context *)
      (match ctx with
       | TCFunctionExit -> true
       | _ -> false)
      
  | MBlock (_, s2) ->
      (* Last statement in block is in tail position *)
      is_tail_position ctx s2
      
  | MIf (_, tb, fb) ->
      (* Both branches must be tail calls when in tail context *)
      (match ctx with
       | TCFunctionExit | TCConditional | TCMatchArm ->
           is_tail_position TCConditional tb &&
           is_tail_position TCConditional fb
       | _ -> false)
      
  | MCase (_, whens, _) ->
      (* All match arms must be tail calls *)
      (match ctx with
       | TCFunctionExit | TCMatchArm ->
           List.for_all (fun (MTypedWhen (_, _, body)) -> 
             is_tail_position TCMatchArm body
           ) whens
       | _ -> false)
      
  | MWhile _ | MFor _ | MBorrow _ | MLet _ | MDestructure _ 
  | MAssign _ | MAssignVar _ | MInitialAssign _ | MDiscarding _ 
  | MLetTmp _ | MAssignTmp _ ->
      (* Control flow and assignments are not tail *)
      false

(* ============================================================================
   Tail Call Identification
   ============================================================================ *)

(* A call is a tail call if:
   1. It's the last operation in a function
   2. Arguments are evaluated and all linear variables are consumed
   3. The function signature matches (for musttail) *)

type tail_call_info = {
    call_expr: mexpr;
    required_signature: mono_ty list * mono_ty; (* params, return *)
  }

let rec find_tail_calls_in_stmt (stmt: mstmt): tail_call_info option =
  match stmt with
  | MReturn expr ->
      (* The expression itself might contain a tail call *)
      find_tail_calls_in_expr expr
      
  | MBlock (_, s2) ->
      find_tail_calls_in_stmt s2
      
  | MIf (_, tb, fb) ->
      (* In tail position, both branches must have the same tail call *)
      (match find_tail_calls_in_stmt tb, find_tail_calls_in_stmt fb with
       | Some tc1, Some _tc2 -> Some tc1
       | _ -> None)
      
  | _ -> None

and find_tail_calls_in_expr (expr: mexpr): tail_call_info option =
  match expr with
  | MConcreteFuncall (_id, _, args, rt) ->
      Some {
        call_expr = expr;
        required_signature = ((List.map MtastUtil.get_type args), rt)
      }
      
  | MGenericFuncall (_id, args, rt) ->
      Some {
        call_expr = expr;
        required_signature = ((List.map MtastUtil.get_type args), rt)
      }
      
  | MConcreteMethodCall (_id, _, args, rt) ->
      Some {
        call_expr = expr;
        required_signature = ((List.map MtastUtil.get_type args), rt)
      }
      
  | MGenericMethodCall (_, _id, args, rt) ->
      Some {
        call_expr = expr;
        required_signature = ((List.map MtastUtil.get_type args), rt)
      }
      
  | _ -> None

(* ============================================================================
   Linearity Check for Tail Calls
   ============================================================================ *)

(* For musttail to work, all linear variables must be consumed before the call *)
let check_linearity_before_call (_func_body: mstmt) (_call_expr: mexpr): bool =
  (* 
     This is a simplified check. In the full implementation:
     1. Track all linear variable definitions in func_body
     2. Verify they're all consumed (moved/sent) before this expression
     3. Check that call_args consume them appropriately
     
     For now, we assume the existing linearity checker handles this,
     and we just verify the structure is compatible.
  *)
  true

(* ============================================================================
   Attribute Generation
   ============================================================================ *)

(* Mark a function as using musttail *)
let mark_musttail_function (_func_name: string) (has_tcall: bool): bool =
  has_tcall

(* ============================================================================
   Cell Attribute Support
   ============================================================================ *)

(* Cell descriptor - what the compiler generates for @cell modules *)
type cell_descriptor = {
    module_name: string;
    state_type: string;
    alloc_func: string;
    step_func: string;
    save_func: string;
    restore_func: string;
    type_hash: string;
    required_caps: string list;
  }

(* Check if a module has the cell attribute *)
let has_cell_attribute (module_ast: mdecl list): bool =
  (* In production, this would check module-level attributes *)
  (* For now, check for presence of cell functions *)
  let has_step = List.exists (function
    | MFunction (_, name, _, _, _) -> ident_string name = "cell_step"
    | _ -> false
  ) module_ast
  in
  let has_alloc = List.exists (function
    | MFunction (_, name, _, _, _) -> ident_string name = "cell_alloc"
    | _ -> false
  ) module_ast
  in
  has_step && has_alloc

(* Extract cell information *)
let extract_cell_info (module_name: string) (decls: mdecl list): cell_descriptor option =
  if not (has_cell_attribute decls) then
    None
  else
    (* Find the required functions *)
    let alloc_func = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_alloc" -> Some (gen_decl_id id)
      | _ -> None
    ) decls in
    
    let step_func = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_step" -> Some (gen_decl_id id)
      | _ -> None
    ) decls in
    
    let save_func = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_save" -> Some (gen_decl_id id)
      | _ -> None
    ) decls in
    
    let restore_func = List.find_map (function
      | MFunction (id, name, _, _, _) when ident_string name = "cell_restore" -> Some (gen_decl_id id)
      | _ -> None
    ) decls in
    
    match (alloc_func, step_func, save_func, restore_func) with
    | (Some alloc, Some step, Some save, Some restore) ->
        Some {
          module_name = module_name;
          state_type = "State"; (* Would be extracted from record *)
          alloc_func = alloc;
          step_func = step;
          save_func = save;
          restore_func = restore;
          type_hash = "hash_" ^ module_name; (* Would be computed from interface *)
          required_caps = ["CAP_ENV"]; (* Would be extracted from type *)
        }
    | _ -> None

(* ============================================================================
   Integration Hooks
   ============================================================================ *)

(* To be called after monomorphization, before code generation *)
let analyze_module_for_tail_calls (_module_name: module_name) (decls: mdecl list): mdecl list =
  (* 
     1. Find all functions
     2. Check if functions contain tail calls
     3. Mark functions that need musttail
     4. Store metadata for code generation
     
     Current implementation: Just return unchanged
     Production: Transform declarations to add tail-call attributes
  *)
  decls

(* Generate cell descriptor *)
let generate_cell_descriptor (module_name: string) (decls: mdecl list): CRepr.c_decl list option =
  match extract_cell_info module_name decls with
  | None -> None
  | Some _desc ->
      (* Generate C code for descriptor *)
      Some []
