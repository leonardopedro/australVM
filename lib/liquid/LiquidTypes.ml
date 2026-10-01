(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L2: the Liquid Austral refinement AST (the target of
   the TRL parser in `LiquidParse`). Normative for this file is
   `docs/LIQUID.md` §2.1 (sorts), §2.2 (grammar) and §2.3 (well-formedness).

   The AST is deliberately a *surface* tree over sorts, not a Why3 term: the
   erasure principle of §1.2 means nothing here survives to codegen, and L4
   lowers this to `.mlw`. Two deliberate omissions, both normative:

   - there are **no quantifiers** (§2.2: `forall`/`exists` are MUST NOT in
     v1), so there is no term node for one;
   - there is no binder for `let`, for the same reason the grammar has none.
*)

(** The sorts of `docs/LIQUID.md` §2.1. [List of] and [ADT] are the only
    structured ones; everything else is scalar. *)
type sort =
  | SInt
  | SNat
  | SBool
  | SReal
  | SProb
  | SList of sort
  | SAdt of string

(** A position inside the contract string, so a diagnostic can point at the
    offending characters rather than at the pragma as a whole (§2.2: errors
    are "span-accurate ... into the pragma string"). Character offsets, not
    bytes: the DSL is ASCII by construction (identifiers are Austral
    identifiers, the only non-ASCII token is spelled `result`). *)
type span = {
  start : int;
  stop  : int;
}

let no_span = { start = 0; stop = 0 }

let span_of_string (s : string) : span =
  { start = 0; stop = String.length s }

(** A refinement term (§2.2 `trl-term`). *)
type term =
  | TInt of Z.t * span
  | TReal of float * span
  | TBool of bool * span
  | TVar of string * sort option * span
  | TAdd of term * term * span
  | TSub of term * term * span
  | TMul of term * term * span
  | TNeg of term * span
  | TIf of term * term * term * span
  | TCall of string * term list * span

(** A refinement formula (§2.2 `trl-formula`). *)
type formula =
  | FTrue of span
  | FFalse of span
  | FRel of relop * term * term * span
  | FNot of formula * span
  | FAnd of formula * formula * span
  | FOr of formula * formula * span
  | FImplies of formula * formula * span

and relop =
  | REq
  | RNe
  | RLt
  | RLe
  | RGt
  | RGe

(** The six `Liquid_*` contract kinds of `docs/LIQUID.md` §3. [CKind] carries
    no payload: the marker kinds are recognised from the pragma name, not from
    a contract string. *)
type contract_kind =
  | KRequires
  | KEnsures
  | KInvariant
  | KTrusted
  | KMeasure
  | KFold

let string_of_contract_kind = function
  | KRequires -> "Requires"
  | KEnsures -> "Ensures"
  | KInvariant -> "Invariant"
  | KTrusted -> "Trusted"
  | KMeasure -> "Measure"
  | KFold -> "Fold"

let contract_kind_of_string = function
  | "Requires" -> Some KRequires
  | "Ensures" -> Some KEnsures
  | "Invariant" -> Some KInvariant
  | "Trusted" -> Some KTrusted
  | "Measure" -> Some KMeasure
  | "Fold" -> Some KFold
  | _ -> None

(** A parsed contract: the kind it was written under, the whole contract
    string (for diagnostics and for the erasure point of §1.2), and the
    formula. Marker kinds carry no formula. *)
type contract = {
  kind : contract_kind;
  source : string;
  formula : formula option;
}

(* ── rendering, for errors and for the golden-key/test harnesses ────────── *)

