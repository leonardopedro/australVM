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
open MonoType

module Mt = Mtast

(******************************************************************************)
(* UTILITY FUNCTIONS *)
(******************************************************************************)

let qident_to_string q =
  Identifier.qident_debug_name q

let rec mono_ty_to_cps_type (ty: mono_ty): CpsGen.cps_type =
  match ty with
  | MonoInteger (_, (Width8 | Width16 | Width32 | Width64 | WidthByteSize | WidthIndex)) -> I64
  | MonoBoolean -> Bool
  | MonoUnit -> Unit
  | MonoDoubleFloat -> F64
  | MonoSingleFloat -> F64
  | MonoAddress _ -> I64
  | MonoPointer _ -> I64
  | MonoSpan _ -> I64
  | MonoSpanMut _ -> I64
  | MonoReadRef _ -> I64
  | MonoWriteRef _ -> I64
  | MonoFnPtr _ -> I64
  | _ -> I64

(******************************************************************************)
(* MAST → CPS EXPRESSION CONVERSION *)
(******************************************************************************)

let rec convert_expr (expr: Mt.mexpr): CpsGen.cps_expr =
  match expr with
  | Mt.MNilConstant -> IntLit 0L
  | Mt.MBoolConstant b -> BoolLit b
  | Mt.MIntConstant s ->
      (try IntLit (Int64.of_string s)
       with _ -> IntLit 0L)
  | Mt.MFloatConstant s ->
      (try FloatLit (float_of_string s)
       with _ -> FloatLit 0.0)
  | Mt.MStringConstant s -> StringLit (Escape.unescape_string s)
  | Mt.MConstVar (q, _) -> Var (qident_to_string q)
  | Mt.MParamVar (id, _) -> Var (ident_string id)
  | Mt.MLocalVar (id, _) -> Var (ident_string id)
  | Mt.MTemporary (id, _) -> Var (ident_string id)
  | Mt.MGenericFunVar (id, _) -> Var (show_mono_id id)
  | Mt.MConcreteFunVar (id, _) -> Var ("fun_" ^ show_decl_id id)
  | Mt.MConcreteFuncall (_, q, args, _) ->
      App (qident_to_string q, List.map convert_expr args)
  | Mt.MGenericFuncall (id, args, _) ->
      App (show_mono_id id, List.map convert_expr args)
  | Mt.MConcreteMethodCall (_, q, args, _) ->
      App (qident_to_string q, List.map convert_expr args)
  | Mt.MGenericMethodCall (_, id, args, _) ->
      App (show_mono_id id, List.map convert_expr args)
  | Mt.MFptrCall (id, args, _) ->
      App (ident_string id, List.map convert_expr args)
  | Mt.MCast (e, _) -> convert_expr e
  | Mt.MComparison (op, e1, e2) ->
      let left = convert_expr e1 in
      let right = convert_expr e2 in
      (match op with
       | LessThan -> CmpLt (left, right)
       | GreaterThan -> CmpGt (left, right)
       | LessThanOrEqual -> CmpLte (left, right)
       | GreaterThanOrEqual -> CmpGte (left, right)
       | Equal -> CmpEq (left, right)
       | NotEqual -> CmpNeq (left, right))
  | Mt.MConjunction (e1, e2) -> And (convert_expr e1, convert_expr e2)
  | Mt.MDisjunction (e1, e2) -> Or (convert_expr e1, convert_expr e2)
  | Mt.MNegation e -> Not (convert_expr e)
  | Mt.MIfExpression (cond, then_e, else_e) ->
      App ("__if_expr", [convert_expr cond; convert_expr then_e; convert_expr else_e])
  | Mt.MRecordConstructor (_, fields) ->
      App ("__record_new", List.map (fun (_, e) -> convert_expr e) fields)
  | Mt.MUnionConstructor (_, tag, fields) ->
      App ("__union_new", BoolLit true :: Var (ident_string tag) :: List.map (fun (_, e) -> convert_expr e) fields)
  | Mt.MEmbed (_, _code, args) ->
      App ("__embed", List.map convert_expr args)
  | Mt.MDeref e -> App ("__deref", [convert_expr e])
  | Mt.MTypecast (e, _) -> convert_expr e
  | Mt.MSizeOf _ -> IntLit 0L
  | Mt.MSlotAccessor (e, name, _) -> App ("__slot_get", [convert_expr e; Var (ident_string name)])
  | Mt.MPointerSlotAccessor (e, name, _) -> App ("__ptr_slot_get", [convert_expr e; Var (ident_string name)])
  | Mt.MArrayIndex (arr, idx, _) -> App ("__array_index", [convert_expr arr; convert_expr idx])
  | Mt.MSpanIndex (span, idx, _) -> App ("__span_index", [convert_expr span; convert_expr idx])

