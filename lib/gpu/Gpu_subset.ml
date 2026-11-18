(*
   Closed GPU scalar subset (PLAN_gpu_kernels.md G03) and helper/intrinsic
   resolution (G04).

   Input is the *monomorphic typed AST* produced by the checked frontend
   (`Compiler.prepare_module`), so every call has already been resolved to a
   declaration/method ID and every type to a monomorphic type. The pass is a
   whitelist over that AST: an unsupported constructor is refused with the
   enclosing function and construct named. There is no wildcard admission and
   no default expression.

   Admitted v1 closure:
     * Nat32 (u32) and Boolean values, immutable locals, literals;
     * local monomorphic helper functions in the same user module, acyclic;
     * reachable same-module Nat32/Boolean constants;
     * comparisons, Boolean conjunction/disjunction/negation, conditionals;
     * exactly the canonical Nat32 modular/bitwise instance methods resolved
       from the compiled Austral.Pervasive module (Gpu_intrinsics).

   Everything else — loops, mutation, borrows, pointers, spans, records,
   unions, casts, floats, 64-bit values, foreign calls, arbitrary @embed,
   function values, cross-module helpers and recursion — is refused.
*)

module Mt = Stages.Mtast

open Common
open MonoType

exception Refused of string

let refuse fmt = Printf.ksprintf (fun s -> raise (Refused s)) fmt

(* -------------------------------------------------------------------------- *)
(* Types                                                                       *)
(* -------------------------------------------------------------------------- *)

let is_nat32 (ty: mono_ty): bool =
  match ty with
  | MonoInteger (Unsigned, Width32) -> true
  | _ -> false

let is_boolean (ty: mono_ty): bool =
  match ty with MonoBoolean -> true | _ -> false

let supported_value_type (ty: mono_ty): bool =
  is_nat32 ty || is_boolean ty

let type_name (ty: mono_ty): string =
  match ty with
  | MonoInteger (Unsigned, Width32) -> "Nat32"
  | MonoBoolean -> "Boolean"
  | MonoInteger _ -> "an integer other than Nat32"
  | MonoDoubleFloat | MonoSingleFloat -> "a float"
  | MonoUnit -> "Unit"
  | MonoPointer _ -> "a pointer"
  | MonoAddress _ -> "an address"
  | MonoReadRef _ | MonoWriteRef _ -> "a reference"
  | MonoSpan _ | MonoSpanMut _ -> "a span"
  | MonoFnPtr _ -> "a function pointer"
  | MonoNamedType _ -> "a named record/union type"
  | MonoRegionTy _ | MonoRegionTyVar _ -> "a region"

(* -------------------------------------------------------------------------- *)
(* Declarations                                                                *)
(* -------------------------------------------------------------------------- *)

type fn_info = {
  fn_name: string;
  fn_params: (string * mono_ty) list;
  fn_ret: mono_ty;
  fn_body: Mt.mstmt;
}

type const_info = {
  const_name: string;
  const_ty: mono_ty;
  const_body: Mt.mexpr;
}

type decls = {
  functions: (Id.decl_id * fn_info) list;
  constants: (string * const_info) list;
}

let decls_of_module (Mt.MonoModule (_, decls)): decls =
  let functions =
    List.filter_map
      (function
        | Mt.MFunction (id, name, params, rt, body) ->
           Some (id,
                 { fn_name = Identifier.ident_string name;
                   fn_params =
                     List.map
                       (fun (Mt.MValueParameter (pid, ty)) ->
                         (Identifier.ident_string pid, ty))
                       params;
                   fn_ret = rt;
                   fn_body = body })
        | _ -> None)
      decls
  in
  let constants =
    List.filter_map
      (function
        | Mt.MConstant (_, name, ty, body) ->
           Some (Identifier.ident_string name,
                 { const_name = Identifier.ident_string name;
                   const_ty = ty;
                   const_body = body })
        | _ -> None)
      decls
  in
  { functions; constants }

let find_function (d: decls) (id: Id.decl_id): fn_info option =
  List.assoc_opt id d.functions

