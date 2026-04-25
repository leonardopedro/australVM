(* CpsGen.ml
   Convert Austral Monomorphic AST to Binary CPS IR
   
   IR Format (as understood by cranelift/src/cps.rs):
   [magic: u32 = 0x43505331][functions: u32]
   For each function:
     [name_len][name][params][return_type][body_len][body...]
   
   Instructions (from cranelift/src/cps.rs):
   0x01: IntLit(value: i64)
   0x02: Var(name: string)
   0x03: Let(name, value, body)
   0x04: App(func, args...)
   0x05: Add(a, b)
   0x06: Sub(a, b)
   0x07: Return(value)
    -- Note: 0x08, 0x09, 0x0A are TODO in Rust bridge
*)

open Stages.Mtast
open MonoType
open Identifier

(*************************************************************************
 * Binary Writer
 *************************************************************************)

type writer = {
  buf: Buffer.t;
}

let create_writer () : writer = {
  buf = Buffer.create 1024;
}

let write_u8 w b =
  Buffer.add_char w.buf (Char.chr (b land 0xFF))

let write_u32 w n =
  let open Int32 in
  let b0 = to_int (logand n 0xFFl) in
  let b1 = to_int (logand (shift_right n 8) 0xFFl) in
  let b2 = to_int (logand (shift_right n 16) 0xFFl) in
  let b3 = to_int (logand (shift_right n 24) 0xFFl) in
  Buffer.add_char w.buf (Char.chr b0);
  Buffer.add_char w.buf (Char.chr b1);
  Buffer.add_char w.buf (Char.chr b2);
  Buffer.add_char w.buf (Char.chr b3)

let write_i64 w n =
  let open Int64 in
  for i = 0 to 7 do
    let byte = to_int (logand (shift_right n (i * 8)) 0xFFL) in
    Buffer.add_char w.buf (Char.chr byte)
  done

let write_string w s =
  let len = String.length s in
  write_u32 w (Int32.of_int len);
  Buffer.add_string w.buf s

let to_bytes w = Buffer.to_bytes w.buf

(*************************************************************************
 * CPS IR Generation
 *************************************************************************)

let magic_number = 0x43505331l  (* "CPS1" in little-endian *)

let rec compile_expr w ctx expr =
  match expr with
  | MIntConstant s ->
      write_u8 w 0x01;  (* IntLit *)
      let n = try Int64.of_string s with _ -> 0L in
      write_i64 w n

  | MBoolConstant b ->
      write_u8 w 0x01;
      write_i64 w (if b then 1L else 0L)

  | MLocalVar (id, _) ->
      write_u8 w 0x02;  (* Var *)
      write_string w (ident_string id)

  | MParamVar (id, _) ->
      write_u8 w 0x02;
      write_string w (ident_string id)

  | MTemporary (id, _) ->
      write_u8 w 0x02;
      write_string w (ident_string id)

  | MConstVar (qname, _) ->
      write_u8 w 0x02;
      write_string w (qident_debug_name qname)

  | MComparison (op, lhs, rhs) ->
      compile_binop w op lhs rhs

  | MIfExpression (cond, tbranch, fbranch) ->
      (* For MVP: compile condition and then branch, ignore else *)
      compile_expr w ctx cond;
      compile_expr w ctx tbranch

  | MDeref expr ->
      write_u8 w 0x09;  (* Deref - might not be implemented in Rust yet *)
      compile_expr w ctx expr

  | MSizeOf _ | MSlotAccessor _ | MPointerSlotAccessor _ 
  | MArrayIndex _ | MSpanIndex _ | MEmbed _ | MRecordConstructor _
  | MUnionConstructor _ | MGenericFunVar _ | MConcreteFunVar _
  | MTypecast _ | MConjunction _ | MDisjunction _ | MNegation _ ->
      (* Complex operations: treat as literal 0 for MVP *)
      write_u8 w 0x01;
      write_i64 w 0L

  | MConcreteFuncall (decl_id, qname, args, _) ->
      compile_funcall w (qident_debug_name qname) args

  | MGenericFuncall (mono_id, args, _) ->
      (* Use placeholder name *)
      let name = Printf.sprintf "func_%d" (Obj.magic mono_id) in
      compile_funcall w name args

  | MConcreteMethodCall _ | MGenericMethodCall _ | MFptrCall _ ->
      write_u8 w 0x01;
      write_i64 w 0L