(** Terms and formulas are variants whose *last* component is the span, not
    records, so OCaml record projection does not apply to them. These
    accessors are how the parser recovers a node's extent for a diagnostic. *)
let rec span_of_term (t : term) : span =
  match t with
  | TInt (_, sp) | TReal (_, sp) | TBool (_, sp) | TVar (_, _, sp)
  | TNeg (_, sp) | TCall (_, _, sp) -> sp
  | TAdd (_, _, sp) | TSub (_, _, sp) | TMul (_, _, sp) | TIf (_, _, _, sp) -> sp

let rec span_of_formula (f : formula) : span =
  match f with
  | FTrue sp | FFalse sp | FNot (_, sp) | FRel (_, _, _, sp)
  | FAnd (_, _, sp) | FOr (_, _, sp) | FImplies (_, _, sp) -> sp

let string_of_relop = function
  | REq -> "=="
  | RNe -> "!="
  | RLt -> "<"
  | RLe -> "<="
  | RGt -> ">"
  | RGe -> ">="

let rec string_of_term = function
  | TInt (z, _) -> Z.to_string z
  | TReal (f, _) -> Printf.sprintf "%g" f
  | TBool (b, _) -> if b then "true" else "false"
  | TVar (v, _, _) -> v
  | TAdd (a, b, _) -> "(" ^ (string_of_term a) ^ " + " ^ (string_of_term b) ^ ")"
  | TSub (a, b, _) -> "(" ^ (string_of_term a) ^ " - " ^ (string_of_term b) ^ ")"
  | TMul (a, b, _) -> "(" ^ (string_of_term a) ^ " * " ^ (string_of_term b) ^ ")"
  | TNeg (a, _) -> "-" ^ (string_of_term a)
  | TIf (c, a, b, _) ->
     "(if " ^ (string_of_term c) ^ " then " ^ (string_of_term a)
     ^ " else " ^ (string_of_term b) ^ ")"
  | TCall (f, args, _) ->
     f ^ "(" ^ (String.concat ", " (List.map string_of_term args)) ^ ")"

let rec string_of_formula = function
  | FTrue _ -> "true"
  | FFalse _ -> "false"
  | FRel (op, a, b, _) ->
     "(" ^ (string_of_term a) ^ " " ^ (string_of_relop op) ^ " "
     ^ (string_of_term b) ^ ")"
  | FNot (f, _) -> "!(" ^ (string_of_formula f) ^ ")"
  | FAnd (a, b, _) -> "(" ^ (string_of_formula a) ^ " && " ^ (string_of_formula b) ^ ")"
  | FOr (a, b, _) -> "(" ^ (string_of_formula a) ^ " || " ^ (string_of_formula b) ^ ")"
  | FImplies (a, b, _) ->
     "(" ^ (string_of_formula a) ^ " ==> " ^ (string_of_formula b) ^ ")"

(** The free variables of a formula, for WF1 (§2.3): every free variable must
    be a parameter, `result`, or a total-fragment module constant. `result`
    itself is excluded — it is bound by the enclosing declaration, not free. *)
let rec free_vars_term (acc : string list) (t : term) : string list =
  match t with
  | TInt _ | TReal _ | TBool _ -> acc
  | TVar (v, _, _) ->
     if List.mem v acc || v = "result" then acc else v :: acc
  | TAdd (a, b, _) | TSub (a, b, _) | TMul (a, b, _) ->
     free_vars_term (free_vars_term acc a) b
  | TNeg (a, _) -> free_vars_term acc a
  | TIf (c, a, b, _) -> free_vars_term (free_vars_term (free_vars_term acc c) a) b
  | TCall (_, args, _) -> List.fold_left free_vars_term acc args

(** Merge two name lists, preserving first-appearance order and de-duplicating.
    `result` is bound by the enclosing declaration, so it is never reported as
    free even if it appears. *)
let union_names (a : string list) (b : string list) : string list =
  List.fold_left
    (fun (acc : string list) (v : string) ->
      if List.mem v acc || v = "result" then acc else v :: acc)
    a b

let rec free_vars (f : formula) : string list =
  match f with
  | FTrue _ | FFalse _ -> []
  | FRel (_, a, b, _) -> free_vars_term (free_vars_term [] a) b
  | FNot (g, _) -> free_vars g
  | FAnd (a, b, _) | FOr (a, b, _) | FImplies (a, b, _) ->
     union_names (free_vars a) (free_vars b)

(* ── WF1 / WF2 checks that need no environment ──────────────────────────── *)

(** WF1 (§2.3): check every free variable against the names in scope — the
    declaration's parameters, `result`, and total-fragment constants. Returns
    the offending variables, in order of first appearance. *)
let check_wf1 (f : formula) ~(bound : string list) : string list =
  List.filter (fun v -> not (List.mem v bound)) (free_vars f)

(** WF2 (§2.3), partially: every variable mentioned must have been assigned a
    sort by the declaration site. A variable with no declared sort is
    reported, because `TVar` carries `sort option` precisely so this can be
    deferred to the typing stage where the parameter types are known. *)
let rec undeclared_sorts_term (acc : string list) (t : term) : string list =
  match t with
  | TInt _ | TReal _ | TBool _ -> acc
  | TVar (v, None, _) ->
     if List.mem v acc then acc else v :: acc
  | TVar _ -> acc
  | TAdd (a, b, _) | TSub (a, b, _) | TMul (a, b, _) ->
     undeclared_sorts_term (undeclared_sorts_term acc a) b
  | TNeg (a, _) -> undeclared_sorts_term acc a
  | TIf (c, a, b, _) ->
     undeclared_sorts_term (undeclared_sorts_term (undeclared_sorts_term acc c) a) b
  | TCall (_, args, _) -> List.fold_left undeclared_sorts_term acc args

let rec undeclared_sorts (f : formula) : string list =
  match f with
  | FTrue _ | FFalse _ -> []
  | FRel (_, a, b, _) -> undeclared_sorts_term (undeclared_sorts_term [] a) b
  | FNot (g, _) -> undeclared_sorts g
  | FAnd (a, b, _) | FOr (a, b, _) | FImplies (a, b, _) ->
     union_names (undeclared_sorts a) (undeclared_sorts b)