(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L3: the totality gate. Normative is T1-T5 of
   `docs/LIQUID.md` §4.1. A function carrying a `Liquid_*` contract is a
   *refinement-level* function: its body must stay inside the Total Austral
   Subset, so every contract it states denotes a terminating computation and
   can be handed to Why3 as a conservative definitional extension without a
   well-foundedness proof.

   The gate runs on the **typed** tree (`tstmt`), not the monomorphic one:
   `Compiler_plugin.typed_pass` sees a `Tast.typed_module`, and that is the
   only seam L1 provides. This is also the better level for T1 — at `Tast`
   every call is a single `TFuncall` carrying a `decl_id`, so the call graph
   needs only one namespace. (Below `Monomorphize` a generic call would carry
   only a `mono_id`, with no back-pointer to its source declaration, and the
   graph would have to track both and could not name the generic nodes.)

   Enforced here:

   - **T1 (no recursion)** — a cycle in the module's call graph, reported with
     the whole path rather than just the back edge, so the diagnostic names
     every function on the cycle. Mirrors `validate.rs::check_cycles` in
     `../unfer/logos/src/austral_codegen`.
   - **T2 (iteration is `fold` only)** — `TWhile` and `TFor` are rejected
     wherever they appear in a refinement-level body.
   - **T4 (purity)** — a refinement-level function may not call a
     `TForeignFunction`. That is how every `uk_*` kernel symbol and every
     `@embed` wrapper reaches a body, so rejecting foreign callees is the
     whole of T4 that the typed AST can see.

   Not enforced here, on purpose:

   - **T3 (structural patterns only)** — needs an analysis of `case` arm
     guards, which `typed_when` does not yet expose. Deferred.
   - **T5 (refined variables are immutable)** — needs the refinement
     environment Γ from L4: L3 runs before any constraint exists, so there is
     nothing to compare an assignment against. L4 owns it.

   A function with no `Liquid_*` pragma is outside the fragment and is left
   alone: the gate never rejects code that did not opt in.
*)
open Stages.Tast

(* ── traversal ────────────────────────────────────────────────────────────── *)

(** Every call in a body, with nested expressions followed so a call in an
    argument position still counts. *)
