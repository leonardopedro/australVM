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
   unions, casts, 64-bit values, foreign calls, arbitrary @embed, function
   values, cross-module helpers and recursion — is refused.

   Two numeric profiles exist. `u32-modular-v1` is the closure above.
   `wgsl-f32-v1` is the explicit opt-in: the same closed shape over Float32,
   admitting only the canonical `TrappingArithmetic(Float32)` add, subtract and
   multiply. Float64, division, conversions/comparisons on floats and every
   other float operation stay refused; the profile is chosen by the caller and
   is never inferred from a program.

   A separate *source* profile (G17a) admits the shape of one compile-time
   bounded `for` loop in the entry body, and G17b lowers it: `build` unrolls
   the loop into one immutable KernelIR local per body execution (KernelIR has
   no loop node) after checking the projected expansion against the node
   budget. `recognize_bounded_fold` is the probe of the same admission. Under
   the default source profile every loop is still refused.
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

(* `Index` is the loop-bound type. It is not a kernel value type; only the
   literal bounds LiftControlPass materialises as Index temporaries may appear,
   and they are resolved to their literal before the fold is admitted. *)
let is_index (ty: mono_ty): bool =
  match ty with
  | MonoInteger (Unsigned, WidthIndex) -> true
  | _ -> false

let index_type (): mono_ty = MonoInteger (Unsigned, WidthIndex)

let is_float32 (ty: mono_ty): bool =
  match ty with MonoSingleFloat -> true | _ -> false

let is_float64 (ty: mono_ty): bool =
  match ty with MonoDoubleFloat -> true | _ -> false

let float32 (): mono_ty = MonoSingleFloat

let supported_value_type ~(profile: Gpu_ir.numeric_profile) (ty: mono_ty): bool =
  match profile with
  | Gpu_ir.U32ModularV1 -> is_nat32 ty || is_boolean ty
  | Gpu_ir.WgslF32V1 -> is_float32 ty || is_boolean ty

let profile_scalar_type = function
  | Gpu_ir.U32ModularV1 -> MonoInteger (Unsigned, Width32)
  | Gpu_ir.WgslF32V1 -> MonoSingleFloat

let value_type_of ~(profile: Gpu_ir.numeric_profile) (ty: mono_ty): Gpu_ir.value_type =
  if is_boolean ty then Gpu_ir.TyBool
  else
    match profile with
    | Gpu_ir.U32ModularV1 -> Gpu_ir.TyU32
    | Gpu_ir.WgslF32V1 -> Gpu_ir.TyF32

let type_name (ty: mono_ty): string =
  match ty with
  | MonoInteger (Unsigned, Width32) -> "Nat32"
  | MonoBoolean -> "Boolean"
  | MonoInteger _ -> "an integer other than Nat32"
  | MonoSingleFloat -> "Float32"
  | MonoDoubleFloat -> "Float64"
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

(* The source profile is separate from the numeric profile: the numeric
   profile says which arithmetic a kernel may use, this one says whether one
   compile-time bounded loop may appear in the entry body. It is opt-in and is
   never inferred. *)
type source_profile =
  | StrictV1
  | BoundedFoldV1

let source_profile_name = function
  | StrictV1 -> "v1"
  | BoundedFoldV1 -> "bounded-fold-v1"

let source_profile_of_string = function
  | "bounded-fold-v1" -> BoundedFoldV1
  | s ->
     refuse "unknown GPU source profile %S; the only opt-in profile is \
             bounded-fold-v1" s

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
  profile: Gpu_ir.numeric_profile;
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
  (* G17b: the entry's source profile plus the per-build state a bounded fold
     needs. [fold_acc] records the entry's one mutable binding, [index_literals]
     resolves the bound temporaries `LiftControlPass` produced, and
     [fold_seen] makes a second loop in the entry a refusal. *)
  entry_name: string;
  source_profile: source_profile;
  fold_acc: (binding * Gpu_ir.expr) option ref;
  index_literals: (string, Z.t) Hashtbl.t;
  fold_seen: bool ref;
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
(* Bounded-fold expansion helpers (G17b)                                       *)
(* -------------------------------------------------------------------------- *)

(* Strip the compiler-inserted ascription/cast wrappers around a literal, so a
   bound is decided on the typed monomorphic AST and not on source text. *)
let rec unwrap_literal (e: Mt.mexpr): Mt.mexpr =
  match e with
  | Mt.MTypecast (inner, _) | Mt.MCast (inner, _) -> unwrap_literal inner
  | other -> other

(* `LiftControlPass` rewrites a loop's bounds into Index-typed compiler
   temporaries, so the bound is a `Temporary` node rooted at a literal.
   Resolve that chain here instead of reading source text: a temporary with no
   literal initializer is a runtime bound and is refused. *)
