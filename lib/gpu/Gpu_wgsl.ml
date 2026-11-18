(*
   WGSL rendering from typed KernelIR (PLAN_gpu_kernels.md G06).

   The renderer is deliberately small and total over the admitted IR: every
   expression/statement variant has one rendering, names are compiler-generated
   (`scalar_entry`, `helper_N`, `vN`), and the map wrapper comes from the
   memory plan rather than from a handwritten template string. User source
   names never appear as WGSL identifiers.

   Validation (Naga) and the memory-wrapper audit belong to the Rust checker
   (G07); this module only produces the exact bytes.
*)

let type_string = function Gpu_ir.TyU32 -> "u32" | Gpu_ir.TyBool -> "bool"

let intrinsic_string = function
  | Gpu_ir.Add -> "+"
  | Gpu_ir.Subtract -> "-"
  | Gpu_ir.Multiply -> "*"
  | Gpu_ir.BitwiseAnd -> "&"
  | Gpu_ir.BitwiseOr -> "|"
  | Gpu_ir.BitwiseXor -> "^"
  | Gpu_ir.BitwiseNot -> "~"

let comparison_string = function
  | Gpu_ir.Eq -> "=="
  | Gpu_ir.NotEqual -> "!="
  | Gpu_ir.LessThan -> "<"
  | Gpu_ir.LessThanOrEqual -> "<="
  | Gpu_ir.GreaterThan -> ">"
  | Gpu_ir.GreaterThanOrEqual -> ">="

let boolean_op_string = function
  | Gpu_ir.Conjunction -> "&&"
  | Gpu_ir.Disjunction -> "||"

type env = {
  params: Gpu_ir.value_type list;
  locals: (int * Gpu_ir.value_type) list;
  helper_rets: (int * Gpu_ir.value_type) list;
}

let local_type (env: env) (id: int): Gpu_ir.value_type =
  match List.assoc_opt id env.locals with
  | Some t -> t
  | None -> failwith (Printf.sprintf "Gpu_wgsl: unknown local %d" id)

let rec expr_type (env: env) (e: Gpu_ir.expr): Gpu_ir.value_type =
  match e with
  | Gpu_ir.Param i ->
     (match List.nth_opt env.params i with
      | Some t -> t
      | None -> failwith (Printf.sprintf "Gpu_wgsl: unknown param %d" i))
  | Gpu_ir.Local id -> local_type env id
  | Gpu_ir.U32 _ -> Gpu_ir.TyU32
  | Gpu_ir.Bool _ -> Gpu_ir.TyBool
  | Gpu_ir.Intrinsic _ -> Gpu_ir.TyU32
  | Gpu_ir.Compare _ -> Gpu_ir.TyBool
  | Gpu_ir.BooleanOp _ -> Gpu_ir.TyBool
  | Gpu_ir.Not _ -> Gpu_ir.TyBool
  | Gpu_ir.Conditional (_, t, _) -> expr_type env t
  | Gpu_ir.Call (f, _) ->
     match List.assoc_opt f env.helper_rets with
     | Some t -> t
     | None -> failwith (Printf.sprintf "Gpu_wgsl: unknown helper %d" f)

let rec render_expr (env: env) (e: Gpu_ir.expr): string =
  match e with
  | Gpu_ir.Param i -> Printf.sprintf "p%d" i
  | Gpu_ir.Local id -> Printf.sprintf "v%d" id
  | Gpu_ir.U32 s -> s ^ "u"
  | Gpu_ir.Bool true -> "true"
  | Gpu_ir.Bool false -> "false"
  | Gpu_ir.Intrinsic (Gpu_ir.BitwiseNot, [a]) ->
     Printf.sprintf "(~%s)" (render_expr env a)
  | Gpu_ir.Intrinsic (op, [a; b]) ->
     Printf.sprintf "(%s %s %s)"
       (render_expr env a) (intrinsic_string op) (render_expr env b)
  | Gpu_ir.Intrinsic (_, args) ->
     failwith (Printf.sprintf "Gpu_wgsl: intrinsic arity %d" (List.length args))
  | Gpu_ir.Compare (op, a, b) ->
     Printf.sprintf "(%s %s %s)"
       (render_expr env a) (comparison_string op) (render_expr env b)
  | Gpu_ir.BooleanOp (op, a, b) ->
     Printf.sprintf "(%s %s %s)"
       (render_expr env a) (boolean_op_string op) (render_expr env b)
  | Gpu_ir.Not a -> Printf.sprintf "(!%s)" (render_expr env a)
  | Gpu_ir.Conditional (c, t, f) ->
     (* WGSL has no ternary operator: select(false, true, condition). *)
     Printf.sprintf "select(%s, %s, %s)"
       (render_expr env f) (render_expr env t) (render_expr env c)
  | Gpu_ir.Call (f, args) ->
     Printf.sprintf "helper_%d(%s)"
       f (String.concat ", " (List.map (render_expr env) args))

