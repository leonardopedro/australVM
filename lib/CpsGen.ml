(* CpsGen.ml
   Convert Austral Monomorphic AST to Binary CPS IR
   
   IR Format (as understood by cranelift/src/cps.rs):
   [magic: u32 = 0x43505331][functions: u32]
   For each function:
     [name_len][name][params][return_type][body_len][body...]
   
   Instructions:
   0x01: IntLit(value: i64)
   0x02: Var(name: string)
   0x03: Let(name, value, body)
   0x04: App(func, args...)
   0x05: Add(a, b)
   0x06: Sub(a, b)
   0x07: Return(value)
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

let write_bytes w b =
  Buffer.add_bytes w.buf b

let to_bytes w = Buffer.to_bytes w.buf

let magic_number = 0x43505331l

(*************************************************************************
 * CPS IR Type Definitions
 *************************************************************************)

type cps_type =
  | Unit
  | I64
  | I32
  | Bool
  | String
  | F64

type cps_expr =
  | IntLit of int64
  | BoolLit of bool
  | FloatLit of float
  | StringLit of string
  | Var of string
  | App of string * cps_expr list
  | Deref of cps_expr
  | CmpLt of cps_expr * cps_expr
  | CmpGt of cps_expr * cps_expr
  | CmpLte of cps_expr * cps_expr
  | CmpGte of cps_expr * cps_expr
  | CmpEq of cps_expr * cps_expr
  | CmpNeq of cps_expr * cps_expr
  | And of cps_expr * cps_expr
  | Or of cps_expr * cps_expr
  | Add of cps_expr * cps_expr
  | Sub of cps_expr * cps_expr
  | Mul of cps_expr * cps_expr
  | Not of cps_expr

and cps_stmt =
  | Skip
  | Let of string * cps_expr * cps_stmt
  | Assign of string * cps_expr
  | Store of cps_expr * cps_expr
  | If of cps_expr * cps_stmt * cps_stmt
  | While of cps_expr * cps_stmt
  | Match of cps_expr * (int64 * cps_stmt) list * cps_stmt
  | Block of cps_stmt * cps_stmt
  | Discard of cps_expr
  | Return of cps_expr

type function_def = {
  name: string;
  params: string list;
  return_type: cps_type;
  body: cps_stmt;
}

(*************************************************************************
 * CPS IR Pretty Printing
 *************************************************************************)

let rec string_of_expr = function
  | IntLit n -> Int64.to_string n
  | BoolLit b -> string_of_bool b
  | FloatLit f -> string_of_float f
  | StringLit s -> "\"" ^ s ^ "\""
  | Var name -> name
  | App (f, args) -> f ^ "(" ^ String.concat ", " (List.map string_of_expr args) ^ ")"
  | Deref e -> "*(" ^ string_of_expr e ^ ")"
  | CmpLt (a, b) -> "(" ^ string_of_expr a ^ " < " ^ string_of_expr b ^ ")"
  | CmpGt (a, b) -> "(" ^ string_of_expr a ^ " > " ^ string_of_expr b ^ ")"
  | CmpLte (a, b) -> "(" ^ string_of_expr a ^ " <= " ^ string_of_expr b ^ ")"
  | CmpGte (a, b) -> "(" ^ string_of_expr a ^ " >= " ^ string_of_expr b ^ ")"
  | CmpEq (a, b) -> "(" ^ string_of_expr a ^ " == " ^ string_of_expr b ^ ")"
  | CmpNeq (a, b) -> "(" ^ string_of_expr a ^ " != " ^ string_of_expr b ^ ")"
  | And (a, b) -> "(" ^ string_of_expr a ^ " && " ^ string_of_expr b ^ ")"
  | Or (a, b) -> "(" ^ string_of_expr a ^ " || " ^ string_of_expr b ^ ")"
  | Add (a, b) -> "(" ^ string_of_expr a ^ " + " ^ string_of_expr b ^ ")"
  | Sub (a, b) -> "(" ^ string_of_expr a ^ " - " ^ string_of_expr b ^ ")"
  | Mul (a, b) -> "(" ^ string_of_expr a ^ " * " ^ string_of_expr b ^ ")"
  | Not e -> "!" ^ string_of_expr e

and string_of_stmt = function
  | Skip -> "skip"
  | Let (n, v, body) -> "let " ^ n ^ " = " ^ string_of_expr v ^ "; " ^ string_of_stmt body
  | Assign (n, v) -> n ^ " = " ^ string_of_expr v
  | Store (ptr, v) -> "*(" ^ string_of_expr ptr ^ ") = " ^ string_of_expr v
  | If (c, t, f) -> "if " ^ string_of_expr c ^ " { " ^ string_of_stmt t ^ " } else { " ^ string_of_stmt f ^ " }"
  | While (c, b) -> "while " ^ string_of_expr c ^ " { " ^ string_of_stmt b ^ " }"
  | Match (c, _, _) -> "match " ^ string_of_expr c ^ " { ... }"
  | Block (a, b) -> string_of_stmt a ^ "; " ^ string_of_stmt b
  | Discard e -> "discard " ^ string_of_expr e
  | Return e -> "return " ^ string_of_expr e

(*************************************************************************
 * CPS IR → Binary Serialization (typed CPS IR → binary)
 *************************************************************************)

let rec serialize_cps_expr w expr =
  match expr with
  | IntLit n ->
      write_u8 w 0x01;
      write_i64 w n
  | BoolLit b ->
      write_u8 w 0x01;
      write_i64 w (if b then 1L else 0L)
  | FloatLit f ->
      write_u8 w 0x01;
      write_i64 w (Int64.of_float f)
  | StringLit _ ->
      write_u8 w 0x01;
      write_i64 w 0L
  | Var name ->
      write_u8 w 0x02;
      write_string w name
  | App (fname, args) ->
      write_u8 w 0x04;
      write_string w fname;
      write_u32 w (Int32.of_int (List.length args));
      List.iter (serialize_cps_expr w) args
  | Deref e ->
      write_u8 w 0x20;
      serialize_cps_expr w e
  | CmpLt (a, b) ->
      write_u8 w 0x10;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | CmpGt (a, b) ->
      write_u8 w 0x11;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | CmpLte (a, b) ->
      write_u8 w 0x12;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | CmpGte (a, b) ->
      write_u8 w 0x13;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | CmpEq (a, b) ->
      write_u8 w 0x14;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | CmpNeq (a, b) ->
      write_u8 w 0x15;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | And (a, b) ->
      write_u8 w 0x16;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | Or (a, b) ->
      write_u8 w 0x17;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | Add (a, b) ->
      write_u8 w 0x05;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | Sub (a, b) ->
      write_u8 w 0x06;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | Mul (a, b) ->
      write_u8 w 0x18;
      serialize_cps_expr w a;
      serialize_cps_expr w b
  | Not e ->
      write_u8 w 0x19;
      serialize_cps_expr w e

and serialize_cps_stmt w = function
  | Skip -> ()
  | Let (name, value, body) ->
      write_u8 w 0x03;
      write_string w name;
      serialize_cps_expr w value;
      serialize_cps_stmt w body
  | Assign (name, value) ->
      write_u8 w 0x03;
      write_string w name;
      serialize_cps_expr w value
  | Store (ptr, value) ->
      write_u8 w 0x30;
      serialize_cps_expr w ptr;
      serialize_cps_expr w value
  | If (cond, then_branch, else_branch) ->
      write_u8 w 0x08;
      serialize_cps_expr w cond;
      let then_w = { buf = Buffer.create 64 } in
      serialize_cps_stmt then_w then_branch;
      let then_data = Buffer.to_bytes then_w.buf in
      write_u32 w (Int32.of_int (Bytes.length then_data));
      write_bytes w then_data;
      let else_w = { buf = Buffer.create 64 } in
      serialize_cps_stmt else_w else_branch;
      let else_data = Buffer.to_bytes else_w.buf in
      write_u32 w (Int32.of_int (Bytes.length else_data));
      write_bytes w else_data
  | While (cond, body) ->
      write_u8 w 0x09;
      serialize_cps_expr w cond;
      let body_w = { buf = Buffer.create 64 } in
      serialize_cps_stmt body_w body;
      let body_data = Buffer.to_bytes body_w.buf in
      write_u32 w (Int32.of_int (Bytes.length body_data));
      write_bytes w body_data
  | Match (cond, cases, default) ->
      write_u8 w 0x0A;
      serialize_cps_expr w cond;
      write_u32 w (Int32.of_int (List.length cases));
      List.iter (fun (val_, body) ->
        write_i64 w val_;
        let body_w = { buf = Buffer.create 64 } in
        serialize_cps_stmt body_w body;
        let body_data = Buffer.to_bytes body_w.buf in
        write_u32 w (Int32.of_int (Bytes.length body_data));
        write_bytes w body_data
      ) cases;
      let def_w = { buf = Buffer.create 64 } in
      serialize_cps_stmt def_w default;
      let def_data = Buffer.to_bytes def_w.buf in
      write_u32 w (Int32.of_int (Bytes.length def_data));
      write_bytes w def_data
  | Block (a, b) ->
      serialize_cps_stmt w a;
      serialize_cps_stmt w b
  | Discard e ->
      serialize_cps_expr w e
  | Return e ->
      write_u8 w 0x07;
      serialize_cps_expr w e

let serialize_function_def w func =
  write_string w func.name;
  write_u32 w (Int32.of_int (List.length func.params));
  List.iter (fun pname ->
    write_string w pname
  ) func.params;
  write_u8 w (match func.return_type with
    | I64 -> 0x01
    | I32 -> 0x03
    | Bool -> 0x02
    | Unit -> 0x00
    | String -> 0x04
    | F64 -> 0x05);
  let body_w = create_writer () in
  serialize_cps_stmt body_w func.body;
  let body_data = Buffer.to_bytes body_w.buf in
  let len = Bytes.length body_data in
  write_u32 w (Int32.of_int len);
  write_bytes w body_data

let serialize_functions (funcs: function_def list): string =
  let w = create_writer () in
  write_u32 w 0x43505331l;
  write_u32 w (Int32.of_int (List.length funcs));
  List.iter (serialize_function_def w) funcs;
  Bytes.to_string (Buffer.to_bytes w.buf)

(*************************************************************************
 * Direct MAST → Binary Compilation (legacy path)
 *************************************************************************)

let rec compile_expr w ctx expr =
  match expr with
  | MIntConstant s ->
      write_u8 w 0x01;
      let n = try Int64.of_string s with _ -> 0L in
      write_i64 w n

  | MBoolConstant b ->
      write_u8 w 0x01;
      write_i64 w (if b then 1L else 0L)

  | MLocalVar (id, _) ->
      write_u8 w 0x02;
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
      write_u8 w 0x08;
      compile_expr w ctx cond;
      compile_expr w ctx tbranch;
      compile_expr w ctx fbranch

  | MDeref expr ->
      write_u8 w 0x09;
      compile_expr w ctx expr

  | MSizeOf _ | MSlotAccessor _ | MPointerSlotAccessor _ 
  | MArrayIndex _ | MSpanIndex _ | MEmbed _ | MRecordConstructor _
  | MUnionConstructor _ | MGenericFunVar _ | MConcreteFunVar _
  | MTypecast _ | MConjunction _ | MDisjunction _ | MNegation _ ->
      write_u8 w 0x01;
      write_i64 w 0L

  | MConcreteFuncall (_, qname, args, _) ->
      compile_funcall w (qident_debug_name qname) args

  | MGenericFuncall (mono_id, args, _) ->
      let name = Printf.sprintf "func_%d" (Obj.magic mono_id) in
      compile_funcall w name args

  | MConcreteMethodCall _ | MGenericMethodCall _ | MFptrCall _
  | MNilConstant | MFloatConstant _ | MStringConstant _
  | MCast (_, _) ->
      write_u8 w 0x01;
      write_i64 w 0L

and compile_binop w op lhs rhs =
  let opcode = match op with
    | Equal -> 0x14
    | NotEqual -> 0x15
    | LessThan -> 0x10
    | LessThanOrEqual -> 0x12
    | GreaterThan -> 0x11
    | GreaterThanOrEqual -> 0x13
  in
  write_u8 w opcode;
  compile_expr w () lhs;
  compile_expr w () rhs

and compile_funcall w func_name args =
  write_u8 w 0x04;
  write_string w func_name;
  write_u32 w (Int32.of_int (List.length args));
  List.iter (compile_expr w ()) args

let rec compile_stmt w ctx stmt =
  match stmt with
  | MSkip -> ()

  | MLet (id, _, body) ->
      write_u8 w 0x03;
      write_string w (ident_string id);
      write_u8 w 0x01;
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
      write_u8 w 0x08;
      compile_stmt w ctx then_stmt;
      compile_stmt w ctx else_stmt

  | MCase (expr, whens, _) ->
      compile_expr w ctx expr;
      (match whens with
       | MTypedWhen (_, _, body) :: _ -> compile_stmt w ctx body
       | [] -> ())

  | MWhile (cond, body) ->
      compile_expr w ctx cond;
      write_u8 w 0x08;
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
      write_u8 w 0x07;
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
  | MFunction (_, name, params, ret_ty, body) ->
      write_string w (ident_string name);
      write_u32 w (Int32.of_int (List.length params));
      write_u8 w (match ret_ty with
        | MonoInteger _ -> 0x01
        | MonoBoolean -> 0x02
        | MonoUnit -> 0x00
        | _ -> 0x01);
      let temp_w = create_writer () in
      compile_stmt temp_w () body;
      let body_bytes = to_bytes temp_w in
      write_u32 w (Int32.of_int (Bytes.length body_bytes));
      Buffer.add_bytes w.buf body_bytes

  | MFunctionMonomorph (_, params, ret_ty, body) ->
      write_string w "monomorph";
      write_u32 w (Int32.of_int (List.length params));
      write_u8 w (match ret_ty with
        | MonoInteger _ -> 0x01
        | MonoBoolean -> 0x02
        | MonoUnit -> 0x00
        | _ -> 0x01);
      let temp_w = create_writer () in
      compile_stmt temp_w () body;
      let body_bytes = to_bytes temp_w in
      write_u32 w (Int32.of_int (Bytes.length body_bytes));
      Buffer.add_bytes w.buf body_bytes

  | _ -> ()

let compile_module _module_name decls =
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
  write_u8 w 0x01;
  let temp_w = create_writer () in
  compile_stmt temp_w () body;
  let body_bytes = to_bytes temp_w in
  write_u32 w (Int32.of_int (Bytes.length body_bytes));
  Buffer.add_bytes w.buf body_bytes;
  Some (to_bytes w)