let bound_u32 ~(index_literals: (string, Z.t) Hashtbl.t) (what: string)
    (e: Mt.mexpr): Z.t =
  match unwrap_literal e with
  | Mt.MIntConstant s -> Z.of_string (Gpu_ir.u32_of_string s)
  | Mt.MTemporary (name, _) ->
     (match Hashtbl.find_opt index_literals (Identifier.ident_string name) with
      | Some v -> v
      | None ->
         refuse
           "%s is a runtime index; bounded-fold-v1 requires a compile-time \
            literal in 0..31 (a dynamic bound is not admitted)" what)
  | _ ->
     refuse
       "%s must be a compile-time integer literal in 0..31 (a dynamic bound is \
        not part of bounded-fold-v1)" what

(* Flatten the lifted body's block nesting into statement order. *)
let rec flatten_stmts (s: Mt.mstmt) (found: Mt.mstmt list): Mt.mstmt list =
  match s with
  | Mt.MSkip -> found
  | Mt.MBlock (a, b) -> flatten_stmts b (flatten_stmts a found)
  | other -> found @ [ other ]

(* The one value a bounded fold's body writes into its accumulator. The lifted
   body is a block of skips, an optional compiler temporary for the assigned
   value and the assignment itself; the temporary is the update expression, so
   no per-iteration temporary is emitted. *)
let fold_body_expression ~(entry: string) (acc: binding)
    (body: Mt.mstmt): Mt.mexpr =
  let assigned_name (q: Identifier.qident) =
    Identifier.ident_string (Identifier.original_name q)
  in
  let check (q: Identifier.qident) =
    if assigned_name q <> acc.b_name then
      refuse
        "the bounded loop in %s assigns %s; bounded-fold-v1 admits exactly \
         one accumulator (%s)" entry (assigned_name q) acc.b_name
  in
  match flatten_stmts body [] with
  | [ Mt.MAssignVar (q, value) ] -> check q; value
  | [ Mt.MLetTmp (name, _ty, value); Mt.MAssignVar (q, Mt.MTemporary (r, _)) ]
    when Identifier.ident_string name = Identifier.ident_string r ->
     check q; value
  | _ ->
     refuse
       "the bounded loop body in %s must be one assignment to the accumulator; \
        nested control flow, effects and a second write are not part of \
        bounded-fold-v1" entry

(* Sizes of the values G17b can emit, used only to project the unrolled
   expansion before building it. *)
let rec expr_nodes (e: Gpu_ir.expr): int =
  1 +
  match e with
  | Gpu_ir.Param _ | Gpu_ir.Local _ | Gpu_ir.U32 _ | Gpu_ir.F32 _
  | Gpu_ir.Bool _ -> 0
  | Gpu_ir.Intrinsic (_, args) | Gpu_ir.Call (_, args) ->
     List.fold_left (fun n a -> n + expr_nodes a) 0 args
  | Gpu_ir.Compare (_, a, b) | Gpu_ir.BooleanOp (_, a, b) ->
     expr_nodes a + expr_nodes b
  | Gpu_ir.Not a -> expr_nodes a
  | Gpu_ir.Conditional (c, t, f) -> expr_nodes c + expr_nodes t + expr_nodes f

let rec stmt_nodes (s: Gpu_ir.stmt): int =
  1 +
  match s with
  | Gpu_ir.Let (_, e) -> expr_nodes e
  | Gpu_ir.Seq items -> List.fold_left (fun n s -> n + stmt_nodes s) 0 items
  | Gpu_ir.If (c, t, f) -> expr_nodes c + stmt_nodes t + stmt_nodes f
  | Gpu_ir.Return e -> expr_nodes e

(* A call costs its callee's body as well: helper definitions are shared in the
   artifact, but every call re-evaluates one. *)
let rec call_weight (ctx: ctx) (e: Gpu_ir.expr): int =
  match e with
  | Gpu_ir.Call (id, args) ->
     let body =
       match List.find_opt (fun (h: Gpu_ir.helper) -> h.Gpu_ir.id = id)
               !(ctx.helper_defs) with
       | Some h -> stmt_nodes h.Gpu_ir.body
       | None -> 1
     in
     body + List.fold_left (fun n a -> n + call_weight ctx a) 0 args
  | Gpu_ir.Param _ | Gpu_ir.Local _ | Gpu_ir.U32 _ | Gpu_ir.F32 _
  | Gpu_ir.Bool _ -> 0
  | Gpu_ir.Intrinsic (_, args) ->
     List.fold_left (fun n a -> n + call_weight ctx a) 0 args
  | Gpu_ir.Compare (_, a, b) | Gpu_ir.BooleanOp (_, a, b) ->
     call_weight ctx a + call_weight ctx b
  | Gpu_ir.Not a -> call_weight ctx a
  | Gpu_ir.Conditional (c, t, f) ->
     call_weight ctx c + call_weight ctx t + call_weight ctx f

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
     if not (supported_value_type ~profile:ctx.profile fn.fn_ret) then
       refuse "%s: helper %s returns %s, which %s does not admit"
         call_site fn.fn_name (type_name fn.fn_ret)
         (Gpu_ir.profile_name ctx.profile);
     let id = !(ctx.next_helper) in
     ctx.next_helper := id + 1;
     Hashtbl.replace ctx.helper_ids decl_id id;
     let params =
       List.mapi
         (fun i (name, ty) ->
           if not (supported_value_type ~profile:ctx.profile ty) then
             refuse "%s: helper %s parameter %s has type %s, which %s does not admit"
               call_site fn.fn_name name (type_name ty)
               (Gpu_ir.profile_name ctx.profile);
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
       List.map (fun b -> value_type_of ~profile:ctx.profile b.b_ty) params
     in
     let ret = value_type_of ~profile:ctx.profile fn.fn_ret in
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
      | Some ty when is_nat32 ty && ctx.profile = Gpu_ir.U32ModularV1 ->
         Gpu_ir.U32 (Gpu_ir.u32_of_string s)
      | _ ->
         refuse
           "integer literal %s is not used at Nat32 type; %s admits \
            %s scalar values" s (Gpu_ir.profile_name ctx.profile)
           (match ctx.profile with
            | Gpu_ir.U32ModularV1 -> "Nat32 and Boolean"
            | Gpu_ir.WgslF32V1 -> "Float32 and Boolean"))
  | Mt.MFloatConstant s ->
     (match expected with
      | Some ty when is_float32 ty && ctx.profile = Gpu_ir.WgslF32V1 ->
         Gpu_ir.F32 (Gpu_ir.f32_of_string s)
      | Some ty when is_float32 ty ->
         refuse "Float32 literals require the wgsl-f32-v1 profile; %s admits \
                 no float values" (Gpu_ir.profile_name ctx.profile)
      | Some ty when is_float64 ty ->
         refuse "Float64 is not part of %s; the only float profile is \
                 wgsl-f32-v1, which admits Float32"
           (Gpu_ir.profile_name ctx.profile)
      | _ ->
         refuse "float literal %s is not used at Float32 type under %s"
           s (Gpu_ir.profile_name ctx.profile))
  | Mt.MStringConstant _ -> refuse "string literals are not part of v1"
  | Mt.MNilConstant -> refuse "the Unit/nil constant is not part of v1"
  | Mt.MParamVar (name, ty) ->
     let n = Identifier.ident_string name in
     check_value_type ~profile:ctx.profile "parameter" n ty expected;
     (match lookup_param ctx n with
      | Some b -> Gpu_ir.Param b.b_param_index
      | None -> refuse "unknown parameter %s" n)
  | Mt.MLocalVar (name, ty) ->
     let n = Identifier.ident_string name in
     check_value_type ~profile:ctx.profile "local" n ty expected;
     (match lookup_local ctx n with
      | Some b when b.b_is_param -> Gpu_ir.Param b.b_param_index
      | Some b when b.b_assigned -> Gpu_ir.Local b.b_local
      | Some _ -> refuse "local %s is read before its initial assignment" n
      | None -> refuse "reference to unknown local %s" n)
  | Mt.MTemporary (name, ty) ->
     let n = Identifier.ident_string name in
     check_value_type ~profile:ctx.profile "temporary" n ty expected;
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
         if not (supported_value_type ~profile:ctx.profile c.const_ty) then
           refuse "constant %s has type %s, which %s does not admit"
             name (type_name c.const_ty) (Gpu_ir.profile_name ctx.profile);
         translate_const ctx c)
  | Mt.MComparison (op, lhs, rhs) ->
     (match ctx.profile with
      | Gpu_ir.WgslF32V1 ->
         refuse "comparisons are not part of wgsl-f32-v1 (only add, subtract \
                 and multiply are admitted)"
      | Gpu_ir.U32ModularV1 ->
         let a = translate_expr ctx (Some (nat32 ())) lhs
         and b = translate_expr ctx (Some (nat32 ())) rhs in
         Gpu_ir.Compare (compare_of_comparison op, a, b))
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
         if not (supported_value_type ~profile:ctx.profile fn.fn_ret) then
           refuse "call to %s returns %s, which %s does not admit"
             name (type_name fn.fn_ret) (Gpu_ir.profile_name ctx.profile);
         let translated = translate_args ctx ("call to " ^ name) fn.fn_params args in
         let id = ensure_helper ctx decl_id ("call to " ^ name) in
         Gpu_ir.Call (id, translated))
  | Mt.MGenericFuncall (_mono_id, _args, _rt) ->
     refuse "generic function calls are not part of v1 (helpers must be monomorphic)"
  | Mt.MConcreteMethodCall (method_id, q, args, rt) ->
     let name = Identifier.ident_string (Identifier.original_name q) in
     (match Gpu_intrinsics.classify ctx.intrinsics method_id with
      | Some intrinsic ->
         let float_op = Gpu_ir.is_float_intrinsic intrinsic in
         (match (ctx.profile, float_op) with
          | Gpu_ir.WgslF32V1, true -> ()
          | Gpu_ir.U32ModularV1, false -> ()
          | Gpu_ir.U32ModularV1, true ->
             refuse "method %s operates on Float32; it requires the \
                     wgsl-f32-v1 profile" name
          | Gpu_ir.WgslF32V1, false ->
             refuse "method %s is integer arithmetic, which %s does not admit"
               name (Gpu_ir.profile_name ctx.profile));
         let scalar = profile_scalar_type ctx.profile in
         if rt <> scalar then
           refuse "intrinsic %s does not return %s" name (type_name scalar);
         let arity = Gpu_ir.intrinsic_arity intrinsic in
         if List.length args <> arity then
           refuse "intrinsic %s expects %d argument(s), got %d"
             name arity (List.length args);
         let translated =
           List.map (translate_expr ctx (Some scalar)) args
         in
         Gpu_ir.Intrinsic (intrinsic, translated)
      | None ->
         if Gpu_intrinsics.is_rejected ctx.intrinsics method_id then
           refuse
             "method %s is not part of %s: only add, subtract and multiply \
              are admitted (division has no GPU profile yet)"
             name (Gpu_ir.profile_name ctx.profile)
         else
           refuse
             "call to method %s: only the canonical Nat32 modularAdd, \
              modularSubtract, modularMultiply and bitwise instance methods \
              (u32-modular-v1) or the canonical Float32 trappingAdd, \
              trappingSubtract and trappingMultiply (wgsl-f32-v1) are \
              admitted (resolved from Austral.Pervasive)" name)
  | Mt.MGenericMethodCall (_, _, _, _) ->
     refuse "generic method calls are not part of v1"
  | Mt.MFptrCall _ -> refuse "function-pointer calls are not part of v1"
  | Mt.MTypecast (inner, ty) ->
     (* Compiler-inserted type ascription from `let x: T := e` (AbstractionPass
        wraps that initializer). It is not a user cast: it only reasserts the
        already-checked type, so admit it for supported types. *)
     if not (supported_value_type ~profile:ctx.profile ty) then
       refuse "type ascription to %s is not part of %s" (type_name ty)
         (Gpu_ir.profile_name ctx.profile);
     translate_expr ctx (Some ty) inner
  | Mt.MCast (inner, ty) ->
     (* The typed frontend also wraps integer literals and call arguments in a
        `TCast` to unify their numeric type. Such a cast is admitted only when
        its target is the profile's own scalar (or Boolean); narrower/wider and
        cross-profile targets stay refused, so an Int32/Int64 or float-to-int
        cast never reaches the emitter. *)
     if not (supported_value_type ~profile:ctx.profile ty) then
       refuse "cast to %s is not part of %s" (type_name ty)
         (Gpu_ir.profile_name ctx.profile);
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