let find_constant (d: decls) (name: string): const_info option =
  List.assoc_opt name d.constants

(* -------------------------------------------------------------------------- *)
(* Translation context                                                         *)
(* -------------------------------------------------------------------------- *)

type binding = {
  b_name: string;
  b_local: int;
  b_ty: mono_ty;
  b_assigned: bool;
  b_is_param: bool;
  b_param_index: int;
}

type ctx = {
  module_name: string;
  intrinsics: Gpu_intrinsics.t;
  decls: decls;
  locals: binding list;
  ret_ty: mono_ty;
  next_local: int ref;
  next_helper: int ref;
  helper_ids: (Id.decl_id, int) Hashtbl.t;
  helper_defs: Gpu_ir.helper list ref;
  visiting: Id.decl_id list;
  depth: int;
  budget: int ref;
}

let tick (ctx: ctx) (what: string): unit =
  incr ctx.budget;
  if !(ctx.budget) > 100000 then
    refuse "%s: KernelIR node budget (100000) exceeded" what

let lookup_local (ctx: ctx) (name: string): binding option =
  List.find_opt (fun b -> b.b_name = name) ctx.locals

let lookup_param (ctx: ctx) (name: string): binding option =
  List.find_opt (fun b -> b.b_name = name && b.b_is_param) ctx.locals

let bind_local (ctx: ctx) (name: string) (ty: mono_ty): binding * ctx =
  let id = !(ctx.next_local) in
  ctx.next_local := id + 1;
  let b = { b_name = name; b_local = id; b_ty = ty;
            b_assigned = false; b_is_param = false; b_param_index = -1 } in
  (b, { ctx with locals = b :: ctx.locals })

