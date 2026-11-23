(*
   MLIR rendering from typed KernelIR (PLAN_gpu_kernels.md G20a).

   This is the second consumer of `Gpu_ir.kernel`, beside `Gpu_wgsl`: the same
   typed values, bindings and branches, lowered to the `arith` / `memref` /
   `scf` / `gpu` dialects instead of WGSL. Nothing here re-reads source or
   re-implements the subset pass, so the two emitters cannot disagree about
   their input — `ir_sha256` covers both.

   Encoding rules G20a fixes, applied here rather than left to a pass:

   - `Nat32` is signless `i32` in MLIR: there is no unsigned type, so every
     ordering comparison is the unsigned predicate (`ult`, `ugt`, `ule`, `uge`).
     A `slt` in this file would be a semantic bug, and `validate` refuses one.
   - modular arithmetic carries no `nuw`/`nsw` flag: `u32-modular-v1` wraps,
     and an overflow flag would claim undefined behaviour the profile lacks.
   - launch ids and indices are converted explicitly, never implicitly.
   - the memory plan is the v1 one: a declared static extent, an explicit tail
     guard that dominates every load and store, one element per thread.

   What this module does not do: verify or lower the result. `validate` is a
   *structural* check — names defined before use, balanced structure, no signed
   comparison, no wrap flag, one kernel function — and never a claim that
   `mlir-opt` accepts the file; that is `scripts/gpu/mlir-probe`'s job (G19a),
   which verifies and lowers the *emitted* module rather than proving this
   renderer correct. `mm0/proof_chain/mlir/stage.toml` keeps that distinction:
   capturing the emitted MLIR moves `kernelir-to-mlir` to `captured`, while
   preservation stays open (no MLIR semantics is declared).
*)

let fail fmt = Printf.ksprintf (fun s -> raise (Invalid_argument s)) fmt

let type_string = function
  | Gpu_ir.TyU32 -> "i32"
  | Gpu_ir.TyBool -> "i1"
  | Gpu_ir.TyF32 -> "f32"

let intrinsic_op = function
  | Gpu_ir.Add -> "arith.addi"
  | Gpu_ir.Subtract -> "arith.subi"
  | Gpu_ir.Multiply -> "arith.muli"
  | Gpu_ir.BitwiseAnd -> "arith.andi"
  | Gpu_ir.BitwiseOr -> "arith.ori"
  | Gpu_ir.BitwiseXor -> "arith.xori"
  | Gpu_ir.BitwiseNot -> "arith.xori"   (* against an all-ones constant *)
  | Gpu_ir.F32Add -> "arith.addf"
  | Gpu_ir.F32Subtract -> "arith.subf"
  | Gpu_ir.F32Multiply -> "arith.mulf"

(** Unsigned predicates: `Nat32` is `i32` in MLIR, so an ordering comparison
    must say `u`. This one function is the whole rule. *)
let comparison_pred = function
  | Gpu_ir.Eq -> "eq"
  | Gpu_ir.NotEqual -> "ne"
  | Gpu_ir.LessThan -> "ult"
  | Gpu_ir.LessThanOrEqual -> "ule"
  | Gpu_ir.GreaterThan -> "ugt"
  | Gpu_ir.GreaterThanOrEqual -> "uge"

let boolean_op_string = function
  | Gpu_ir.Conjunction -> "arith.andi"
  | Gpu_ir.Disjunction -> "arith.ori"

type env = {
  params: string list;              (** loaded scalar SSA names, per argument *)
  mutable locals: (int * string * Gpu_ir.value_type) list;
  mutable next: int;
  mutable depth: int;
  mutable region: int;              (** >0 inside an scf region *)
  mutable result: (string * Gpu_ir.value_type) option;
  mutable buf: Buffer.t;
}

let indent_of depth = String.make (depth * 2) ' '

let emit env line =
  Buffer.add_string env.buf (indent_of env.depth);
  Buffer.add_string env.buf line;
  Buffer.add_char env.buf '\n'