and check_value_type ~(profile: Gpu_ir.numeric_profile)
    (what: string) (name: string) (ty: mono_ty)
    (expected: mono_ty option): unit =
  if not (supported_value_type ~profile ty) then
    refuse "%s %s has type %s, which %s does not admit"
      what name (type_name ty) (Gpu_ir.profile_name profile);
  (match expected with
   | Some e when is_nat32 e && not (is_nat32 ty) ->
      refuse "%s %s is not Nat32 in a Nat32 context" what name
   | Some e when is_boolean e && not (is_boolean ty) ->
      refuse "%s %s is not Boolean in a Boolean context" what name
   | Some e when is_float32 e && not (is_float32 ty) ->
      refuse "%s %s is not Float32 in a Float32 context" what name
   | _ -> ())

and translate_args (ctx: ctx) (where: string) (params: (string * mono_ty) list)
    (args: Mt.mexpr list): Gpu_ir.expr list =
  if List.length params <> List.length args then
    refuse "%s: expected %d argument(s), got %d"
      where (List.length params) (List.length args);
  List.map2
    (fun (pname, ty) arg ->
      if not (supported_value_type ~profile:ctx.profile ty) then
        refuse "%s: parameter %s has type %s, which %s does not admit"
          where pname (type_name ty) (Gpu_ir.profile_name ctx.profile);
      translate_expr ctx (Some ty) arg)
    params args