(******************************************************************************)
(* MAST → CPS STATEMENT CONVERSION *)
(******************************************************************************)

let rec convert_stmt (stmt: Mt.mstmt): CpsGen.cps_stmt =
  match stmt with
  | Mt.MSkip -> Skip
  | Mt.MLet (id, ty, body) ->
      let var_name = ident_string id in
      let init = zero_value (mono_ty_to_cps_type ty) in
      Let (var_name, init, convert_stmt body)
  | Mt.MDestructure (bindings, expr, body) ->
      let tmp_name = "__destructure_tmp" in
      let tmp_let = Let (tmp_name, convert_expr expr, Skip) in
      let binding_stmts = List.map (fun (Mtast.MonoBinding { rename; name; _ }) ->
        Let (ident_string rename, App ("__slot_get", [Var tmp_name; Var (ident_string name)]), Skip)
      ) bindings in
      let body_stmt = convert_stmt body in
      let combined = List.fold_right (fun s acc -> Block (s, acc)) binding_stmts body_stmt in
      Block (tmp_let, combined)
  | Mt.MAssign (dest, src) ->
      let dest_expr = convert_expr dest in
      let src_expr = convert_expr src in
      (match dest_expr with
       | Var name -> Assign (name, src_expr)
       | _ -> Assign ("__assign_target", src_expr))
  | Mt.MAssignVar (q, src) ->
      Assign (qident_to_string q, convert_expr src)
  | Mt.MInitialAssign (q, src) ->
      Assign (qident_to_string q, convert_expr src)
  | Mt.MIf (cond, then_stmt, else_stmt) ->
      If (convert_expr cond, convert_stmt then_stmt, convert_stmt else_stmt)
  | Mt.MCase (expr, whens, _) ->
      let scrutinee = convert_expr expr in
      let tmp_name = "__match_scrutinee" in
      let scrutinee_let = Let (tmp_name, scrutinee, Skip) in
      let case_stmts = List.map (fun (Mtast.MTypedWhen (tag, bindings, body)) ->
        let tag_check = CmpEq (Var tmp_name, Var (ident_string tag)) in
        let binding_stmts = List.map (fun (Mtast.MonoBinding { rename; ty; _ }) ->
          Let (ident_string rename, zero_value (mono_ty_to_cps_type ty), Skip)
        ) bindings in
        let body_stmt = convert_stmt body in
        let combined = List.fold_right (fun s acc -> Block (s, acc)) binding_stmts body_stmt in
        If (tag_check, combined, Skip)
      ) whens in
      let combined_cases = List.fold_right (fun s acc -> Block (s, acc)) case_stmts Skip in
      Block (scrutinee_let, combined_cases)
  | Mt.MWhile (cond, body) ->
      While (convert_expr cond, convert_stmt body)
  | Mt.MFor (id, start_expr, end_expr, body) ->
      let var_name = ident_string id in
      let init_let = Let (var_name, convert_expr start_expr, Skip) in
      let cond = CmpLt (Var var_name, convert_expr end_expr) in
      let inc = Assign (var_name, Add (Var var_name, IntLit 1L)) in
      let loop_body = Block (convert_stmt body, inc) in
      Block (init_let, While (cond, loop_body))
  | Mt.MBorrow { rename; body; _ } ->
      Let (ident_string rename, IntLit 0L, convert_stmt body)
  | Mt.MBlock (s1, s2) ->
      Block (convert_stmt s1, convert_stmt s2)
  | Mt.MDiscarding expr ->
      Discard (convert_expr expr)
  | Mt.MReturn expr ->
      Return (convert_expr expr)
  | Mt.MLetTmp (id, _, expr) ->
      Let (ident_string id, convert_expr expr, Skip)
  | Mt.MAssignTmp (id, expr) ->
      Assign (ident_string id, convert_expr expr)