and compile_binop w op lhs rhs =
  compile_expr w () lhs;
  compile_expr w () rhs;
  match op with
  | Equal | NotEqual | Less | LessEqual | Greater | GreaterEqual ->
      (* Comparisons: use Add as placeholder, will always be 0 or 1 *)
      write_u8 w 0x05

and compile_funcall w func_name args =
  write_u8 w 0x04;  (* App *)
  write_string w func_name;
  write_u32 w (Int32.of_int (List.length args));
  List.iter (compile_expr w ()) args

let rec compile_stmt w ctx stmt =
  match stmt with
  | MSkip -> ()

  | MLet (id, _, body) ->
      write_u8 w 0x03;  (* Let *)
      write_string w (ident_string id);
      write_u8 w 0x01;  (* value = 0, placeholder *)
      write_i64 w 0L;
      compile_stmt w ctx body

  | MAssign (_, value) ->
      compile_expr w ctx value

  | MAssignVar (_, value) ->
      compile_expr w ctx value

  | MInitialAssign (_, expr) ->
      compile_expr w ctx expr

  | MIf (cond, then_stmt, else_stmt) ->
      compile_expr w ctx cond;
      compile_stmt w ctx then_stmt;
      compile_stmt w ctx else_stmt

  | MCase (expr, whens, _) ->
      compile_expr w ctx expr;
      (match whens with
       | MTypedWhen (_, _, body) :: _ -> compile_stmt w ctx body
       | [] -> ())

  | MWhile (cond, body) ->
      compile_expr w ctx cond;
      compile_stmt w ctx body

  | MFor (_, start, end_, body) ->
      compile_expr w ctx start;
      compile_expr w ctx end_;
      compile_stmt w ctx body

  | MBorrow { body; _ } ->
      compile_stmt w ctx body

  | MBlock (s1, s2) ->
      compile_stmt w ctx s1;
      compile_stmt w ctx s2

  | MDiscarding expr ->
      compile_expr w ctx expr

  | MReturn expr ->
      write_u8 w 0x07;  (* Return *)
      compile_expr w ctx expr

  | MLetTmp (id, _, expr) ->
      write_u8 w 0x03;
      write_string w (ident_string id);
      compile_expr w ctx expr;
      write_u8 w 0x02;
      write_string w (ident_string id)

  | MAssignTmp (id, expr) ->
      write_u8 w 0x03;
      write_string w (ident_string id);
      compile_expr w ctx expr;
      write_u8 w 0x02;
      write_string w (ident_string id)

  | MDestructure (_, expr, body) ->
      compile_expr w ctx expr;
      compile_stmt w ctx body

let compile_function w mfunc =
  match mfunc with
  | MFunction (_, name, params, ret_ty, body)
  | MFunctionMonomorph (_, params, ret_ty, body) ->
      write_string w (ident_string name);
      write_u32 w (Int32.of_int (List.length params));
      write_u8 w (match ret_ty with
        | MonoInteger _ -> 0x01
        | MonoBoolean -> 0x02
        | MonoUnit -> 0x00
        | _ -> 0x01);
      let body_start = w.buf in
      let temp_w = create_writer () in
      compile_stmt temp_w () body;
      let body_bytes = to_bytes temp_w in
      write_u32 w (Int32.of_int (Bytes.length body_bytes));
      Buffer.add_bytes w.buf body_bytes

  | _ -> () (* Skip other decl types *)

let compile_module module_name decls =
  (* Filter to just functions *)
  let funcs = List.filter (function
    | MFunction _ | MFunctionMonomorph _ -> true
    | _ -> false
  ) decls in
  
  if funcs = [] then None
  else
    let w = create_writer () in
    write_u32 w magic_number;
    write_u32 w (Int32.of_int (List.length funcs));
    List.iter (compile_function w) funcs;
    Some (to_bytes w)

let compile_function_expr name params body =
  let w = create_writer () in
  write_u32 w magic_number;
  write_u32 w 1l;
  write_string w name;
  write_u32 w (Int32.of_int (List.length params));
  write_u8 w 0x01; (* Return i64 *)
  let temp_w = create_writer () in
  compile_stmt temp_w () body;
  let body_bytes = to_bytes temp_w in
  write_u32 w (Int32.of_int (Bytes.length body_bytes));
  Buffer.add_bytes w.buf body_bytes;
  Some (to_bytes w)