let rec calls_in_expr (acc : Id.decl_id list) (e : texpr) : Id.decl_id list =
  match e with
  | TFuncall (id, _, args, _, _) ->
     List.fold_left calls_in_expr (id :: acc) args
  | TFunVar (id, _, _) -> id :: acc
  | TCast (e', _) | TNegation e' | TSlotAccessor (e', _, _)
  | TPointerSlotAccessor (e', _, _) ->
     calls_in_expr acc e'
  | TComparison (_, a, b) | TConjunction (a, b) | TDisjunction (a, b) ->
     calls_in_expr (calls_in_expr acc a) b
  | TIfExpression (c, a, b) ->
     calls_in_expr (calls_in_expr (calls_in_expr acc c) a) b
  | TRecordConstructor (_, fs) | TUnionConstructor (_, _, fs) ->
     List.fold_left (fun acc (_, e') -> calls_in_expr acc e') acc fs
  | _ -> acc

and calls_in_stmt (acc : Id.decl_id list) (s : tstmt) : Id.decl_id list =
  match s with
  | TSkip _ -> acc
  | TLet (_, _, _, _, s') -> calls_in_stmt acc s'
  | TLetTmp (_, _, e) | TAssignTmp (_, e) -> calls_in_expr acc e
  | TAssignVar (_, _, e) -> calls_in_expr acc e
  | TDestructure (_, _, _, e, s') -> calls_in_stmt (calls_in_expr acc e) s'
  | TAssign (_, a, b) -> calls_in_expr (calls_in_expr acc a) b
  | TInitialAssign (_, e) -> calls_in_expr acc e
  | TIf (_, c, t, f) -> calls_in_stmt (calls_in_stmt (calls_in_expr acc c) t) f
  | TCase (_, e, whens, _) ->
     List.fold_left
       (fun acc (TypedWhen (_, _, body)) -> calls_in_stmt acc body)
       (calls_in_expr acc e)
       whens
  | TWhile (_, c, body) | TFor (_, _, _, c, body) ->
     calls_in_stmt (calls_in_expr acc c) body
  | TBorrow { body; _ } -> calls_in_stmt acc body
  | TBlock (_, a, b) -> calls_in_stmt (calls_in_stmt acc a) b
  | TDiscarding (_, e) | TReturn (_, e) -> calls_in_expr acc e

(** Does the body contain a loop form? T2: the only iteration construct in the
    fragment is `fold`, plus structural `match`. *)
let rec loops_in_stmt (s : tstmt) : bool =
  match s with
  | TWhile _ | TFor _ -> true
  | TLet (_, _, _, _, s') -> loops_in_stmt s'
  | TDestructure (_, _, _, _, s') -> loops_in_stmt s'
  | TIf (_, _, t, f) -> loops_in_stmt t || loops_in_stmt f
  | TCase (_, _, whens, _) ->
     List.exists (fun (TypedWhen (_, _, body)) -> loops_in_stmt body) whens
  | TBorrow { body; _ } -> loops_in_stmt body
  | TBlock (_, a, b) -> loops_in_stmt a || loops_in_stmt b
  | _ -> false

(* ── T1: cycle detection ─────────────────────────────────────────────────── *)

let key (id: Id.decl_id) : int =
  match id with
  | Id.DeclId i -> i

(** Depth-first search returning the cycle *with* its closing edge, so the
    caller can name every function on it. `path` is the DFS stack and `on_path`
    the set of its nodes. *)
let rec find_cycle (graph : (int, int list) Hashtbl.t)
    (on_path : (int, unit) Hashtbl.t) (path : int list) (n : int)
  : int list option =
  match Hashtbl.find_opt graph n with
  | None -> None
  | Some succs ->
     List.fold_left
       (fun found m ->
         match found with
         | Some _ -> found
         | None ->
            if Hashtbl.mem on_path m then Some (List.rev path @ [ m ])
            else begin
              Hashtbl.replace on_path m ();
              let r = find_cycle graph on_path (m :: path) m in
              Hashtbl.remove on_path m;
              r
            end)
       None succs

(* ── entry point ─────────────────────────────────────────────────────────── *)

(** Check one refinement-level function, returning [None] when it is total or
    the reason it is not. *)
let check_function ~(name : string) ~(entry : Id.decl_id) ~(body : tstmt)
    ~(graph : (int, int list) Hashtbl.t)
    ~(foreign : (int, unit) Hashtbl.t)
    ~(names : (int, string) Hashtbl.t) : string option =
  (* T2 before T1: a loop is a local, unambiguous defect, and reporting it
     before walking the graph keeps the diagnostic to one sentence. *)
  if loops_in_stmt body then
    Some
      (Printf.sprintf
         "T2 (§4.1): `%s` uses `while`/`for`; the Total Austral Subset admits \
          only `fold` over lists and structural `match` as iteration"
         name)
  else
    (* T4: no foreign callee. *)
    let foreign_callee =
      List.find_opt (fun id -> Hashtbl.mem foreign (key id)) (calls_in_stmt [] body)
    in
    (match foreign_callee with
     | Some _ ->
        Some
          (Printf.sprintf
             "T4 (§4.1): `%s` calls a foreign function; a refinement-level \
              function must be pure — no `uk_*` symbols, no `@embed` wrappers"
             name)
     | None ->
         (* T1: cycle detection over the module's graph. *)
         let entry = key entry in
         (match find_cycle graph (Hashtbl.create 16) [ entry ] entry with
          | None -> None
          | Some path ->
             let rendered =
               String.concat " -> "
                 (List.map
                    (fun k ->
                      match Hashtbl.find_opt names k with
                      | Some s -> s
                      | None -> Printf.sprintf "decl#%d" k)
                    path)
             in
             Some
               (Printf.sprintf
                  "T1 (§4.1): `%s` is in a call cycle, so it is not total: %s"
                  name rendered)))