let fresh env =
  let n = env.next in
  env.next <- n + 1;
  Printf.sprintf "%%v%d" n

(** The KernelIR type of an expression. Locals no longer carry their type in
    the environment (they carry an SSA name), so this is only used for values
    whose type is structural. *)
let rec expr_type env (e: Gpu_ir.expr): Gpu_ir.value_type =
  match e with
  | Gpu_ir.Param _ -> Gpu_ir.TyU32
  | Gpu_ir.Local id ->
     (match List.find_opt (fun (k, _, _) -> k = id) env.locals with
      | Some (_, _, ty) -> ty
      | None -> fail "Gpu_mlir: unknown local %d" id)
  | Gpu_ir.U32 _ -> Gpu_ir.TyU32
  | Gpu_ir.F32 _ -> Gpu_ir.TyF32
  | Gpu_ir.Bool _ -> Gpu_ir.TyBool
  | Gpu_ir.Intrinsic (i, _) ->
     if Gpu_ir.is_float_intrinsic i then Gpu_ir.TyF32 else Gpu_ir.TyU32
  | Gpu_ir.Compare _ -> Gpu_ir.TyBool
  | Gpu_ir.BooleanOp _ -> Gpu_ir.TyBool
  | Gpu_ir.Not _ -> Gpu_ir.TyBool
  | Gpu_ir.Conditional (_, t, _) -> expr_type env t
  | Gpu_ir.Call _ ->
     fail "Gpu_mlir: helper call is not lowered yet (G20a covers the scalar \
           body; the v1 helper corpus is a later step)"

(** Render an expression in place, returning the SSA name holding its value. *)
let rec render_expr env (e: Gpu_ir.expr): string =
  match e with
  | Gpu_ir.Param i ->
     (match List.nth_opt env.params i with
      | Some name -> name
      | None -> fail "Gpu_mlir: unknown param %d" i)
  | Gpu_ir.Local id ->
     (match List.find_opt (fun (k, _, _) -> k = id) env.locals with
      | Some (_, name, _) -> name
      | None -> fail "Gpu_mlir: unknown local %d" id)
  | Gpu_ir.U32 s ->
     let v = fresh env in
     emit env (Printf.sprintf "%s = arith.constant %s : i32" v s);
     v
  | Gpu_ir.F32 s ->
     let v = fresh env in
     emit env (Printf.sprintf "%s = arith.constant %s : f32" v s);
     v
  | Gpu_ir.Bool b ->
     let v = fresh env in
     emit env (Printf.sprintf "%s = arith.constant %s"
                 v (if b then "true" else "false"));
     v
  | Gpu_ir.Intrinsic (Gpu_ir.BitwiseNot, [a]) ->
     let all = fresh env in
     emit env (Printf.sprintf "%s = arith.constant -1 : i32" all);
     let va = render_expr env a in
     let v = fresh env in
     emit env (Printf.sprintf "%s = arith.xori %s, %s : i32" v va all);
     v
  | Gpu_ir.Intrinsic (op, [a; b]) ->
     let va = render_expr env a and vb = render_expr env b in
     let ty = type_string (expr_type env e) in
     let v = fresh env in
     emit env (Printf.sprintf "%s = %s %s, %s : %s"
                 v (intrinsic_op op) va vb ty);
     v
  | Gpu_ir.Intrinsic (_, args) ->
     fail "Gpu_mlir: intrinsic arity %d" (List.length args)
  | Gpu_ir.Compare (op, a, b) ->
     let va = render_expr env a and vb = render_expr env b in
     let v = fresh env in
     emit env (Printf.sprintf "%s = arith.cmpi %s, %s, %s : %s"
                 v (comparison_pred op) va vb (type_string (expr_type env a)));
     v
  | Gpu_ir.BooleanOp (op, a, b) ->
     let va = render_expr env a and vb = render_expr env b in
     let v = fresh env in
     emit env (Printf.sprintf "%s = %s %s, %s : i1"
                 v (boolean_op_string op) va vb);
     v
  | Gpu_ir.Not a ->
     let va = render_expr env a in
     let v = fresh env in
     emit env (Printf.sprintf "%s = arith.xori %s, true : i1" v va);
     v
  | Gpu_ir.Conditional (c, t, f) ->
     let vc = render_expr env c in
     let ty = type_string (expr_type env t) in
     let v = fresh env in
     emit env (Printf.sprintf "%s = scf.if %s -> (%s) {" v vc ty);
     env.depth <- env.depth + 1;
     let vt = render_expr env t in
     emit env (Printf.sprintf "scf.yield %s : %s" vt ty);
     env.depth <- env.depth - 1;
     emit env "} else {";
     env.depth <- env.depth + 1;
     let vf = render_expr env f in
     emit env (Printf.sprintf "scf.yield %s : %s" vf ty);
     env.depth <- env.depth - 1;
     emit env "}";
     v
  | Gpu_ir.Call _ ->
     fail "Gpu_mlir: helper call is not lowered yet (G20a covers the scalar \
           body; the v1 helper corpus is a later step)"