let indent n = String.make (n * 4) ' '

let rec render_stmt (env: env) (depth: int) (s: Gpu_ir.stmt): string * env =
  match s with
  | Gpu_ir.Let (id, e) ->
     let ty = expr_type env e in
     let line =
       Printf.sprintf "%slet v%d: %s = %s;\n"
         (indent depth) id (type_string ty) (render_expr env e)
     in
     (line, { env with locals = (id, ty) :: env.locals })
  | Gpu_ir.Seq items ->
     let buf = Buffer.create 64 in
     let env = List.fold_left
       (fun env s ->
         let (text, env') = render_stmt env depth s in
         Buffer.add_string buf text;
         env')
       env items
     in
     (Buffer.contents buf, env)
  | Gpu_ir.If (c, t, f) ->
     let (then_text, _) = render_stmt env (depth + 1) t in
     let (else_text, _) = render_stmt env (depth + 1) f in
     (Printf.sprintf "%sif %s {\n%s%s} else {\n%s%s}\n"
        (indent depth) (render_expr env c)
        then_text (indent depth)
        else_text (indent depth),
      env)
  | Gpu_ir.Return e ->
     (Printf.sprintf "%sreturn %s;\n" (indent depth) (render_expr env e), env)

let render_helper (helper: Gpu_ir.helper): string =
  let params =
    List.mapi
      (fun i ty -> Printf.sprintf "p%d: %s" i (type_string ty))
      helper.Gpu_ir.params
  in
  let env = {
    params = helper.Gpu_ir.params;
    locals = [];
    helper_rets = [];
  } in
  let (body, _) = render_stmt env 1 helper.Gpu_ir.body in
  Printf.sprintf "fn helper_%d(%s) -> %s {\n%s}\n\n"
    helper.Gpu_ir.id (String.concat ", " params)
    (type_string helper.Gpu_ir.ret) body

let render_bindings (arity: int): string =
  let inputs =
    List.init arity (fun i ->
      Printf.sprintf "@group(0) @binding(%d) var<storage, read> input%d: array<u32>;\n"
        i i)
  in
  let output =
    Printf.sprintf "@group(0) @binding(%d) var<storage, read_write> output0: array<u32>;\n\n"
      arity
  in
  String.concat "" inputs ^ output

let render_entry_params (arity: int): string =
  String.concat ", "
    (List.init arity (fun i -> Printf.sprintf "p%d: u32" i))

let render_entry_call_args (arity: int): string =
  String.concat ", " (List.init arity (fun i -> Printf.sprintf "input%d[i]" i))

let render (kernel: Gpu_ir.kernel): string =
  Gpu_ir.check_kernel kernel;
  let helper_rets = List.map (fun (h: Gpu_ir.helper) -> (h.Gpu_ir.id, h.Gpu_ir.ret)) kernel.Gpu_ir.helpers in
  let kenv = { params = List.init kernel.Gpu_ir.input_arity (fun _ -> Gpu_ir.TyU32);
               locals = []; helper_rets } in
  let (body, _) = render_stmt kenv 1 kernel.Gpu_ir.body in
  let helpers =
    String.concat "" (List.map render_helper kernel.Gpu_ir.helpers)
  in
  let arity = kernel.Gpu_ir.input_arity in
  let buf = Buffer.create 1024 in
  Buffer.add_string buf
    "// Generated from typed KernelIR by the Austral GPU backend. Do not edit.\n";
  Buffer.add_string buf (render_bindings arity);
  Buffer.add_string buf helpers;
  Buffer.add_string buf
    (Printf.sprintf "fn scalar_entry(%s) -> u32 {\n%s}\n\n"
       (render_entry_params arity) body);
  Buffer.add_string buf
    (Printf.sprintf
       "@compute @workgroup_size(%d)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>) {\n\
        \    let i = gid.x;\n\
        \    if (i >= arrayLength(&input0)) { return; }\n\
        \    output0[i] = scalar_entry(%s);\n}\n"
       kernel.Gpu_ir.memory.Gpu_ir.workgroup_size (render_entry_call_args arity));
  Buffer.contents buf
