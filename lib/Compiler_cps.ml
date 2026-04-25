(*
   Phase 6: CPS Compiler Integration Layer
   Purpose: Convert Monomorphic AST (Mtast) to CPS IR binary format
   
   This module bridges the Austral compiler's monomorphized AST (Mtast)
   with the CpsGen module, creating binary IR that can be compiled
   by the Cranelift JIT backend.
*)

open Stages
open CpsGen
open Identifier
open Id

(* We need Mtast module explicitly *)
module Mt = Mtast

(******************************************************************************)
(* UTILITY FUNCTIONS *)
(******************************************************************************)

(* Convert qident to string for CPS function names *)
let qident_to_string (q: qident): string =
  let (QN (_, name)) = q in
  identifier_string name

(* Convert mono_ty to CPS type (simplified for now) *)
let rec mono_ty_to_cps_type (ty: Mt.mono_ty): CpsGen.cps_type =
  match ty with
  | Mt.MonoNamed (QN (_, name)) ->
      let name_str = identifier_string name in
      (match name_str with
       | "Int64" -> I64
       | "Int32" -> I32
       | "Bool" -> Bool
       | _ -> I64)  (* Default to i64 *)
  | Mt.MonoInteger -> I64
  | Mt.MonoBool -> Bool
  | Mt.MonoUnit -> Unit
  | _ -> I64  (* Fallback for complex types *)

(******************************************************************************)
(* MAST → CPS EXPRESSION CONVERSION *)
(******************************************************************************)

let rec convert_expr (expr: Mt.mexpr): CpsGen.cps_expr =
  match expr with
  | Mt.MNilConstant -> failwith "NilConstant not yet supported in CPS"
  | Mt.MBoolConstant b -> BoolLit b
  | Mt.MIntConstant s -> 
      (try 
         IntLit (Int64.of_string s)
       with _ -> failwith ("Invalid int literal: " ^ s))
  | Mt.MFloatConstant s -> FloatLit (float_of_string s)
  | Mt.MStringConstant s -> StringLit (Escape.unescape_string s)
  | Mt.MConstVar (q, _) -> Var (qident_to_string q)
  | Mt.MParamVar (id, _) -> Var (identifier_string id)
  | Mt.MLocalVar (id, _) -> Var (identifier_string id)
  | Mt.MTemporary (id, _) -> Var (identifier_string id)
  | Mt.MGenericFunVar (id, _) -> Var (mono_id_string id)
  | Mt.MConcreteFunVar (id, _) -> Var ("fun_" ^ string_of_int (id_to_int id))
  | Mt.MConcreteFuncall (_, q, args, _) ->
      let name = qident_to_string q in
      let args = List.map convert_expr args in
      App (name, args)
  | Mt.MGenericFuncall (id, args, _) ->
      let name = mono_id_string id in
      let args = List.map convert_expr args in
      App (name, args)
  | Mt.MConcreteMethodCall (_, q, args, _) ->
      let name = qident_to_string q in
      let args = List.map convert_expr args in
      App (name, args)
  | Mt.MGenericMethodCall (_, id, args, _) ->
      let name = mono_id_string id in
      let args = List.map convert_expr args in
      App (name, args)
  | Mt.MFptrCall (id, args, _) ->
      let name = identifier_string id in
      let args = List.map convert_expr args in
      App (name, args)
  | Mt.MCast (e, _) -> convert_expr e
  | Mt.MComparison (op, e1, e2) ->
      let left = convert_expr e1 in
      let right = convert_expr e2 in
      (match op with
       | LT -> CmpLt (left, right)
       | GT -> CmpGt (left, right)
       | LTE -> CmpLte (left, right)
       | GTE -> CmpGte (left, right)
       | EQ -> CmpEq (left, right)
       | NEQ -> CmpNeq (left, right))
  | Mt.MConjunction (e1, e2) ->
      let left = convert_expr e1 in
      let right = convert_expr e2 in
      And (left, right)
  | Mt.MDisjunction (e1, e2) ->
      let left = convert_expr e1 in
      let right = convert_expr e2 in
      Or (left, right)

(******************************************************************************)
(* MAST → CPS STATEMENT CONVERSION *)
(******************************************************************************)