let rec render_stmt env (s: Gpu_ir.stmt): unit =
  match s with
  | Gpu_ir.Let (id, e) ->
     let ty = expr_type env e in
     let v = render_expr env e in
     env.locals <- (id, v, ty) :: env.locals
  | Gpu_ir.Seq items -> List.iter (render_stmt env) items
  | Gpu_ir.If (c, t, f) ->
     let vc = render_expr env c in
     let (then_text, then_res) = render_branch env t in
     let (else_text, else_res) = render_branch env f in
     (match then_res, else_res with
      | Some (vt, tyt), Some (vf, tyf) ->
         if tyt <> tyf then
           fail "Gpu_mlir: the two branches of an if return %s and %s"
             (type_string tyt) (type_string tyf);
         let ty = type_string tyt in
         let v = fresh env in
         emit env (Printf.sprintf "%s = scf.if %s -> (%s) {" v vc ty);
         env.depth <- env.depth + 1;
         Buffer.add_string env.buf then_text;
         emit env (Printf.sprintf "scf.yield %s : %s" vt ty);
         env.depth <- env.depth - 1;
         emit env "} else {";
         env.depth <- env.depth + 1;
         Buffer.add_string env.buf else_text;
         emit env (Printf.sprintf "scf.yield %s : %s" vf ty);
         env.depth <- env.depth - 1;
         emit env "}";
         env.result <- Some (v, tyt)
      | None, None ->
         emit env (Printf.sprintf "scf.if %s {" vc);
         env.depth <- env.depth + 1;
         Buffer.add_string env.buf then_text;
         env.depth <- env.depth - 1;
         emit env "} else {";
         env.depth <- env.depth + 1;
         Buffer.add_string env.buf else_text;
         env.depth <- env.depth - 1;
         emit env "}"
      | _ ->
         fail "Gpu_mlir: an if either returns from both branches or from \
               neither (a one-armed return has no lowering here)")
  | Gpu_ir.Return e ->
     env.result <- Some (render_expr env e, expr_type env e)