and zero_value (ty: CpsGen.cps_type): CpsGen.cps_expr =
  match ty with
  | I64 -> IntLit 0L
  | I32 -> IntLit 0L
  | Bool -> BoolLit false
  | Unit -> IntLit 0L
  | String -> IntLit 0L
  | F64 -> FloatLit 0.0

(******************************************************************************)
(* BUILD CPS FUNCTION FROM MAST DECLARATION *)
(******************************************************************************)

let build_cps_function (decl: Mt.mdecl): CpsGen.function_def option =
  match decl with
  | Mt.MFunction (_, name, params, ret_ty, body) ->
      let func_name = ident_string name in
      let param_names = List.map (fun (Mtast.MValueParameter (id, _)) -> ident_string id) params in
      let return_type = mono_ty_to_cps_type ret_ty in
      let body_stmt = convert_stmt body in
      Some {
        name = func_name;
        params = param_names;
        return_type = return_type;
        body = body_stmt
      }
  | Mt.MFunctionMonomorph (id, params, ret_ty, body) ->
      let func_name = "monomorph_" ^ show_mono_id id in
      let param_names = List.map (fun (Mtast.MValueParameter (pid, _)) -> ident_string pid) params in
      let return_type = mono_ty_to_cps_type ret_ty in
      let body_stmt = convert_stmt body in
      Some {
        name = func_name;
        params = param_names;
        return_type = return_type;
        body = body_stmt
      }
  | Mt.MMethodMonomorph (id, params, ret_ty, body) ->
      let func_name = "method_" ^ show_mono_id id in
      let param_names = List.map (fun (Mtast.MValueParameter (pid, _)) -> ident_string pid) params in
      let return_type = mono_ty_to_cps_type ret_ty in
      let body_stmt = convert_stmt body in
      Some {
        name = func_name;
        params = param_names;
        return_type = return_type;
        body = body_stmt
      }
  | Mt.MConstant (_, name, ty, expr) ->
      let const_name = ident_string name in
      let return_type = mono_ty_to_cps_type ty in
      Some {
        name = const_name;
        params = [];
        return_type = return_type;
        body = Return (convert_expr expr)
      }
  | _ -> None

(******************************************************************************)
(* MAIN ENTRY POINT: COMPILE MODULE *)
(******************************************************************************)

let compile_module_cps (mono_module: Mt.mono_module): CpsGen.function_def list =
  match mono_module with
  | Mt.MonoModule (_, decls) ->
      List.filter_map build_cps_function decls

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
              (ident_string name) (List.length params)
        | Mt.MFunctionMonomorph (id, params, _, _) ->
            Printf.printf "  FunctionMonomorph: %s (params: %d)\n"
              (show_mono_id id) (List.length params)
        | Mt.MConstant (_, name, _, _) ->
            Printf.printf "  Constant: %s\n" (ident_string name)
        | Mt.MRecord (_, name, _) ->
            Printf.printf "  Record: %s\n" (ident_string name)
        | _ -> Printf.printf "  Other declaration\n"
      ) decls

let debug_print_cps_functions (funcs: CpsGen.function_def list) =
  Printf.printf "CPS Functions: %d\n" (List.length funcs);
  List.iter (fun func ->
    Printf.printf "  %s(%s) -> " func.name (String.concat ", " func.params);
    Printf.printf "%s\n" (match func.return_type with
      | I64 -> "I64" | I32 -> "I32" | Bool -> "Bool" | Unit -> "Unit" | String -> "String" | F64 -> "F64");
    Printf.printf "    Body: %s\n" (CpsGen.string_of_stmt func.body)
  ) funcs
