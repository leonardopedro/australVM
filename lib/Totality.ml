(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(* avm-pkg spec task 2.1 (docs/pkg_manager_plan.md §3.1): the package
   totality pass. Package modules must be structurally terminating so a
   derivation cannot diverge inside the hermetic runner. This is an
   independent pass over the typed AST — spec §6.2 forbids touching the
   linear type checker (lib/Linear/) — and it only runs when the compiler
   is invoked with `--pkg-totality`, so ordinary compilation is untouched.

   v1 rules (PLAN_avm_pkg.md §5.1: sound over-approximations of the
   spec's three rules):

   1. No `while` statement. The spec allows an induction-variable loop
      with a statically provable upper bound; v1 does not attempt the
      bound proof, so it rejects every `while`. `for` loops are allowed:
      they are bounded by construction.

   2. No recursion *within the module*: direct or mutual call cycles
      through module-level functions and instance methods are rejected by
      cycle detection over the intra-module call graph. Cross-module
      recursion is impossible (Austral's module import graph is acyclic,
      so a function can only call functions in already-compiled modules or
      in its own module), which makes intra-module acyclicity sufficient
      for whole-program termination of the pure computation.

   3. No unmediated side effects or unprovable control flow: foreign
      function declarations and their import/export pragmas, the
      `Unsafe_Module` pragma, `@embed` expressions, function-pointer calls
      (`TFptrCall`), and dynamic method dispatch (`TVarMethodCall`) are
      all rejected because their targets/effects cannot be bounded here.

   Violations raise a normal compilation error (`Error.err`) naming the
   module, the construct, and the rule it broke. *)

open Identifier
open Common
open Id
open Stages.Tast
open Reporter
open Error

(* A call-graph node. Functions and instance methods share one namespace by
   tagging the underlying integer ids. *)
type node =
  | FunctionNode of int
  | MethodNode of int

let node_key (n: node): string =
  match n with
  | FunctionNode id -> "function#" ^ (string_of_int id)
  | MethodNode id -> "method#" ^ (string_of_int id)

let function_node (id: decl_id): node =
  match id with
  | Id.DeclId n -> FunctionNode n

let method_node (id: ins_meth_id): node =
  match id with
  | Id.InsMethId n -> MethodNode n

let violation (module_name: module_name) (rule: string) (detail: string): 'a =
  err ("Package totality violation in module `"
       ^ (mod_name_string module_name)
       ^ "` (" ^ rule ^ "): " ^ detail)

(* Pragma rule: package modules cannot perform unmediated I/O. Liquid
   contracts are pure specifications and are allowed. *)
let check_pragma (module_name: module_name) (fn_name: identifier) (p: pragma): unit =
  match p with
  | ForeignImportPragma _ ->
     violation module_name "rule 3"
       ("function `" ^ (ident_string fn_name)
        ^ "` has a Foreign_Import pragma: package modules may not perform unmediated I/O")
  | ForeignExportPragma _ ->
     violation module_name "rule 3"
       ("function `" ^ (ident_string fn_name)
        ^ "` has a ForeignExport pragma: package modules may not perform unmediated I/O")
  | UnsafeModulePragma ->
     violation module_name "rule 3"
       ("function `" ^ (ident_string fn_name)
        ^ "` carries the Unsafe_Module pragma: package modules must be safe")
  | RustOwnershipPragma _ ->
     violation module_name "rule 3"
       ("function `" ^ (ident_string fn_name)
        ^ "` is an `extern rust` foreign import: package modules may not perform unmediated I/O")
  | LiquidPragma _ ->
     ()

(* Collect the call-graph edges of one expression, rejecting the constructs
   of rule 3 on sight. *)
let rec expr_edges (module_name: module_name) (e: texpr): node list =
  match e with
  | TNilConstant
  | TBoolConstant _
  | TIntConstant _
  | TFloatConstant _
  | TStringConstant _
  | TConstVar _
  | TParamVar _
  | TLocalVar _
  | TFunVar _
  | TTemporary _
  | TSizeOf _ ->
     []
  | TFuncall (id, _, args, _, _) ->
     (function_node id) :: (List.concat_map (expr_edges module_name) args)
  | TMethodCall (id, _, _, args, _, _) ->
     (method_node id) :: (List.concat_map (expr_edges module_name) args)
  | TVarMethodCall _ ->
     violation module_name "rule 3"
       "dynamic method dispatch (method call on a type-class-constrained type parameter) has an unknown target and cannot be proven to terminate"
  | TFptrCall _ ->
     violation module_name "rule 3"
       "calling through a function pointer has an unknown target and cannot be proven to terminate"
  | TCast (e, _)
  | TDeref e
  | TSlotAccessor (e, _, _)
  | TPointerSlotAccessor (e, _, _)
  | TNegation e ->
     expr_edges module_name e
  | TComparison (_, lhs, rhs)
  | TConjunction (lhs, rhs)
  | TDisjunction (lhs, rhs)
  | TArrayIndex (lhs, rhs, _)
  | TSpanIndex (lhs, rhs, _) ->
     (expr_edges module_name lhs) @ (expr_edges module_name rhs)
  | TIfExpression (c, t, f) ->
     (expr_edges module_name c) @ (expr_edges module_name t) @ (expr_edges module_name f)
  | TRecordConstructor (_, fields)
  | TUnionConstructor (_, _, fields) ->
     List.concat_map (fun (_, v) -> expr_edges module_name v) fields
  | TEmbed (_, _, _) ->
     violation module_name "rule 3"
       "`@embed` performs unmediated foreign code execution: package modules may not embed code"

(* Collect the call-graph edges of one statement, rejecting `while` (rule 1)
   on sight. *)
let rec stmt_edges (module_name: module_name) (s: tstmt): node list =
  match s with
  | TSkip _ ->
     []
  | TLet (_, _, _, _, body) ->
     stmt_edges module_name body
  | TDestructure (_, _, _, e, body) ->
     (expr_edges module_name e) @ (stmt_edges module_name body)
  | TAssign (_, lhs, rhs) ->
     (expr_edges module_name lhs) @ (expr_edges module_name rhs)
  | TAssignVar (_, _, e)
  | TInitialAssign (_, e) ->
     expr_edges module_name e
  | TIf (_, c, t, f) ->
     (expr_edges module_name c) @ (stmt_edges module_name t) @ (stmt_edges module_name f)
  | TCase (_, e, whens, _) ->
     (expr_edges module_name e)
     @ (List.concat_map (fun (TypedWhen (_, _, body)) -> stmt_edges module_name body) whens)
  | TWhile _ ->
     violation module_name "rule 1"
       "`while` loops are rejected in package modules: v1 cannot prove an induction variable's upper bound, so no `while` is admitted (use a `for` loop, which is bounded by construction)"
  | TFor (_, _, lo, hi, body) ->
     (expr_edges module_name lo) @ (expr_edges module_name hi) @ (stmt_edges module_name body)
  | TBorrow { body; _ } ->
     stmt_edges module_name body
  | TBlock (_, a, b) ->
     (stmt_edges module_name a) @ (stmt_edges module_name b)
  | TDiscarding (_, e)
  | TReturn (_, e) ->
     expr_edges module_name e
  | TLetTmp (_, _, e)
  | TAssignTmp (_, e) ->
     expr_edges module_name e

(* The definitions of this module: functions and instance methods, with
   human-readable names for error messages. *)
let module_defs (decls: typed_decl list): (node * string) list =
  List.concat_map
    (fun decl ->
      match decl with
      | TFunction (id, _, name, _, _, _, _, _, _) ->
         [(function_node id, ident_string name)]
      | TInstance (_, _, _, _, _, methods, _) ->
         List.map
           (fun (TypedMethodDef (mid, name, _, _, _)) ->
             (method_node mid, ident_string name))
           methods
      | _ ->
         [])
    decls

let node_name (defs: (node * string) list) (n: node): string =
  match List.assoc_opt n defs with
  | Some name -> name
  | None -> node_key n

(* Rule 2: reject any cycle in the intra-module call graph. Classic
   three-colour DFS; on a back edge the current path is reported as the
   offending cycle. External targets (functions from other modules) are
   not nodes of this graph and are ignored. *)
let check_acyclic (module_name: module_name) (defs: (node * string) list) (edges: (node * node list) list): unit =
  let edge_of (n: node): node list =
    match List.assoc_opt n edges with
    | Some targets -> targets
    | None -> []
  in
  let defined (n: node): bool = List.mem_assoc n defs in
  (* Black: fully explored. Gray (on_path): on the current DFS path —
     re-entering a gray node is exactly a call cycle. *)
  let visited: (node, unit) Hashtbl.t = Hashtbl.create 64 in
  let on_path: (node, unit) Hashtbl.t = Hashtbl.create 64 in
  let rec dfs (stack: node list) (n: node): unit =
    if Hashtbl.mem on_path n then begin
      (* Back edge onto the active call path. `stack` is newest-first;
         reversed it is the call order, so the cycle runs from n's first
         occurrence down to the most recent caller and back to n. *)
      let rec drop_to x = function
        | [] -> []
        | y :: rest -> if x = y then (y :: rest) else drop_to x rest
      in
      let cycle_path = (drop_to n (List.rev stack)) @ [n] in
      let names = String.concat " -> " (List.map (node_name defs) cycle_path) in
      violation module_name "rule 2"
        ("recursive call cycle: " ^ names
         ^ " — recursion (direct or mutual) is rejected in package modules: v1 recognizes no structurally descending recursion")
    end
    else if Hashtbl.mem visited n then
      ()
    else begin
      Hashtbl.add on_path n ();
      List.iter
        (fun target -> if defined target then dfs (n :: stack) target)
        (edge_of n);
      Hashtbl.remove on_path n;
      Hashtbl.add visited n ()
    end
  in
  List.iter (fun (n, _) -> dfs [] n) defs

(* Entry point: check one compiled module against the package totality
   rules. Raises a compilation error on the first violation. *)
let check_module (m: typed_module): unit =
  let (TypedModule (module_name, decls)) = m in
  with_frame "Package totality"
    (fun _ ->
      (* Rules 3 (pragmas + foreign declarations) in one pass over decls. *)
      List.iter
        (fun decl ->
          match decl with
          | TForeignFunction (_, _, name, _, _, _, _, _) ->
             violation module_name "rule 3"
               ("foreign function `" ^ ident_string name
                ^ "`: package modules may not declare foreign functions (all I/O must take an explicit capability reference)")
          | TFunction (_, _, name, _, _, _, body, _, pragmas) ->
             List.iter (check_pragma module_name name) pragmas;
             ignore (stmt_edges module_name body)
          | TInstance (_, _, _, _, _, methods, _) ->
             List.iter
               (fun (TypedMethodDef (_, _name, _, _, body)) ->
                 ignore (stmt_edges module_name body))
               methods
          | _ ->
             ())
        decls;
      (* Rules 1 and 2: build the intra-module call graph (the walks above
         rejected `while`, `@embed`, foreign calls on sight) and reject any
         recursion. *)
      let defs = module_defs decls in
      let edges =
        List.concat_map
          (fun decl ->
            match decl with
            | TFunction (id, _, _, _, _, _, body, _, _) ->
               [(function_node id, stmt_edges module_name body)]
            | TInstance (_, _, _, _, _, methods, _) ->
               List.map
                 (fun (TypedMethodDef (mid, _, _, _, body)) ->
                   (method_node mid, stmt_edges module_name body))
                 methods
            | _ ->
               [])
          decls
      in
      check_acyclic module_name defs edges)