(** Render one branch of an `if` into its own buffer, so the `scf.if` header
    can be written before the region that the branch's lines belong to. *)
and render_branch env (s: Gpu_ir.stmt): string * (string * Gpu_ir.value_type) option =
  let sub = Buffer.create 128 in
  let saved_buf = env.buf and saved_locals = env.locals
  and saved_result = env.result in
  env.buf <- sub;
  env.locals <- saved_locals;
  env.depth <- env.depth + 1;
  env.region <- env.region + 1;
  env.result <- None;
  render_stmt env s;
  let result = env.result in
  env.buf <- saved_buf;
  env.locals <- saved_locals;
  env.depth <- env.depth - 1;
  env.region <- env.region - 1;
  env.result <- saved_result;
  (Buffer.contents sub, result)

let extent_string extent =
  if extent <= 0 then fail "Gpu_mlir: static extent must be positive, got %d" extent;
  string_of_int extent

(** The full module text. Deterministic: no timestamps, no hash-map order, and
    the identity line is the same `ir_sha256` PC04/PC07 pin. *)
let render ~(extent: int) (kernel: Gpu_ir.kernel): string =
  Gpu_ir.check_kernel kernel;
  let extent = extent_string extent in
  if kernel.Gpu_ir.numeric_profile <> Gpu_ir.U32ModularV1 then
    fail "Gpu_mlir: numeric profile %s is not lowered (G20a covers \
          u32-modular-v1)"
      (Gpu_ir.profile_name kernel.Gpu_ir.numeric_profile);
  if kernel.Gpu_ir.helpers <> [] then
    fail "Gpu_mlir: helper call is not lowered yet (G20a covers the scalar \
          body; the v1 helper corpus is a later step)";
  let arity = kernel.Gpu_ir.input_arity in
  let ir_sha256 = Gpu_ir.sha256_hex (Gpu_ir.canonical_bytes kernel) in
  let buf = Buffer.create 1024 in
  let add line = Buffer.add_string buf line; Buffer.add_char buf '\n' in
  add "// Generated from typed KernelIR by the Austral GPU backend (G20a). Do not edit.";
  Printf.bprintf buf "// kernelir-sha256: %s\n" ir_sha256;
  Printf.bprintf buf "// numeric-profile: %s\n"
    (Gpu_ir.profile_name kernel.Gpu_ir.numeric_profile);
  Printf.bprintf buf "// extent: %s\n" extent;
  add "// abi: g19a-pending (memref per operand; not verified by an MLIR toolchain)";
  Printf.bprintf buf
    "module attributes {austral.extent = %s : i64, austral.numeric_profile = \
     \"%s\", austral.abi = \"g19a-pending\"} {\n"
    extent (Gpu_ir.profile_name kernel.Gpu_ir.numeric_profile);
  Printf.bprintf buf "  gpu.module @%s {\n" kernel.Gpu_ir.entry_name;
  let memref = Printf.sprintf "memref<%sxi32>" extent in
  let params =
    List.init arity (fun i -> Printf.sprintf "%%arg%d: %s" i memref)
    @ [Printf.sprintf "%%arg%d: %s" arity memref]
  in
  Printf.bprintf buf
    "    gpu.func @main(%s) kernel attributes {gpu.known_block_size = \
     array<i32: %d, 1, 1>} {\n"
    (String.concat ", " params) kernel.Gpu_ir.memory.Gpu_ir.workgroup_size;
  let env = { params = List.init arity (fun i -> Printf.sprintf "%%arg%d_v" i);
              locals = []; next = 0; depth = 3; region = 0; result = None; buf } in
  emit env (Printf.sprintf "%%c_extent = arith.constant %s : index" extent);
  (* `gpu.thread_id` is block-local, so the element index is the global one:
     block_id * block_dim + thread_id. Using the block-local id would make a
     second block rewrite the first block's elements whenever the extent is
     larger than the declared block size (G21a measured this at extent 65
     with 64 threads: index 64 was never written). *)
  emit env "%tid = gpu.thread_id x";
  emit env "%bid = gpu.block_id x";
  emit env "%bdim = gpu.block_dim x";
  emit env "%bid_base = arith.muli %bid, %bdim : index";
  emit env "%gid = arith.addi %bid_base, %tid : index";
  emit env "%in_bounds = arith.cmpi ult, %gid, %c_extent : index";
  emit env "scf.if %in_bounds {";
  env.depth <- env.depth + 1;
  for i = 0 to arity - 1 do
    emit env (Printf.sprintf "%%arg%d_v = memref.load %%arg%d[%%gid] : %s"
                i i memref)
  done;
  render_stmt env kernel.Gpu_ir.body;
  let result =
    match env.result with
    | Some (v, _) -> v
    | None -> fail "Gpu_mlir: the kernel body has no return"
  in
  emit env (Printf.sprintf "memref.store %s, %%arg%d[%%gid] : %s"
              result arity memref);
  env.depth <- env.depth - 1;
  emit env "}";
  emit env "gpu.return";
  add "    }";
  add "  }";
  add "}";
  Buffer.contents buf

(* -------------------------------------------------------------------------- *)
(* Structural validation                                                       *)
(* -------------------------------------------------------------------------- *)

module SMap = Map.Make (String)

(** A *structural* check of the emitted text. Deliberately not a verifier: it
    refuses the mistakes G20a's encoding rules exist to prevent — a signed
    ordering comparison, a `nuw`/`nsw` flag, an undefined value, an unbalanced
    region, a missing or duplicated kernel entry — and nothing about types,
    traits or semantics. With no `mlir-opt` in the tree, saying more would be
    pretending. *)
let validate (text: string): (string * int) list =
  let lines = String.split_on_char '\n' text in
  let depth = ref 0 in
  let defined = ref SMap.empty in
  let used = ref SMap.empty in
  let cmpi = ref 0 and loads = ref 0 and stores = ref 0
  and kernels = ref 0 and modules = ref 0 in
  let value_re = Str.regexp "%\\([a-zA-Z_][a-zA-Z0-9_]*\\)" in
  let def_re = Str.regexp " *%\\([a-zA-Z_][a-zA-Z0-9_]*\\) = " in
  let arg_re = Str.regexp "%\\(arg[0-9]+\\): " in
  List.iter
    (fun line ->
       String.iter
         (function
           | '{' -> incr depth
           | '}' -> decr depth
           | _ -> ())
         line;
       if !depth < 0 then fail "Gpu_mlir: unbalanced closing brace";
       if Str.string_match def_re line 0 then
         defined := SMap.add (Str.matched_group 1 line) true !defined;
       (* every block argument of the kernel function is defined by its
          declaration, and there is more than one `%argN:` on that line *)
       let rec args from =
         try
           ignore (Str.search_forward arg_re line from);
           defined := SMap.add (Str.matched_group 1 line) true !defined;
           args (Str.match_end ())
         with Not_found -> ()
       in
       args 0;
       if Str.string_match (Str.regexp ".*gpu\\.func @") line 0 then incr kernels;
       if Str.string_match (Str.regexp ".*gpu\\.module @") line 0 then incr modules;
       if Str.string_match (Str.regexp ".*arith\\.cmpi ") line 0 then begin
         incr cmpi;
         if Str.string_match
              (Str.regexp ".*arith\\.cmpi \\(slt\\|sgt\\|sle\\|sge\\)") line 0
         then fail "Gpu_mlir: signed ordering comparison %s (Nat32 is unsigned)"
                (Str.matched_group 1 line)
       end;
       if Str.string_match (Str.regexp ".*memref\\.load ") line 0 then incr loads;
       if Str.string_match (Str.regexp ".*memref\\.store ") line 0 then incr stores;
       if Str.string_match (Str.regexp ".*\\(nuw\\|nsw\\)") line 0
       then fail "Gpu_mlir: wrap flag on modular arithmetic";
       let rec scan from =
         try
           ignore (Str.search_forward value_re line from);
           let name = Str.matched_group 1 line in
           used := SMap.add name true !used;
           scan (Str.match_end ())
         with Not_found -> ()
       in
       scan 0)
    lines;
  if !depth <> 0 then fail "Gpu_mlir: unbalanced braces (%d open)" !depth;
  if !kernels <> 1 then fail "Gpu_mlir: expected exactly one gpu.func, found %d" !kernels;
  if !modules <> 1 then fail "Gpu_mlir: expected exactly one gpu.module, found %d" !modules;
  SMap.iter
    (fun name _ ->
       if not (SMap.mem name !defined) then
         fail "Gpu_mlir: value %%%s is used but never defined" name)
    !used;
  [("cmpi", !cmpi); ("loads", !loads); ("stores", !stores)]