let rec convert_stmt (stmt: Mt.mstmt): CpsGen.cps_stmt =
  match stmt with
  | Mt.MSkip -> Skip
  | Mt.MLet (id, _, body) ->
      let var_name = identifier_string id in
      (* Initialize with 0 as placeholder, will be assigned later *)
      Let (var_name, IntLit 0L, convert_stmt body)
  | Mt.MDestructure (_, _, _) -> failwith "Destructure not yet supported"
  | Mt.MAssign (dest, src) ->
      let dest_expr = convert_expr dest in
      let src_expr = convert_expr src in
      (match dest_expr with
       | Var name -> Assign (name, src_expr)
       | _ -> failwith "Only variable assignment supported")
  | Mt.MAssignVar (q, src) ->
      let name = qident_to_string q in
      let src_expr = convert_expr src in
      Assign (name, src_expr)
  | Mt.MInitialAssign (q, src) ->
      let name = qident_to_string q in
      let src_expr = convert_expr src in
      Assign (name, src_expr)
  | Mt.MIf (cond, then_stmt, else_stmt) ->
      let cond_expr = convert_expr cond in
      let then_branch = convert_stmt then_stmt in
      let else_branch = convert_stmt else_stmt in
      If (cond_expr, then_branch, else_branch)
  | Mt.MCase (_, _, _) -> failwith "Case/match not yet supported"
  | Mt.MWhile (_, _) -> failwith "While not yet supported"
  | Mt.MFor (_, _, _, _) -> failwith "For not yet supported"
  | Mt.MBorrow _ -> failwith "Borrow not yet supported"
  | Mt.MBlock (s1, s2) ->
      let stmt1 = convert_stmt s1 in
      let stmt2 = convert_stmt s2 in
      Block (stmt1, stmt2)
  | Mt.MDiscarding expr ->
      let expr = convert_expr expr in
      Discard expr
  | Mt.MReturn expr ->
      let expr = convert_expr expr in
      Return expr
  | Mt.MLetTmp (id, _, expr) ->
      let name = identifier_string id in
      let expr = convert_expr expr in
      Let (name, expr, Skip)
  | Mt.MAssignTmp (id, expr) ->
      let name = identifier_string id in
      let expr = convert_expr expr in
      Assign (name, expr)

(******************************************************************************)
(* BUILD CPS FUNCTION FROM MAST DECLARATION *)
(******************************************************************************)

let build_cps_function (decl: Mt.mdecl): CpsGen.function_def option =
  match decl with
  | Mt.MFunction (_, name, params, ret_ty, body) ->
      let func_name = identifier_string name in
      let param_names = List.map (fun (id, _) -> identifier_string id) params in
      let return_type = mono_ty_to_cps_type ret_ty in
      let body_stmt = convert_stmt body in
      Some {
        name = func_name;
        params = param_names;
        return_type = return_type;
        body = body_stmt
      }
      
  | Mt.MFunctionMonomorph (_, params, ret_ty, body) ->
      (* Monomorphized function - need to get unique name from mono_id *)
      (* For now, skip - will be handled in full implementation *)
      None
      
  | Mt.MConstant (_, name, ty, expr) ->
      (* Constants can be treated as 0-parameter functions *)
      let const_name = identifier_string name in
      let return_type = mono_ty_to_cps_type ty in
      let body_expr = convert_expr expr in
      let body_stmt = Return body_expr in
      Some {
        name = const_name;
        params = [];
        return_type = return_type;
        body = body_stmt
      }
      
  | _ -> 
      (* Skip records, unions, foreign functions, instances for now *)
      None

(******************************************************************************)
(* MAIN ENTRY POINT: COMPILE MODULE *)
(******************************************************************************)

let compile_module_cps (mono_module: Mt.mono_module): CpsGen.function_def list =
  match mono_module with
  | Mt.MonoModule (_, decls) ->
      (* Filter and convert each function declaration *)
      let functions = List.filter_map build_cps_function decls in
      functions

(******************************************************************************)
(* BINARY GENERATION AND WRITING *)
(******************************************************************************)

let write_cps_binary (funcs: CpsGen.function_def list) (output_path: string): unit =
  let binary = CpsGen.serialize_functions funcs in
  let oc = open_out_bin output_path in
  output_string oc binary;
  close_out oc

let compile_and_save (mono_module: Mt.mono_module) (output_path: string): unit =
  let funcs = compile_module_cps mono_module in
  if List.length funcs > 0 then
    write_cps_binary funcs output_path
  else
    ()  (* No functions to compile *)

(******************************************************************************)
(* TESTING HELPERS *)
(******************************************************************************)

let debug_print_mono_module (mono_module: Mt.mono_module) =
  match mono_module with
  | Mt.MonoModule (name, decls) ->
      Printf.printf "Module: %s\n" (mod_name_string name);
      Printf.printf "Declarations: %d\n" (List.length decls);
      List.iter (function
        | Mt.MFunction (_, name, params, _, _) ->
            Printf.printf "  Function: %s (params: %d)\n" 
              (identifier_string name) (List.length params)
        | Mt.MConstant (_, name, _, _) ->
            Printf.printf "  Constant: %s\n" (identifier_string name)
        | Mt.MRecord (_, name, _) ->
            Printf.printf "  Record: %s\n" (identifier_string name)
        | _ -> Printf.printf "  Other declaration\n"
      ) decls

let debug_print_cps_functions (funcs: CpsGen.function_def list) =
  Printf.printf "CPS Functions: %d\n" (List.length funcs);
  List.iter (fun func ->
    Printf.printf "  %s(%s) -> " 
      func.name (String.concat ", " func.params);
    Printf.printf "%s\n" (match func.return_type with
      | I64 -> "I64" | I32 -> "I32" | Bool -> "Bool" | Unit -> "Unit" | String -> "String");
    Printf.printf "    Body: %s\n" (CpsGen.string_of_stmt func.body)
  ) funcs
