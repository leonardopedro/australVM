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

(** L9: index-carrying types, declared here because [formula] refers to them.

    The narrative — why a type is not a term, and what coherence means here —
    is in the note above [ty_wf] further down, next to the functions that use
    it. *)
type ty =
  | TySort of sort
  (** `Span[Nat8, n]`, `Vector[Nat8, 8]`. *)
  | TyApp of string * ty_arg list * span

(** A bracketed argument is either a type or an index term. The distinction is
    explicit rather than inferred from a sort, so a malformed annotation is
    reportable instead of silently reinterpreted. *)
and ty_arg =
  | TyArgTy of ty
  | TyArgIndex of term


(** A refinement formula (§2.2 `trl-formula`). *)
type formula =
  | FTrue of span
  | FFalse of span
  | FRel of relop * term * term * span
  | FNot of formula * span
  | FAnd of formula * formula * span
  | FOr of formula * formula * span
  | FImplies of formula * formula * span
  (** L9: `e : T`. A *type annotation*, not a relation — the claim is that the
      program variable [e] inhabits the index-carrying type [T]. Kept apart from
      [FRel] so the two can never be confused downstream: a relation is
      arithmetic over values, an annotation is a type-level fact about one. *)
  | FType of term * ty * span

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
  | FAnd (_, _, sp) | FOr (_, _, sp) | FImplies (_, _, sp)
  | FType (_, _, sp) -> sp

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

let rec string_of_sort (s : sort) : string =
  match s with
  | SInt -> "Int"
  | SNat -> "Nat"
  | SBool -> "Bool"
  | SReal -> "Real"
  | SProb -> "Prob"
  | SList s -> "List[" ^ string_of_sort s ^ "]"
  | SAdt n -> n

let describe_index (i : term) : string =
  match i with
  | TReal (_, _) -> "a real"
  | TBool (_, _) -> "a boolean"
  | TVar (_, None, _) -> "a variable with no declared sort"
  | TVar (_, Some s, _) -> "a variable declared " ^ string_of_sort s
  | _ -> "not a nat"

let rec ty_equal (a : ty) (b : ty) : bool =
  match (a, b) with
  | TySort s, TySort s' -> s = s'
  | TyApp (na, aa, _), TyApp (nb, ab, _) ->
      na = nb
      && List.length aa = List.length ab
      && List.for_all2
           (fun x y ->
             match (x, y) with
             | TyArgTy x, TyArgTy y -> ty_equal x y
             | TyArgIndex x, TyArgIndex y -> string_of_term x = string_of_term y
             | _ -> false)
           aa ab
  | _ -> false

let rec ty_indices (t : ty) : term list =
  match t with
  | TySort _ -> []
  | TyApp (_, args, _) ->
      List.concat
        (List.map
           (function
             | TyArgTy ty -> ty_indices ty
             | TyArgIndex i -> [ i ])
           args)

let rec ty_head (t : ty) : string option =
  match t with
  | TySort _ -> None
  | TyApp (name, _, _) -> Some name

let ty_index_is_nat (i : term) : bool =
  match i with
  | TInt (z, _) -> Z.sign z >= 0
  | TVar (_, Some SNat, _) -> true
  | _ -> false

let rec ty_wf (t : ty) : (string * span) option =
  match t with
  | TySort _ -> None
  | TyApp (name, args, sp) ->
      let rec go = function
        | [] -> None
        | arg :: rest -> (
            match arg with
            | TyArgTy ty -> (
                match ty_wf ty with
                | None -> go rest
                | Some e -> Some e)
            | TyArgIndex i ->
                if ty_index_is_nat i then go rest
                else
                  let msg =
                    Printf.sprintf
                      "index of `%s` must be a non-negative literal or a variable declared Nat, but `%s` is %s"
                      name (string_of_term i) (describe_index i)
                  in
                  Some (msg, sp))
      in
      go args

let rec ty_to_string (t : ty) : string =
  match t with
  | TySort s -> string_of_sort s
  | TyApp (name, [], _) -> name
  | TyApp (name, args, _) ->
      let one = function
        | TyArgTy ty -> ty_to_string ty
        | TyArgIndex i -> string_of_term i
      in
      Printf.sprintf "%s[%s]" name (String.concat ", " (List.map one args))

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
  | FType (e, t, _) -> "(" ^ (string_of_term e) ^ " : " ^ (ty_to_string t) ^ ")"

(** L9: an index-carrying type over TRL terms.

    A type may take **indices**, which are [term]s rather than types:
    `Span[Nat8, n]` and `Vector[Nat8, 8]` name a value whose extent is itself a
    refinement-level expression. That is what makes the layer worth having — an
    index fixed at type-check time cannot be related to a program variable, so
    nothing can be said about two spans of the same element type and different
    lengths.

    ## Why a separate [ty] and not a [term]

    A type is not a value. Reusing [term] would make `Span[Nat8, n]` something
    you could add to an integer, and erasure would become a rule to remember
    rather than a property of the representation. Kept apart, a type has **no
    term projection at all** — there is no function from [ty] to [term] — so an
    index cannot reach codegen by construction. Erasure here is structural, not
    a pass someone has to remember to run.

    ## Coherence

    Two index-carrying types coincide only when their indices do, which is why
    indices are terms and not integers: `Span[Nat8, n]` and `Span[Nat8, 8]` are
    distinguishable, and relating them is a refinement obligation rather than a
    syntactic test. [ty_equal] below is deliberately the *syntactic* equality.

    ## Index well-formedness

    An index must be a nat. The check is **conservative and under-claiming**,
    matching the rest of this layer: a non-negative literal and a variable
    declared `Nat` are accepted, and everything else is refused with a reason.
    In particular an *unannotated* variable is refused rather than assumed,
    because guessing here would let `Span[Nat8, x]` through on the chance that
    `x` happens to be an integer, and a non-integer extent is not an extent. *)
(** Syntactic equality. Two types with different index *terms* differ even if
    the terms happen to denote the same number: telling them apart needs a
    refinement proof, and treating them as equal would make the layer unsound in
    the direction that loses information. *)
(** The indices of a type, left to right. *)
(** The head name of an applied type, e.g. `Some` for `Span[Nat8, n]`. *)
(** Can this term serve as an index? Conservative: see the note above. *)
(** Why an index was refused, in the caller's words. *)
(** L9 well-formedness for a type annotation. [None] is well-formed; [Some] is
    a span-accurate reason, as §2.2 requires. *)



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
  | FType (e, t, _) ->
     (* The index terms count. `x : Span[Nat8, n]` mentions `n`, and WF1
        requires every free variable to be in scope — an index is a term and
        is subject to the same rule as any other. Forgetting this would let a
        contract refer to a variable the declaration does not have. *)
     let rec ty_vars acc = function
       | TySort _ -> acc
       | TyApp (_, args, _) ->
           List.fold_left
             (fun acc -> function
               | TyArgTy ty -> ty_vars acc ty
               | TyArgIndex i -> free_vars_term acc i)
             acc args
     in
     ty_vars (free_vars_term [] e) t

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
  | FType (e, t, _) ->
     (* Same reasoning as `free_vars`: an index term may carry a variable whose
        sort was never declared, and WF2 has to see it. *)
     let rec ty_undeclared acc = function
       | TySort _ -> acc
       | TyApp (_, args, _) ->
           List.fold_left
             (fun acc -> function
               | TyArgTy ty -> ty_undeclared acc ty
               | TyArgIndex i -> undeclared_sorts_term acc i)
             acc args
     in
     ty_undeclared (undeclared_sorts_term [] e) t