let mark_assigned (ctx: ctx) (b: binding): ctx =
  let b' = { b with b_assigned = true } in
  { ctx with locals = b' :: List.filter (fun x -> x.b_local <> b.b_local) ctx.locals }

(* -------------------------------------------------------------------------- *)
(* Helper functions                                                            *)
(* -------------------------------------------------------------------------- *)

let rec ensure_helper (ctx: ctx) (decl_id: Id.decl_id) (call_site: string): int =
  match Hashtbl.find_opt ctx.helper_ids decl_id with
  | Some id -> id
  | None ->
     if ctx.depth >= 128 then
       refuse "%s: helper call depth exceeds 128" call_site;
     if List.mem decl_id ctx.visiting then
       refuse "%s: recursive call cycle in the helper closure" call_site;
     if Hashtbl.length ctx.helper_ids >= 128 then
       refuse "%s: more than 128 reachable helpers" call_site;
     let fn =
       match find_function ctx.decls decl_id with
       | Some fn -> fn
       | None ->
          refuse
            "%s: call to a function that is not a local helper of module %s \
             (foreign or cross-module calls are not part of v1)"
            call_site ctx.module_name
     in
     if not (supported_value_type fn.fn_ret) then
       refuse "%s: helper %s returns %s, which v1 does not admit"
         call_site fn.fn_name (type_name fn.fn_ret);
     let id = !(ctx.next_helper) in
     ctx.next_helper := id + 1;
     Hashtbl.replace ctx.helper_ids decl_id id;
     let params =
       List.mapi
         (fun i (name, ty) ->
           if not (supported_value_type ty) then
             refuse "%s: helper %s parameter %s has type %s, which v1 does not admit"
               call_site fn.fn_name name (type_name ty);
           { b_name = name; b_local = -1; b_ty = ty;
             b_assigned = true; b_is_param = true; b_param_index = i })
         fn.fn_params
     in
     let hctx =
       { ctx with locals = params;
                  visiting = decl_id :: ctx.visiting;
                  depth = ctx.depth + 1 }
     in
     let (body, _) = translate_stmt hctx fn.fn_body in
     let param_types =
       List.map
         (fun b -> if is_boolean b.b_ty then Gpu_ir.TyBool else Gpu_ir.TyU32)
         params
     in
     let ret = if is_boolean fn.fn_ret then Gpu_ir.TyBool else Gpu_ir.TyU32 in
     ctx.helper_defs :=
       !(ctx.helper_defs) @ [ { Gpu_ir.id; params = param_types; ret; body } ];
     id

(* -------------------------------------------------------------------------- *)
(* Expressions                                                                 *)
(* -------------------------------------------------------------------------- *)

and translate_expr (ctx: ctx) (expected: mono_ty option) (e: Mt.mexpr): Gpu_ir.expr =
  tick ctx "expression";
  match e with
  | Mt.MBoolConstant b -> Gpu_ir.Bool b
  | Mt.MIntConstant s ->
     (match expected with
      | Some ty when is_nat32 ty -> Gpu_ir.U32 (Gpu_ir.u32_of_string s)
      | _ ->
         refuse
           "integer literal %s is not used at Nat32 type; v1 admits only \
            Nat32 and Boolean scalar values" s)
  | Mt.MFloatConstant _ -> refuse "float literals are not part of v1"
  | Mt.MStringConstant _ -> refuse "string literals are not part of v1"
  | Mt.MNilConstant -> refuse "the Unit/nil constant is not part of v1"
  | Mt.MParamVar (name, ty) ->
     let n = Identifier.ident_string name in
     check_value_type "parameter" n ty expected;
     (match lookup_param ctx n with
      | Some b -> Gpu_ir.Param b.b_param_index
      | None -> refuse "unknown parameter %s" n)
  | Mt.MLocalVar (name, ty) ->
     let n = Identifier.ident_string name in
     check_value_type "local" n ty expected;
     (match lookup_local ctx n with
      | Some b when b.b_is_param -> Gpu_ir.Param b.b_param_index
      | Some b when b.b_assigned -> Gpu_ir.Local b.b_local
      | Some _ -> refuse "local %s is read before its initial assignment" n
      | None -> refuse "reference to unknown local %s" n)
  | Mt.MTemporary (name, ty) ->
     let n = Identifier.ident_string name in
     check_value_type "temporary" n ty expected;
     (match lookup_local ctx n with
      | Some b when b.b_assigned -> Gpu_ir.Local b.b_local
      | Some _ -> refuse "temporary %s is read before assignment" n
      | None -> refuse "reference to unknown temporary %s" n)
  | Mt.MConstVar (q, _ty) ->
     let name = Identifier.ident_string (Identifier.original_name q) in
     let source_module = Identifier.source_module_name q in
     if Identifier.mod_name_string source_module <> ctx.module_name then
       refuse "constant %s comes from %s; v1 admits only same-module constants"
         name (Identifier.mod_name_string source_module);
     (match find_constant ctx.decls name with
      | None -> refuse "unknown constant %s" name
      | Some c ->
         if not (supported_value_type c.const_ty) then
           refuse "constant %s has type %s, which v1 does not admit"
             name (type_name c.const_ty);
         translate_const ctx c)
  | Mt.MComparison (op, lhs, rhs) ->
     let a = translate_expr ctx (Some (nat32 ())) lhs
     and b = translate_expr ctx (Some (nat32 ())) rhs in
     Gpu_ir.Compare (compare_of_comparison op, a, b)
  | Mt.MConjunction (a, b) ->
     Gpu_ir.BooleanOp
       (Gpu_ir.Conjunction,
        translate_expr ctx (Some MonoBoolean) a,
        translate_expr ctx (Some MonoBoolean) b)
  | Mt.MDisjunction (a, b) ->
     Gpu_ir.BooleanOp
       (Gpu_ir.Disjunction,
        translate_expr ctx (Some MonoBoolean) a,
        translate_expr ctx (Some MonoBoolean) b)
  | Mt.MNegation a ->
     Gpu_ir.Not (translate_expr ctx (Some MonoBoolean) a)
  | Mt.MIfExpression (c, t, f) ->
     (match expected with
      | None -> refuse "conditional expression without a known result type"
      | Some ty ->
         Gpu_ir.Conditional
           (translate_expr ctx (Some MonoBoolean) c,
            translate_expr ctx (Some ty) t,
            translate_expr ctx (Some ty) f))
  | Mt.MConcreteFuncall (decl_id, q, args, _rt) ->
     let name = Identifier.ident_string (Identifier.original_name q) in
     let source_module = Identifier.source_module_name q in
     if Identifier.mod_name_string source_module <> ctx.module_name then
       refuse
         "call to %s from module %s: v1 admits only calls to local helpers"
         name (Identifier.mod_name_string source_module);
     (match find_function ctx.decls decl_id with
      | None -> refuse "call to %s: not a local function of module %s" name ctx.module_name
      | Some fn ->
         if not (supported_value_type fn.fn_ret) then
           refuse "call to %s returns %s, which v1 does not admit"
             name (type_name fn.fn_ret);
         let translated = translate_args ctx ("call to " ^ name) fn.fn_params args in
         let id = ensure_helper ctx decl_id ("call to " ^ name) in
         Gpu_ir.Call (id, translated))
  | Mt.MGenericFuncall (_mono_id, _args, _rt) ->
     refuse "generic function calls are not part of v1 (helpers must be monomorphic)"
  | Mt.MConcreteMethodCall (method_id, q, args, rt) ->
     let name = Identifier.ident_string (Identifier.original_name q) in
     (match Gpu_intrinsics.classify ctx.intrinsics method_id with
      | Some intrinsic ->
         if not (is_nat32 rt) then
           refuse "intrinsic %s does not return Nat32" name;
         let arity = Gpu_ir.intrinsic_arity intrinsic in
         if List.length args <> arity then
           refuse "intrinsic %s expects %d argument(s), got %d"
             name arity (List.length args);
         let translated =
           List.map (translate_expr ctx (Some (nat32 ()))) args
         in
         Gpu_ir.Intrinsic (intrinsic, translated)
      | None ->
         refuse
           "call to method %s: only the canonical Nat32 modularAdd, \
            modularSubtract, modularMultiply and bitwise instance methods are \
            admitted (resolved from Austral.Pervasive)" name)
  | Mt.MGenericMethodCall (_, _, _, _) ->
     refuse "generic method calls are not part of v1"
  | Mt.MFptrCall _ -> refuse "function-pointer calls are not part of v1"
  | Mt.MTypecast (inner, ty) ->
     (* Compiler-inserted type ascription from `let x: T := e` (AbstractionPass
        wraps that initializer). It is not a user cast: it only reasserts the
        already-checked type, so admit it for supported types. *)
     if not (supported_value_type ty) then
       refuse "type ascription to %s is not part of v1" (type_name ty);
     translate_expr ctx (Some ty) inner
  | Mt.MCast (inner, ty) ->
     (* The typed frontend also wraps integer literals and call arguments in a
        `TCast` to unify their numeric type. Such a cast is admitted only when
        its target is Nat32 or Boolean — all narrower/wider/float targets stay
        refused, so Int32/Int64/Float casts never reach the emitter. *)
     if not (supported_value_type ty) then
       refuse "cast to %s is not part of v1" (type_name ty);
     translate_expr ctx (Some ty) inner
  | Mt.MConcreteFunVar _ | Mt.MGenericFunVar _ ->
     refuse "function values and dynamic dispatch are not part of v1"
  | Mt.MRecordConstructor _ -> refuse "record construction is not part of v1"
  | Mt.MUnionConstructor _ -> refuse "union construction is not part of v1"
  | Mt.MSlotAccessor _ | Mt.MPointerSlotAccessor _ ->
     refuse "record slot access is not part of v1"
  | Mt.MArrayIndex _ | Mt.MSpanIndex _ ->
     refuse "array/span indexing is not part of v1 (the map wrapper owns all storage accesses)"
  | Mt.MEmbed _ ->
     refuse "arbitrary @embed is not part of v1 (only audited builtin intrinsics are)"
  | Mt.MDeref _ -> refuse "pointer dereference is not part of v1"
  | Mt.MSizeOf _ -> refuse "sizeOf is not part of v1"

and check_value_type (what: string) (name: string) (ty: mono_ty)
    (expected: mono_ty option): unit =
  if not (supported_value_type ty) then
    refuse "%s %s has type %s, which v1 does not admit"
      what name (type_name ty);
  (match expected with
   | Some e when is_nat32 e && not (is_nat32 ty) ->
      refuse "%s %s is not Nat32 in a Nat32 context" what name
   | Some e when is_boolean e && not (is_boolean ty) ->
      refuse "%s %s is not Boolean in a Boolean context" what name
   | _ -> ())

and translate_args (ctx: ctx) (where: string) (params: (string * mono_ty) list)
    (args: Mt.mexpr list): Gpu_ir.expr list =
  if List.length params <> List.length args then
    refuse "%s: expected %d argument(s), got %d"
      where (List.length params) (List.length args);
  List.map2
    (fun (pname, ty) arg ->
      if not (supported_value_type ty) then
        refuse "%s: parameter %s has type %s, which v1 does not admit"
          where pname (type_name ty);
      translate_expr ctx (Some ty) arg)
    params args

and translate_const (ctx: ctx) (c: const_info): Gpu_ir.expr =
  (* Constants are evaluated under the same rules. There is no constant cache
     in v1; the node budget bounds inlining. Recursion is impossible in the
     linear source but guarded here as well. *)
  if not (supported_value_type c.const_ty) then
    refuse "constant %s has type %s, which v1 does not admit"
      c.const_name (type_name c.const_ty);
  translate_expr { ctx with locals = [] } (Some c.const_ty) c.const_body

and nat32 (): mono_ty = MonoInteger (Unsigned, Width32)

and compare_of_comparison: comparison_operator -> Gpu_ir.comparison = function
  | Equal -> Gpu_ir.Eq
  | NotEqual -> Gpu_ir.NotEqual
  | LessThan -> Gpu_ir.LessThan
  | LessThanOrEqual -> Gpu_ir.LessThanOrEqual
  | GreaterThan -> Gpu_ir.GreaterThan
  | GreaterThanOrEqual -> Gpu_ir.GreaterThanOrEqual

(* -------------------------------------------------------------------------- *)
(* Statements                                                                  *)
(* -------------------------------------------------------------------------- *)

and translate_stmt (ctx: ctx) (s: Mt.mstmt): Gpu_ir.stmt * ctx =
  tick ctx "statement";
  match s with
  | Mt.MSkip -> (Gpu_ir.Seq [], ctx)
  | Mt.MLet (name, ty, body) ->
     let n = Identifier.ident_string name in
     if not (supported_value_type ty) then
       refuse "binding %s has type %s, which v1 does not admit"
         n (type_name ty);
     let (_b, ctx') = bind_local ctx n ty in
     (* The binding is visible only inside its body; an initial assignment
        inside that body updates the context for the following statements. *)
     let (body', _ctx_after) = translate_stmt ctx' body in
     (body', ctx)
  | Mt.MInitialAssign (q, value) ->
     let n = Identifier.ident_string (Identifier.original_name q) in
     (match lookup_local ctx n with
      | None ->
         refuse "initial assignment to unknown local %s" n
      | Some b when b.b_is_param ->
         refuse "assignment to parameter %s is not part of v1" n
      | Some b when b.b_assigned ->
         refuse "local %s is assigned more than once (mutation is not part of v1)" n
      | Some b ->
         let e = translate_expr ctx (Some b.b_ty) value in
         (Gpu_ir.Let (b.b_local, e), mark_assigned ctx b))
  | Mt.MAssignVar (q, _) ->
     refuse
       "mutable assignment to %s is not part of v1 (only immutable bindings)"
       (Identifier.ident_string (Identifier.original_name q))
  | Mt.MAssign _ ->
     refuse "assignment/pointer store is not part of v1"
  | Mt.MBlock (a, b) ->
     let (a', ctx') = translate_stmt ctx a in
     let (b', _) = translate_stmt ctx' b in
     (Gpu_ir.Seq [a'; b'], ctx)
  | Mt.MIf (c, t, f) ->
     let (t', _) = translate_stmt ctx t
     and (f', _) = translate_stmt ctx f in
     (Gpu_ir.If (translate_expr ctx (Some MonoBoolean) c, t', f'), ctx)
  | Mt.MReturn e ->
     (Gpu_ir.Return (translate_expr ctx (Some ctx.ret_ty) e), ctx)
  | Mt.MDestructure _ ->
     refuse "destructuring bindings are not part of v1"
  | Mt.MCase _ -> refuse "match/case is not part of v1"
  | Mt.MWhile _ -> refuse "while loops are not part of v1"
  | Mt.MFor _ -> refuse "for loops are not part of v1"
  | Mt.MBorrow _ -> refuse "borrows and mutable borrows are not part of v1"
  | Mt.MDiscarding _ -> refuse "discarded expressions are not part of v1"
  | Mt.MLetTmp (name, ty, value) ->
     (* A compiler-generated initialization temporary: a one-time binding of a
        supported value. It carries no continuation of its own. *)
     let n = Identifier.ident_string name in
     if not (supported_value_type ty) then
       refuse "compiler temporary %s has type %s, which v1 does not admit"
         n (type_name ty);
     let (b, ctx') = bind_local ctx n ty in
     let e = translate_expr ctx (Some ty) value in
     (Gpu_ir.Let (b.b_local, e), mark_assigned ctx' b)
  | Mt.MAssignTmp (name, value) ->
     let n = Identifier.ident_string name in
     (match lookup_local ctx n with
      | Some b when not b.b_assigned ->
         let e = translate_expr ctx (Some b.b_ty) value in
         (Gpu_ir.Let (b.b_local, e), mark_assigned ctx b)
      | Some _ ->
         refuse "compiler temporary %s is assigned more than once" n
      | None -> refuse "assignment to unknown compiler temporary %s" n)

(* -------------------------------------------------------------------------- *)
(* Entry point                                                                 *)
(* -------------------------------------------------------------------------- *)

let build ~(entry: string) (env: Env.env) (mono: Mt.mono_module): Gpu_ir.kernel =
  let Mt.MonoModule (module_name, _) = mono in
  let module_str = Identifier.mod_name_string module_name in
  let (d: decls) = decls_of_module mono in
  let (entry_id, fn) =
    match List.find_opt (fun (_, f) -> f.fn_name = entry) d.functions with
    | Some pair -> pair
    | None ->
       refuse "entry function %s is not a monomorphic function of module %s \
               (generic entries and other modules are not part of v1)"
         entry module_str
  in
  let arity = List.length fn.fn_params in
  if arity <> 1 && arity <> 2 then
    refuse "entry %s takes %d parameter(s); v1 requires one or two Nat32 inputs"
      entry arity;
  List.iter
    (fun (name, ty) ->
      if not (is_nat32 ty) then
        refuse "entry %s parameter %s has type %s; v1 requires Nat32"
          entry name (type_name ty))
    fn.fn_params;
  if not (is_nat32 fn.fn_ret) then
    refuse "entry %s returns %s; v1 requires a Nat32 result"
      entry (type_name fn.fn_ret);
  let params =
    List.mapi
      (fun i (name, ty) ->
        { b_name = name; b_local = -1; b_ty = ty;
          b_assigned = true; b_is_param = true; b_param_index = i })
      fn.fn_params
  in
  let ctx = {
    module_name = module_str;
    intrinsics = Gpu_intrinsics.table_of_env env;
    decls = d;
    locals = params;
    ret_ty = fn.fn_ret;
    next_local = ref 0;
    next_helper = ref 0;
    helper_ids = Hashtbl.create 16;
    helper_defs = ref [];
    visiting = [entry_id];
    depth = 0;
    budget = ref 0;
  } in
  let (body, _) = translate_stmt ctx fn.fn_body in
  let kernel = {
    Gpu_ir.entry_name = entry;
    input_arity = arity;
    helpers = !(ctx.helper_defs);
    body;
    memory = Gpu_ir.default_memory_plan arity;
  } in
  Gpu_ir.check_kernel kernel;
  kernel
