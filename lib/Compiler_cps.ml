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

let ends_with s suffix =
  let len_s = String.length s in
  let len_suffix = String.length suffix in
  len_s >= len_suffix && String.sub s (len_s - len_suffix) len_suffix = suffix

let mono_ty_to_cps_type (ty: mono_ty): CpsGen.cps_type =
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

let type_size (_ty: mono_ty): int = 8 (* All types currently 8 bytes in JIT *)

let record_layouts = Hashtbl.create 16

let rec find_slot_offset slots target_name current_offset =
  match slots with
  | [] -> 0
  | MonoSlot (name, ty) :: rest ->
      if Identifier.ident_string name = Identifier.ident_string target_name then
        current_offset
      else
        find_slot_offset rest target_name (current_offset + type_size ty)

let union_layouts = Hashtbl.create 16

let rec find_union_case_offset cases target_tag_name current_offset =
  match cases with
  | [] -> 0
  | MonoCase (name, _) :: rest ->
      if Identifier.ident_string name = Identifier.ident_string target_tag_name then
        current_offset
      else
        find_union_case_offset rest target_tag_name (current_offset + 1)

let find_record_slot_offset id name =
  match Hashtbl.find_opt record_layouts id with
  | Some slots -> find_slot_offset slots name 0
  | None -> 0