and translate_const (ctx: ctx) (c: const_info): Gpu_ir.expr =
  (* Constants are evaluated under the same rules. There is no constant cache
     in v1; the node budget bounds inlining. Recursion is impossible in the
     linear source but guarded here as well. *)
  if not (supported_value_type ~profile:ctx.profile c.const_ty) then
    refuse "constant %s has type %s, which %s does not admit"
      c.const_name (type_name c.const_ty) (Gpu_ir.profile_name ctx.profile);
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
     if not (supported_value_type ~profile:ctx.profile ty) then
       refuse "binding %s has type %s, which %s does not admit"
         n (type_name ty) (Gpu_ir.profile_name ctx.profile);
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
         (match !(ctx.fold_acc) with
          | Some (other, _) when other.b_local <> b.b_local ->
             refuse
               "bounded-fold-v1 admits exactly one private accumulator; %s \
                and %s are both initialized" other.b_name b.b_name
          | Some _ ->
             refuse "local %s is initialized more than once" b.b_name
          | None -> ());
         if ctx.source_profile = BoundedFoldV1 then
           ctx.fold_acc := Some (b, e);
         (Gpu_ir.Let (b.b_local, e), mark_assigned ctx b))
  | Mt.MAssignVar (q, _) ->
     refuse
       "mutable assignment to %s is not part of v1 (only immutable bindings)"
       (Identifier.ident_string (Identifier.original_name q))
  | Mt.MAssign _ ->
     refuse "assignment/pointer store is not part of v1"
  | Mt.MBlock (a, b) ->
     (* Statements are a flat sequence in KernelIR. Propagating the context
        after `b` is what lets a statement after a fold see the accumulator
        binding the expansion produced. *)
     let (a', ctx') = translate_stmt ctx a in
     let (b', ctx'') = translate_stmt ctx' b in
     (Gpu_ir.Seq [a'; b'], ctx'')
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
  | Mt.MFor (_id, start, finish, body) ->
     if ctx.source_profile = StrictV1 then
       refuse
         "for loops are not part of v1; the entry %s requires \
          --gpu-source-profile=bounded-fold-v1" ctx.entry_name;
     if ctx.depth > 0 then
       refuse
         "for loops are not part of v1 outside the entry body (bounded-fold-v1 \
          admits one loop in the entry, not in a helper)";
     if !(ctx.fold_seen) then
       refuse
         "bounded-fold-v1 admits one for loop in the entry body; a second (or \
          nested) loop is not admitted";
     ctx.fold_seen := true;
     let where = Printf.sprintf "entry %s loop" ctx.entry_name in
     let start_v = bound_u32 ~index_literals:ctx.index_literals (where ^ " start") start in
     if not (Z.equal start_v Z.zero) then
       refuse
         "bounded-fold-v1 admits `for i from 0 to K`; the entry's loop starts \
          at %s" (Z.to_string start_v);
     let k =
       let v = bound_u32 ~index_literals:ctx.index_literals (where ^ " bound") finish in
       if Z.compare v (Z.of_int 31) > 0 then
         refuse
           "the entry's literal loop bound %s is greater than 31; \
            bounded-fold-v1 admits 0..31" (Z.to_string v);
       Z.to_int v
     in
     let acc_b =
       match !(ctx.fold_acc) with
       | Some (b, _) -> b
       | None ->
          refuse
            "the entry's bounded loop needs an already initialized private \
             accumulator before it; none was found"
     in
     let value = fold_body_expression ~entry:ctx.entry_name acc_b body in
     (* Project the expansion before building it. One `Let` per body execution
        plus the shared initializer; a call also costs its callee's body, since
        the definition is shared but every iteration evaluates one. *)
     let first = translate_expr ctx (Some acc_b.b_ty) value in
     let per_iter = 1 + expr_nodes first + call_weight ctx first in
     let iterations = k + 1 in
     let projected = per_iter * iterations in
     if !(ctx.budget) + projected > 100000 then
       refuse
         "bounded-fold-v1 expansion of %s projects %d KernelIR node(s) for %d \
          iteration(s), over the 100000-node budget"
         ctx.entry_name projected iterations;
     let rec go i ctx acc_stmts =
       if i > k then (List.rev acc_stmts, ctx)
       else
         let e =
           if i = 0 then first else translate_expr ctx (Some acc_b.b_ty) value
         in
         (* Shadow the accumulator's source name; the KernelIR local id is what
            distinguishes the iterations. *)
         let (nb, ctx') = bind_local ctx acc_b.b_name acc_b.b_ty in
         let nb' = { nb with b_assigned = true } in
         let ctx'' =
           { ctx' with
             locals = nb' :: List.filter
                        (fun x -> x.b_local <> nb'.b_local) ctx'.locals }
         in
         go (i + 1) ctx'' (Gpu_ir.Let (nb.b_local, e) :: acc_stmts)
     in
     let (lets, ctx') = go 0 ctx [] in
     (Gpu_ir.Seq lets, ctx')
  | Mt.MBorrow _ -> refuse "borrows and mutable borrows are not part of v1"
  | Mt.MDiscarding _ -> refuse "discarded expressions are not part of v1"
  | Mt.MLetTmp (name, ty, value) when is_index ty ->
     (* `LiftControlPass` materialises a loop's bounds as Index temporaries, so
        this is where a `for` first becomes visible to the emitter. Under the
        strict profile it is still a refusal; the bounded-fold profile records
        the literal and keeps the plumbing out of the kernel values. *)
     if ctx.source_profile = StrictV1 then
       refuse
         "compiler temporary %s has type Index: the function contains a for \
          loop, and for loops are not part of v1; the entry requires \
          --gpu-source-profile=bounded-fold-v1"
         (Identifier.ident_string name);
     if ctx.depth > 0 then
       refuse
         "compiler temporary %s has type Index: bounded-fold-v1 admits no loop \
          in a helper" (Identifier.ident_string name);
     let n = Identifier.ident_string name in
     let (b, ctx') = bind_local ctx n ty in
     (match unwrap_literal value with
      | Mt.MIntConstant s ->
         Hashtbl.replace ctx.index_literals n
           (Z.of_string (Gpu_ir.u32_of_string s))
      | _ -> ());
     (Gpu_ir.Seq [], mark_assigned ctx' b)
  | Mt.MLetTmp (name, ty, value) ->
     (* A compiler-generated initialization temporary: a one-time binding of a
        supported value. It carries no continuation of its own. *)
     let n = Identifier.ident_string name in
     if not (supported_value_type ~profile:ctx.profile ty) then
       refuse "compiler temporary %s has type %s, which %s does not admit"
         n (type_name ty) (Gpu_ir.profile_name ctx.profile);
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

let entry_signature ~(profile: Gpu_ir.numeric_profile) ~(entry: string)
    (module_str: string) (d: decls): Id.decl_id * fn_info * binding list =
  let profile_name = Gpu_ir.profile_name profile in
  let scalar = profile_scalar_type profile in
  let (entry_id, fn) =
    match List.find_opt (fun (_, f) -> f.fn_name = entry) d.functions with
    | Some pair -> pair
    | None ->
       refuse "entry function %s is not a monomorphic function of module %s \
               (generic entries and other modules are not part of %s)"
         entry module_str profile_name
  in
  let arity = List.length fn.fn_params in
  if arity <> 1 && arity <> 2 then
    refuse "entry %s takes %d parameter(s); %s requires one or two %s inputs"
      entry arity profile_name (type_name scalar);
  List.iter
    (fun (name, ty) ->
      if ty <> scalar then
        refuse "entry %s parameter %s has type %s; %s requires %s"
          entry name (type_name ty) profile_name (type_name scalar))
    fn.fn_params;
  if fn.fn_ret <> scalar then
    refuse "entry %s returns %s; %s requires a %s result"
      entry (type_name fn.fn_ret) profile_name (type_name scalar);
  let params =
    List.mapi
      (fun i (name, ty) ->
        { b_name = name; b_local = -1; b_ty = ty;
          b_assigned = true; b_is_param = true; b_param_index = i })
      fn.fn_params
  in
  (entry_id, fn, params)

let build ?(profile = Gpu_ir.U32ModularV1)
    ?(source_profile = StrictV1) ~(entry: string)
    (env: Env.env) (mono: Mt.mono_module): Gpu_ir.kernel =
  let Mt.MonoModule (module_name, _) = mono in
  let module_str = Identifier.mod_name_string module_name in
  let (d: decls) = decls_of_module mono in
  let (entry_id, fn, params) = entry_signature ~profile ~entry module_str d in
  let arity = List.length fn.fn_params in
  let ctx = {
    module_name = module_str;
    profile;
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
    entry_name = entry;
    source_profile;
    fold_acc = ref None;
    index_literals = Hashtbl.create 8;
    fold_seen = ref false;
  } in
  let (body, _) = translate_stmt ctx fn.fn_body in
  let kernel = {
    Gpu_ir.entry_name = entry;
    input_arity = arity;
    helpers = !(ctx.helper_defs);
    body;
    memory = Gpu_ir.default_memory_plan arity;
    numeric_profile = profile;
  } in
  Gpu_ir.check_kernel kernel;
  kernel

(* -------------------------------------------------------------------------- *)
(* G17a: bounded-fold source admission probe                                   *)
(* -------------------------------------------------------------------------- *)

type bounded_fold = {
  fold_entry: string;
  (* The private accumulator: its name, the IR local the first assignment
     would use, and the resolved initializer. *)
  fold_binder: string;
  fold_binder_local: int;
  fold_init: Gpu_ir.expr;
  (* The literal bound as written, and the number of body executions. The C
     renderer, the language specification and (since the endpoint was
     reconciled) the CPS lowering all use the inclusive endpoint, so
     `from 0 to K` runs K+1 times; docs/gpu/BOUNDED_FOLD.md records the
     measurement. *)
  fold_literal_bound: int;
  fold_iterations: int;
  fold_update: Gpu_ir.expr;
  fold_result: Gpu_ir.expr;
  fold_params: int;
}

let recognize_bounded_fold ?(profile = Gpu_ir.U32ModularV1)
    ?(source_profile = BoundedFoldV1) ~(entry: string)
    (env: Env.env) (mono: Mt.mono_module): bounded_fold =
  let Mt.MonoModule (module_name, _) = mono in
  let module_str = Identifier.mod_name_string module_name in
  let (d: decls) = decls_of_module mono in
  let (entry_id, fn, params) = entry_signature ~profile ~entry module_str d in
  let scalar = profile_scalar_type profile in
  let profile_name = Gpu_ir.profile_name profile in
  let base = {
    module_name = module_str;
    profile;
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
    entry_name = entry;
    source_profile;
    fold_acc = ref None;
    index_literals = Hashtbl.create 8;
    fold_seen = ref false;
  } in
  let accumulator: (binding * Gpu_ir.expr) option ref = ref None in
  let index_literals = base.index_literals in
  let the_fold: bounded_fold option ref = ref None in
  let result: Gpu_ir.expr option ref = ref None in
  let returned = ref false in

  let statement_after_return (s: Mt.mstmt) =
    match s with
    | Mt.MSkip -> ()
    | _ -> refuse "statements after the entry's return are not part of \
                   bounded-fold-v1"
  in

  let assign_acc (ctx: ctx) (b: binding) (value: Mt.mexpr): ctx =
    let e = translate_expr ctx (Some b.b_ty) value in
    (match !accumulator with
     | Some (other, _) when other.b_local <> b.b_local ->
        refuse
          "bounded-fold-v1 admits exactly one private accumulator; %s and %s \
           are both assigned" other.b_name b.b_name
     | Some _ ->
        refuse "bounded-fold-v1 admits one assignment to the accumulator %s \
                (a second mutable binding is not admitted)" b.b_name
     | None -> ());
    accumulator := Some (b, e);
    mark_assigned ctx b
  in

  let fold_body (ctx: ctx) (acc_b: binding) (body: Mt.mstmt): Gpu_ir.expr =
    (* The lifted body is a block of skips, an optional compiler temporary for
       the assigned value and the one assignment to the accumulator. Flatten
       it rather than assuming a particular nesting of `MBlock`. *)
    let rec collect (s: Mt.mstmt) (found: Mt.mstmt list) =
      match s with
      | Mt.MSkip -> found
      | Mt.MBlock (a, b) -> collect b (collect a found)
      | other -> found @ [ other ]
    in
    let assigns_acc (q: Identifier.qident) (value: Mt.mexpr): Gpu_ir.expr option =
      let n = Identifier.ident_string (Identifier.original_name q) in
      match lookup_local ctx n with
      | None -> refuse "the bounded loop assigns unknown local %s" n
      | Some b when b.b_local <> acc_b.b_local ->
         refuse
           "the bounded loop assigns %s; bounded-fold-v1 admits exactly one \
            accumulator (%s)" n acc_b.b_name
      | Some _ -> Some (translate_expr ctx (Some acc_b.b_ty) value)
    in
    let unsupported () =
      refuse
        "the bounded loop body must be one assignment to the accumulator; \
         nested control flow, effects and a second write are not part of \
         bounded-fold-v1"
    in
    match collect body [] with
    | [ Mt.MAssignVar (q, value) ] ->
       (match assigns_acc q value with Some e -> e | None -> unsupported ())
    | [ Mt.MLetTmp (name, _ty, value); Mt.MAssignVar (q, Mt.MTemporary (r, _)) ]
      when Identifier.ident_string name = Identifier.ident_string r ->
       (match assigns_acc q value with Some e -> e | None -> unsupported ())
    | _ -> unsupported ()
  in

  let rec walk (ctx: ctx) (s: Mt.mstmt): ctx =
    if !returned then statement_after_return s;
    match s with
    | Mt.MSkip -> ctx
    | Mt.MLet (name, ty, body) ->
       let n = Identifier.ident_string name in
       if not (supported_value_type ~profile:ctx.profile ty) then
         refuse "binding %s has type %s, which %s does not admit"
           n (type_name ty) profile_name;
       let (_, ctx') = bind_local ctx n ty in
       walk ctx' body
    | Mt.MLetTmp (name, ty, value) ->
       let n = Identifier.ident_string name in
       if is_index ty then begin
         (* Loop-bound plumbing, not a kernel value: record its literal if it
            has one and keep it out of expression translation. *)
         let (b, ctx') = bind_local ctx n ty in
         (match unwrap_literal value with
          | Mt.MIntConstant s ->
             Hashtbl.replace index_literals n
               (Z.of_string (Gpu_ir.u32_of_string s))
          | _ -> ());
         mark_assigned ctx' b
       end else begin
         if not (supported_value_type ~profile:ctx.profile ty) then
           refuse "compiler temporary %s has type %s, which %s does not admit"
             n (type_name ty) profile_name;
         let (b, ctx') = bind_local ctx n ty in
         ignore (translate_expr ctx (Some ty) value);
         mark_assigned ctx' b
       end
    | Mt.MAssignTmp (name, value) ->
       let n = Identifier.ident_string name in
       (match lookup_local ctx n with
        | Some b when not b.b_assigned ->
           ignore (translate_expr ctx (Some b.b_ty) value);
           mark_assigned ctx b
        | Some _ ->
           refuse "compiler temporary %s is assigned more than once" n
        | None -> refuse "assignment to unknown compiler temporary %s" n)
    | Mt.MBlock (a, b) -> walk (walk ctx a) b
    | Mt.MInitialAssign (q, value) ->
       let n = Identifier.ident_string (Identifier.original_name q) in
       (match lookup_local ctx n with
        | None -> refuse "initial assignment to unknown local %s" n
        | Some b when b.b_is_param ->
           refuse "assignment to parameter %s is not part of bounded-fold-v1" n
        | Some b when b.b_assigned ->
           refuse "local %s is assigned more than once" n
        | Some b ->
           if b.b_ty <> scalar then
             refuse
               "the private accumulator %s has type %s; %s requires %s"
               n (type_name b.b_ty) (source_profile_name source_profile)
               (type_name scalar);
           assign_acc ctx b value)
    | Mt.MAssignVar (q, _) ->
       refuse
         "mutable assignment to %s is only admitted as the single body of the \
          entry's bounded for loop"
         (Identifier.ident_string (Identifier.original_name q))
    | Mt.MAssign _ ->
       refuse
         "assignment through an address is not part of bounded-fold-v1 (the \
          fold writes one named private accumulator)"
    | Mt.MFor (id, start, finish, body) ->
       (match !the_fold with
        | Some _ ->
           refuse
             "bounded-fold-v1 admits one for loop in the entry body; a second \
              (or nested) loop is not admitted"
        | None -> ());
       (match source_profile with
        | StrictV1 ->
           refuse
             "for loops are not part of v1; the entry %s requires \
              --gpu-source-profile=bounded-fold-v1" entry
        | BoundedFoldV1 -> ());
       let start_v =
         bound_u32 ~index_literals
           (Printf.sprintf "entry %s loop start" entry) start
       in
       if not (Z.equal start_v Z.zero) then
         refuse
           "bounded-fold-v1 admits `for i from 0 to K`; the entry's loop \
            starts at %s" (Z.to_string start_v);
       let k =
         let v =
           bound_u32 ~index_literals
             (Printf.sprintf "entry %s loop bound" entry) finish
         in
         if Z.compare v (Z.of_int 31) > 0 then
           refuse
             "the entry's literal loop bound %s is greater than 31; \
              bounded-fold-v1 admits 0..31"
             (Z.to_string v);
         Z.to_int v
       in
       let acc_b, acc_init =
         match !accumulator with
         | Some pair -> pair
         | None ->
            refuse
              "the entry's bounded loop needs an already initialized private \
               accumulator before it; none was found"
       in
       let binder_name = Identifier.ident_string id in
       let binder =
         { b_name = binder_name; b_local = -1; b_ty = index_type ();
           b_assigned = true; b_is_param = false; b_param_index = -1 }
       in
       let body_ctx = { ctx with locals = binder :: ctx.locals } in
       let update = fold_body body_ctx acc_b body in
       the_fold := Some {
         fold_entry = entry;
         fold_binder = acc_b.b_name;
         fold_binder_local = acc_b.b_local;
         fold_init = acc_init;
         fold_literal_bound = k;
         fold_iterations = k + 1;
         fold_update = update;
         fold_result = Gpu_ir.Local acc_b.b_local;
         fold_params = List.length fn.fn_params;
       };
       ctx
    | Mt.MReturn e ->
       if !returned then
         refuse "bounded-fold-v1 admits exactly one return, at the end of the entry body";
       let v = translate_expr ctx (Some fn.fn_ret) e in
       (match v with
        | Gpu_ir.Local id when id = (match !accumulator with
                                    | Some (b, _) -> b.b_local
                                    | None -> -1) -> ()
        | _ ->
           refuse
             "the entry's return must be the private accumulator itself; the \
              fold's result is the value after the last iteration");
       result := Some v;
       returned := true;
       ctx
    | Mt.MIf _ ->
       refuse
         "conditionals in the entry body are not part of bounded-fold-v1 (the \
          admitted body is one bounded loop over one accumulator)"
    | Mt.MDestructure _ ->
       refuse "destructuring bindings are not part of bounded-fold-v1"
    | Mt.MCase _ -> refuse "match/case is not part of bounded-fold-v1"
    | Mt.MWhile _ -> refuse "while loops are not part of bounded-fold-v1"
    | Mt.MBorrow _ -> refuse "borrows and mutable borrows are not part of bounded-fold-v1"
    | Mt.MDiscarding _ ->
       refuse "discarded expressions are not part of bounded-fold-v1 (effects are not admitted)"
  in
  ignore (walk base fn.fn_body);
  let fold =
    match !the_fold with
    | Some f -> f
    | None ->
       refuse
         "entry %s has no top-level bounded for loop; recognize_bounded_fold \
          only reports the bounded-fold-v1 shape" entry
  in
  (match !result with
   | Some _ -> ()
   | None -> refuse "entry %s has no return of the accumulator" entry);
  fold