let rec get_expr_type (expr: Mt.mexpr): mono_ty =
  match expr with
  | Mt.MNilConstant -> MonoUnit
  | Mt.MBoolConstant _ -> MonoBoolean
  | Mt.MIntConstant _ -> MonoInteger (Signed, Width64)
  | Mt.MFloatConstant _ -> MonoDoubleFloat
  | Mt.MStringConstant _ -> MonoPointer (MonoInteger (Unsigned, Width8))
  | Mt.MConstVar (_, ty) -> ty
  | Mt.MParamVar (_, ty) -> ty
  | Mt.MLocalVar (_, ty) -> ty
  | Mt.MTemporary (_, ty) -> ty
  | Mt.MGenericFunVar (_, ty) -> ty
  | Mt.MConcreteFunVar (_, ty) -> ty
  | Mt.MConcreteFuncall (_, _, _, ty) -> ty
  | Mt.MGenericFuncall (_, _, ty) -> ty
  | Mt.MConcreteMethodCall (_, _, _, ty) -> ty
  | Mt.MGenericMethodCall (_, _, _, ty) -> ty
  | Mt.MFptrCall (_, _, ty) -> ty
  | Mt.MCast (_, ty) -> ty
  | Mt.MComparison _ -> MonoBoolean
  | Mt.MConjunction _ -> MonoBoolean
  | Mt.MDisjunction _ -> MonoBoolean
  | Mt.MNegation _ -> MonoBoolean
  | Mt.MIfExpression (_, then_e, _) -> get_expr_type then_e
  | Mt.MRecordConstructor (ty, _) -> ty
  | Mt.MUnionConstructor (ty, _, _) -> ty
  | Mt.MEmbed (ty, _, _) -> ty
  | Mt.MDeref e -> (match get_expr_type e with MonoPointer ty | MonoAddress ty | MonoReadRef (ty, _) | MonoWriteRef (ty, _) -> ty | _ -> MonoUnit)
  | Mt.MTypecast (_, ty) -> ty
  | Mt.MSizeOf _ -> MonoInteger (Unsigned, Width64)
  | Mt.MSlotAccessor (_, _, ty) -> ty
  | Mt.MPointerSlotAccessor (_, _, ty) -> ty
  | Mt.MArrayIndex (e, _, _) -> (match get_expr_type e with MonoPointer ty | MonoAddress ty | MonoSpan (ty, _) | MonoSpanMut (ty, _) -> ty | _ -> MonoUnit)
  | Mt.MSpanIndex (e, _, _) -> (match get_expr_type e with MonoPointer ty | MonoAddress ty | MonoSpan (ty, _) | MonoSpanMut (ty, _) -> ty | _ -> MonoUnit)

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
  | Mt.MConstVar (q, _) -> 
      let name = qident_to_string q in
      if ends_with name ":ExitSuccess" || ends_with name ".ExitSuccess" || name = "ExitSuccess" then
        App ("au_exit", [IntLit 0L])
      else
        Var name
  | Mt.MParamVar (id, _) -> Var (ident_string id)
  | Mt.MLocalVar (id, _) -> Var (ident_string id)
  | Mt.MTemporary (id, _) -> Var (ident_string id)
  | Mt.MGenericFunVar (id, _) -> Var (show_mono_id id)
  | Mt.MConcreteFunVar (id, _) -> Var ("fun_" ^ show_decl_id id)
  | Mt.MConcreteFuncall (_, q, args, _) ->
      let name = qident_to_string q in
      let is_exit_success = 
        ends_with name ":ExitSuccess" || 
        ends_with name ".ExitSuccess" ||
        name = "ExitSuccess" 
      in
      if is_exit_success then
        App ("au_exit", [IntLit 0L])
      else
        App (name, List.map convert_expr args)
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
  | Mt.MRecordConstructor (ty, fields) ->
      let size = match ty with
        | MonoNamedType id -> 
            (match Hashtbl.find_opt record_layouts id with
             | Some slots -> List.length slots * 8
             | None -> 8)
        | _ -> 8
      in
      App ("__record_new", IntLit (Int64.of_int size) :: List.map (fun (_, e) -> convert_expr e) fields)
  | Mt.MUnionConstructor (ty, tag, fields) ->
      let (tag_idx, size) = match ty with
        | MonoNamedType id ->
            (match Hashtbl.find_opt union_layouts id with
             | Some cases -> 
                 (* find_union_case_offset compares identifiers, pass raw tag *)
                 let idx = find_union_case_offset cases tag 0 in
                 let max_fields = List.fold_left (fun acc (MonoCase (_, s)) -> max acc (List.length s)) 0 cases in
                 (idx, (max_fields + 1) * 8)
             | None -> (0, 16))
        | _ -> (0, 16)
      in
      App ("__union_new", IntLit (Int64.of_int size) :: IntLit (Int64.of_int tag_idx) :: List.map (fun (_, e) -> convert_expr e) fields)
  | Mt.MEmbed (_, _code, args) ->
      App ("__embed", List.map convert_expr args)
  | Mt.MDeref e -> App ("__deref", [convert_expr e])
  | Mt.MTypecast (e, _) -> convert_expr e
  | Mt.MSizeOf _ -> IntLit 0L
  | Mt.MSlotAccessor (e, name, _) -> 
      let recty = get_expr_type e in
      let offset = match recty with
        | MonoNamedType id -> find_record_slot_offset id name
        | _ -> 0
      in
      App ("__slot_get", [convert_expr e; IntLit (Int64.of_int offset)])
  | Mt.MPointerSlotAccessor (e, name, _) -> 
      let recty = match get_expr_type e with
        | MonoPointer ty | MonoAddress ty | MonoReadRef (ty, _) | MonoWriteRef (ty, _) -> ty
        | _ -> MonoUnit
      in
      let offset = match recty with
        | MonoNamedType id -> find_record_slot_offset id name
        | _ -> 0
      in
      App ("__ptr_slot_get", [convert_expr e; IntLit (Int64.of_int offset)])
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
      let scrutinee_ty = get_expr_type expr in
      let tmp_name = "__match_scrutinee" in
      let scrutinee_let = Let (tmp_name, scrutinee, Skip) in
      (* For union types, load the discriminant tag from offset 0 *)
      let tag_name = "__match_tag" in
      let tag_load = match scrutinee_ty with
        | MonoNamedType _ ->
            (* Union pointer: load tag from offset 0 *)
            Let (tag_name, App ("__slot_get", [Var tmp_name; IntLit 0L]), Skip)
        | _ ->
            (* Primitive: compare directly *)
            Let (tag_name, Var tmp_name, Skip)
      in
      let case_stmts = List.mapi (fun i (Mtast.MTypedWhen (tag, bindings, body)) ->
        (* Compute the numeric index of this tag in the union layout *)
        let tag_idx = match scrutinee_ty with
          | MonoNamedType id ->
              (match Hashtbl.find_opt union_layouts id with
               | Some cases -> find_union_case_offset cases tag 0
               | None -> i)
          | _ -> i
        in
        let tag_check = CmpEq (Var tag_name, IntLit (Int64.of_int tag_idx)) in
        (* Bind union fields: load from offsets 8, 16, ... *)
        let binding_stmts = List.mapi (fun j (Mtast.MonoBinding { rename; _ }) ->
          let offset = (j + 1) * 8 in
          Let (ident_string rename,
               App ("__slot_get", [Var tmp_name; IntLit (Int64.of_int offset)]),
               Skip)
        ) bindings in
        let body_stmt = convert_stmt body in
        let combined = List.fold_right (fun s acc -> Block (s, acc)) binding_stmts body_stmt in
        If (tag_check, combined, Skip)
      ) whens in
      let combined_cases = List.fold_right (fun s acc -> Block (s, acc)) case_stmts Skip in
      Block (scrutinee_let, Block (tag_load, combined_cases))
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
      (* First pass: collect record layouts *)
      List.iter (function
        | Mt.MRecordMonomorph (id, slots) -> Hashtbl.replace record_layouts id slots
        | Mt.MUnionMonomorph (id, cases) -> Hashtbl.replace union_layouts id cases
        | Mt.MRecord (_, _, _) -> () (* FIXME: handle global records if needed *)
        | Mt.MUnion (_, _, _) -> ()
        | _ -> ()
      ) decls;
      (* Second pass: convert functions *)
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
