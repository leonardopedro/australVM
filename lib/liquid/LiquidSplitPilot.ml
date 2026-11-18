(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L11c: check indexed contracts at calls and in branches,
   for one pilot wrapper.

   ## What the pilot is

   A module opts in by declaring, with ordinary Austral and the ordinary
   `Liquid_*` pragmas:

     - a record with exactly the slots `base`, `off`, `len` — the audited
       range/span wrapper;
     - `length(s)` and `offset(s)`, each of which must *be* the corresponding
       slot read (`return s.len;` / `return s.off;`), so a contract that says
       `length(s)` is talking about the representation and not about a fiction;
     - `split(s, k)` returning a union with a two-span case, whose body builds
       the two halves.

   Nothing else in the module changes. A module without that shape is left
   alone: the pass returns [None] and the compiler behaves exactly as before.

   ## What is checked, and why in this order

   1. **The measures are the representation.** `length`/`offset` are matched
      structurally against the slot reads. A contract over a measure that is
      not the field would be a refinement of something the program does not
      have, so the pilot refuses to reason about it.

   2. **The split's own body is verified, not assumed.** The body must build
      the left half with `off = offset(s)` and `len = k`, and the right half
      with `off = offset(s) + k` and `len = length(s) - k`, with both halves
      sharing `base`. From those four terms the pilot *derives* contiguity
      (`left.off + left.len = right.off`) and the total-length law
      (`left.len + right.len = length(s)`), so an implementation that overlaps
      its halves or loses a byte is refused at its definition. This is the
      "constructor/checker contract" the plan asks the branch facts to come
      from: they are the verified body's terms, instantiated at the call.

   3. **Call sites prove the callee's `Requires`.** The requirement is
      instantiated with the actuals (capture-avoiding: v1 has no quantifiers,
      so substitution cannot capture) and discharged by, in order: conversion
      in the L11b index language, literal arithmetic, or a fact the caller
      already assumes. Anything else is refused — the pilot has no prover, and
      "unknown" must not justify a split.

   4. **A `case` on a split result gets the halves' facts.** In the two-span
      branch, `left.len = k`, `right.len = length(s) - k`, `left.off =
      offset(s)`, `right.off = offset(s) + k`, and the disjointness
      `left.off + left.len <= right.off` — the last one only after the pilot
      *checks* that both sides rewrite to the same term, so it is derived
      rather than asserted.

   5. **Facts are scoped.** Branch facts live in a scope that is popped at the
      end of the branch, and the binders they mention die with it, so evidence
      cannot leak by construction. When a goal fails but a *dropped* fact would
      have discharged it, the refusal says so, because "cannot be discharged"
      and "you are using branch-local evidence" are different mistakes.

   6. **Measurements are snapshotted before the move.** The facts are about
      `length(s)`/`offset(s)` as *values* (fresh binders), not about the
      variable `s`. Moving `s` into `split` therefore does not invalidate them,
      which is what lets a caller keep reasoning about the extent it measured
      before the transfer. Using the moved value again is the linear type
      checker's refusal, not this pass's, and the pilot does not duplicate it.

   ## What is deliberately not here

   - No prover: a symbolic obligation the facts do not settle is a refusal, not
     a deferred goal. L11d is where a stated law gets a checked proof.
   - No arithmetic beyond L11b's conversion rules and fact rewriting, so
     `length(s) - k + k = length(s)` is *not* discharged. That is a real limit
     and it is reported as one, never as a pass.
   - Functions whose bodies use constructs outside the pilot fragment are
     skipped (the pass is inert for them), which is the under-claiming
     direction: a skipped function is not a checked one.
*)

open Stages.Tast

exception Refuse of string

(** Raised for a construct the pilot cannot read. Distinct from [Refuse] on
    purpose: a function outside the fragment is *skipped* (the pass is inert
    for it), while a [Refuse] is a failed check and must reach the user. The
    first version of this module caught both, which silently turned every
    caller-side refusal into "not analysed" — a green result for code nobody
    had looked at. *)
exception Outside of string

let refuse fmt = Printf.ksprintf (fun m -> raise (Refuse m)) fmt
let outside fmt = Printf.ksprintf (fun m -> raise (Outside m)) fmt

(* ── names and types ─────────────────────────────────────────────────────── *)

let name_of (n : Identifier.identifier) : string = Identifier.ident_string n

let qname (q : Identifier.qident) : string =
  Identifier.ident_string (Identifier.original_name q)

let ty_is_named (t : Type.ty) (n : string) : bool =
  match t with Type.NamedType (q, _, _) -> qname q = n | _ -> false

let ty_is_int64 (t : Type.ty) : bool =
  match t with Type.Integer (_, Type.Width64) -> true | _ -> false

let slot_name (Type.TypedSlot (n, _)) : string = name_of n

let slot_ty (Type.TypedSlot (_, t)) : Type.ty = t

(* ── the recognised pilot ────────────────────────────────────────────────── *)

type pilot = {
  span_ty : string;
  split_fn : string;
  both_case : string;
  len_fn : string;
  off_fn : string;
}

let span_slots = [ "base"; "off"; "len" ]

let find_record (decls : typed_decl list) : (string * Type.typed_slot list) option =
  let rec go = function
    | [] -> None
    | TRecord (_, _, n, _, _, slots, _) :: rest ->
       if List.map slot_name slots = span_slots then Some (name_of n, slots) else go rest
    | _ :: rest -> go rest
  in
  go decls

let find_union_case (decls : typed_decl list) (span_ty : string) :
    (string * string) option =
  let two_spans (slots : Type.typed_slot list) =
    List.length slots = 2 && List.for_all (fun s -> ty_is_named (slot_ty s) span_ty) slots
  in
  let rec go = function
    | [] -> None
    | TUnion (_, _, un, _, _, cases, _) :: rest -> (
        match
          List.find_opt
            (fun (Stages.Linked.LCase (_, _, slots)) -> two_spans slots)
            cases
        with
        | Some (Stages.Linked.LCase (_, cn, _)) -> Some (name_of un, name_of cn)
        | None -> go rest)
    | _ :: rest -> go rest
  in
  go decls

(** The variable a projection starts from, seeing through the reference a
    borrowed parameter is read through. *)
let rec project_base (e : texpr) : string option =
  match e with
  | TParamVar (v, _) | TLocalVar (v, _) | TTemporary (v, _) -> Some (name_of v)
  | TDeref e' | TCast (e', _) -> project_base e'
  | _ -> None

(** `s_tmp_ref1` is how the typer spells "the borrow of `s`". Accepting a
    prefix rather than the exact generated suffix keeps the pass from pinning a
    name the typer is free to change. *)
let is_borrow_of (param : string) (v : string) : bool =
  v = param
  || (String.length v > String.length param
      && String.sub v 0 (String.length param) = param)

let rec project_slot (e : texpr) : (string * string) option =
  match e with
  | TSlotAccessor (e', s, _) -> (
      match project_base e' with Some v -> Some (v, name_of s) | None -> None)
  | TDeref e' | TCast (e', _) -> project_slot e'
  | _ -> None

(** A measure is grounded when its body is exactly `return <param>.<slot>;`,
    wherever the reference machinery has put the deref. *)
let measure_reads_slot (decls : typed_decl list) (fn : string) (span_ty : string)
    (slot : string) : bool =
  let rec body_expr (s : tstmt) : texpr option =
    match s with
    | TReturn (_, e) -> Some e
    | TBorrow { body; _ } -> body_expr body
    | TBlock (_, a, _) -> body_expr a
    | _ -> None
  in
  let rec go = function
    | [] -> false
    | TFunction (_, _, n, _, params, _, body, _, _) :: rest ->
       if name_of n <> fn then go rest
       else
         let param =
           match params with
           | Type.ValueParameter (p, t) :: _ when ty_is_named t span_ty ->
              Some (name_of p)
           | _ -> None
         in
         (match (param, body_expr body) with
          | Some p, Some e -> (
              match project_slot e with
              | Some (v, s) -> s = slot && is_borrow_of p v
              | None -> false)
          | _ -> false)
    | _ :: rest -> go rest
  in
  go decls

let recognize (decls : typed_decl list) : pilot option =
  match find_record decls with
  | None -> None
  | Some (span_ty, _) -> (
      match find_union_case decls span_ty with
      | None -> None
      | Some (_, both_case) ->
         let has_split =
           List.exists
             (function
               | TFunction (_, _, n, _, params, rt, _, _, _) ->
                  name_of n = "split"
                  && (match params with
                      | [ Type.ValueParameter (_, a); Type.ValueParameter (_, b) ] ->
                         ty_is_named a span_ty && ty_is_int64 b
                      | _ -> false)
                  && (match rt with Type.NamedType _ -> true | _ -> false)
               | _ -> false)
             decls
         in
         if
           has_split
           && measure_reads_slot decls "length" span_ty "len"
           && measure_reads_slot decls "offset" span_ty "off"
         then
           Some
             { span_ty; split_fn = "split"; both_case; len_fn = "length";
               off_fn = "offset" }
         else None)

(* ── facts ───────────────────────────────────────────────────────────────── *)

type fact_kind = FEq | FLe

type fact = { fk : fact_kind; fa : LiquidIndex.t; fb : LiquidIndex.t }

type vinfo =
  | VSpan of { len : LiquidIndex.t; off : LiquidIndex.t }
  | VInt of LiquidIndex.t
  | VOther

type split_info = { si_len : LiquidIndex.t; si_off : LiquidIndex.t; si_k : LiquidIndex.t }

type ctx = {
  mutable idx : LiquidIndex.env;
  pilot : pilot;
  mutable vars : (string * vinfo) list;
  mutable scopes : fact list list;  (** innermost first *)
  mutable dropped : (string * fact) list;
  mutable fn : string;
  mutable splits : (string * split_info) list;
  mutable loc : string option;
  mutable aliases : (string * string) list;
}

let fresh (ctx : ctx) (hint : string) : LiquidIndex.t =
  let env, id = LiquidIndex.bind ctx.idx ~name:hint ~sort:LiquidTypes.SNat in
  ctx.idx <- env;
  LiquidIndex.var id

(** A borrow renames its variable (`s` becomes `s_tmp_ref1` in the body), so a
    lookup has to follow the rename before it can find the measurement. *)
let rec resolve (ctx : ctx) (n : string) : string =
  match List.assoc_opt n ctx.aliases with Some m when m <> n -> resolve ctx m | _ -> n

let span_of (ctx : ctx) (n : string) : (LiquidIndex.t * LiquidIndex.t) option =
  match List.assoc_opt (resolve ctx n) ctx.vars with
  | Some (VSpan { len; off }) -> Some (len, off)
  | _ -> None

let int_of (ctx : ctx) (n : string) : LiquidIndex.t option =
  match List.assoc_opt (resolve ctx n) ctx.vars with Some (VInt t) -> Some t | _ -> None

let at (ctx : ctx) : string =
  match ctx.loc with None -> "" | Some l -> " at " ^ l

(* ── surface terms → index terms ─────────────────────────────────────────── *)

let rec term_of_expr (ctx : ctx) (e : texpr) : LiquidIndex.t =
  match e with
  | TIntConstant s -> (
      match int_of_string_opt s with
      | Some n when n >= 0 -> LiquidIndex.lit n
      | _ ->
         refuse
           "L11c: `%s` uses the integer literal `%s`, which is not a natural \
            index; a negative or oversized length is a typed refusal at the \
            boundary%s"
           ctx.fn s (at ctx))  | TDeref e' -> term_of_expr ctx e'
  | TParamVar (n, _) | TLocalVar (n, _) | TTemporary (n, _) ->
     (match int_of (ctx) (name_of n) with
      | Some t -> t
      | None -> (
          match span_of ctx (name_of n) with
          | Some _ ->
             outside
               "`%s` uses the span `%s` where a number is required" ctx.fn
               (name_of n)
          | None ->
             outside "`%s` has no measured value" ctx.fn (name_of n)))
  | TMethodCall (_, q, _, [ a; b ], _, _) -> (
      match qname q with
      | "trappingAdd" -> LiquidIndex.add (term_of_expr ctx a) (term_of_expr ctx b)
      | "trappingSubtract" ->
         LiquidIndex.nat_sub (term_of_expr ctx a) (term_of_expr ctx b)
      | other -> outside "`%s` calls `%s`" ctx.fn other)
  | TFuncall (_, q, [ a ], _, _) when qname q = ctx.pilot.len_fn ->
     measure_of_expr ctx ctx.pilot.len_fn a
  | TFuncall (_, q, [ a ], _, _) when qname q = ctx.pilot.off_fn ->
     measure_of_expr ctx ctx.pilot.off_fn a
  | TSlotAccessor (e', s, _) -> (
      match (e', name_of s) with
      | (TParamVar (n, _) | TLocalVar (n, _)), "len" -> (
          match span_of ctx (name_of n) with
          | Some (len, _) -> len
          | None -> outside "`%s` is not a measured span" (name_of n))
      | (TParamVar (n, _) | TLocalVar (n, _)), "off" -> (
          match span_of ctx (name_of n) with
          | Some (_, off) -> off
          | None -> outside "`%s` is not a measured span" (name_of n))
      | _ -> outside "`%s` reads a slot the pilot does not measure" ctx.fn)
  | TCast (e', _) -> term_of_expr ctx e'
  | TNegation _ ->
     (* A negation in an extent position is a *value* problem, not a shape the
        pilot cannot read: `split(w, -1)` asks for a negative length, and the
        boundary must refuse it as a typed error rather than pass it through as
        "not analysed". *)
     refuse
       "L11c: `%s` negates a length or offset; an extent is a natural, so a \
        negative input is a typed refusal at the boundary%s"
       ctx.fn (at ctx)
  | _ -> outside "`%s` uses an expression outside the pilot fragment" ctx.fn

and measure_of_expr (ctx : ctx) (which : string) (a : texpr) : LiquidIndex.t =
  match a with
  | TDeref e' | TCast (e', _) -> measure_of_expr ctx which e'
  | TParamVar (n, _) | TLocalVar (n, _) -> (
      match span_of ctx (name_of n) with
      | Some (len, off) -> if which = ctx.pilot.len_fn then len else off
      | None -> outside "`%s(%s)` measures something that is not a span" ctx.fn which (name_of n))
  | _ -> outside "`%s` is applied to an unmeasurable expression" which

(** The same translation for a contract term, with the callee's parameters
    substituted by the call's actuals. *)
let rec term_of_contract (ctx : ctx) ~(sub : (string * texpr) list)
    (t : LiquidTypes.term) : LiquidIndex.t =
  match t with
  | LiquidTypes.TInt (z, _) ->
     if Z.sign z < 0 then
       refuse
         "L11c: the contract mentions the negative literal `%s`; a length or \
          offset is a natural%s"
         (Z.to_string z) (at ctx)
     else if not (Z.fits_int z) then
       refuse "L11c: the contract literal `%s` does not fit the host integer%s"
         (Z.to_string z) (at ctx)
     else LiquidIndex.lit (Z.to_int z)
  | LiquidTypes.TVar (n, _, _) -> (
      match List.assoc_opt n sub with
      | Some e -> term_of_expr ctx e
      | None -> (
          match int_of ctx n with
          | Some t -> t
          | None -> outside "the contract mentions the unmeasured `%s`" n))
  | LiquidTypes.TAdd (a, b, _) ->
     LiquidIndex.add (term_of_contract ctx ~sub a) (term_of_contract ctx ~sub b)
  | LiquidTypes.TSub (a, b, _) ->
     LiquidIndex.nat_sub (term_of_contract ctx ~sub a) (term_of_contract ctx ~sub b)
  | LiquidTypes.TCall (f, [ arg ], _)
    when f = ctx.pilot.len_fn || f = ctx.pilot.off_fn -> (
      match arg with
      | LiquidTypes.TVar (n, _, _) -> (
          let span_terms =
            match List.assoc_opt n sub with
            | Some e -> (
                match e with
                | TParamVar (v, _) | TLocalVar (v, _) -> span_of ctx (name_of v)
                | _ -> None)
            | None -> span_of ctx n
          in
          match span_terms with
          | Some (len, off) -> if f = ctx.pilot.len_fn then len else off
          | None -> outside "`%s(%s)` measures something that is not a span" f n)
      | _ -> outside "`%s` is applied to an unmeasurable expression" f)
  | _ -> outside "the contract uses a term outside the pilot fragment" 

let rec facts_of_formula (ctx : ctx) ~(sub : (string * texpr) list)
    (f : LiquidTypes.formula) : fact list =
  match f with
  | LiquidTypes.FTrue _ -> []
  | LiquidTypes.FRel (LiquidTypes.REq, a, b, _) ->
     [ { fk = FEq; fa = term_of_contract ctx ~sub a; fb = term_of_contract ctx ~sub b } ]
  | LiquidTypes.FRel (LiquidTypes.RLe, a, b, _) ->
     [ { fk = FLe; fa = term_of_contract ctx ~sub a; fb = term_of_contract ctx ~sub b } ]
  | LiquidTypes.FAnd (a, b, _) -> facts_of_formula ctx ~sub a @ facts_of_formula ctx ~sub b
  | _ ->
     outside
       "the contract uses a connective or relation outside the pilot fragment \
        (equalities, `<=` and `&&` only)"

(* ── rewriting with assumed equalities ───────────────────────────────────── *)

let all_facts (ctx : ctx) : fact list = List.concat ctx.scopes

(* Only one direction, always: a fact whose *left* side is a bare binder
   replaces that binder. Substituting the other way too made `v18 = v15` and
   `v20 = v15 + v16` fight each other, each step undoing the previous one until
   the fuel ran out — and the answer then depended on the fuel. *)
let rewrite_once (ctx : ctx) (t : LiquidIndex.t) : LiquidIndex.t option =
  let rec go = function
    | [] -> None
    | f :: rest -> (
        match (f.fk, f.fa.LiquidIndex.node) with
        | FEq, LiquidIndex.Var id when LiquidIndex.occurs id t ->
           Some (LiquidIndex.subst_var id f.fb t)
        | _ -> go rest)
  in
  go (all_facts ctx)

let rewrite (ctx : ctx) (t : LiquidIndex.t) : LiquidIndex.t =
  let rec go fuel t =
    if fuel = 0 then t
    else match rewrite_once ctx t with None -> t | Some t' -> go (fuel - 1) t'
  in
  go 64 t

let same (a : LiquidIndex.t) (b : LiquidIndex.t) : bool =
  LiquidIndex.canonical a = LiquidIndex.canonical b

let assumption_matches (ctx : ctx) (f : fact) : bool =
  List.exists
    (fun g ->
      g.fk = f.fk
      && ((same g.fa f.fa && same g.fb f.fb) || (same g.fa f.fb && same g.fb f.fa)))
    (all_facts ctx)

let dropped_matches (ctx : ctx) (f : fact) : string option =
  let rec go = function
    | [] -> None
    | (label, g) :: rest ->
       if
         g.fk = f.fk
         && ((same g.fa f.fa && same g.fb f.fb) || (same g.fa f.fb && same g.fb f.fa))
       then Some label
       else go rest
  in
  go ctx.dropped

let literal (t : LiquidIndex.t) : int option =
  match t.LiquidIndex.node with LiquidIndex.Lit n -> Some n | _ -> None

(** Discharge one obligation. The order is the point: conversion first (it
    needs no assumption), then literal arithmetic, then the caller's own
    facts, then an explanation if a branch-local fact would have done. *)
let discharge (ctx : ctx) ~(what : string) (f : fact) : unit =
  let a = rewrite ctx f.fa and b = rewrite ctx f.fb in
  let settled =
    match f.fk with
    | FEq -> LiquidIndex.convert ctx.idx a b = LiquidIndex.Convertible
    | FLe -> (
        match (literal a, literal b) with
        | Some x, Some y -> x <= y
        | _ -> LiquidIndex.convert ctx.idx a b = LiquidIndex.Convertible)
  in
  if settled then ()
  else if assumption_matches ctx { f with fa = a; fb = b } then ()
  else
    match dropped_matches ctx { f with fa = a; fb = b } with
    | Some label ->
       refuse
         "L11c: `%s` needs `%s`, which is only justified by branch-local \
          evidence from %s; facts derived inside a `case` branch do not \
          survive it%s"
         ctx.fn (what) label (at ctx)
    | None ->
       refuse
         "L11c: `%s` cannot discharge `%s` (`%s` vs `%s`); the pilot has no \
          prover, and an unknown obligation does not justify the claim%s"
         ctx.fn what
         (LiquidIndex.canonical a) (LiquidIndex.canonical b) (at ctx)

(* ── the split's own body ────────────────────────────────────────────────── *)

type built = { b_off : LiquidIndex.t; b_len : LiquidIndex.t; b_base : string }

(** The `base` slot is a pointer/address, not a number: the pilot compares it
    as a *projection text* (`s.base`), which is exactly the common-origin
    claim, instead of pretending it has an integer value. *)
let rec projection (ctx : ctx) (e : texpr) : string option =
  match e with
  | TDeref e' | TCast (e', _) -> projection ctx e'
  | TSlotAccessor (e', s, _) -> (
      match project_base e' with
      | Some v -> Some (resolve ctx v ^ "." ^ name_of s)
      | None -> None)
  | TParamVar (v, _) | TLocalVar (v, _) | TTemporary (v, _) ->
     Some (resolve ctx (name_of v))
  | _ -> None

let field (ctx : ctx) (fs : (Identifier.identifier * texpr) list) (n : string) :
    LiquidIndex.t =
  match List.assoc_opt n (List.map (fun (k, v) -> (name_of k, v)) fs) with
  | Some e -> term_of_expr ctx e
  | None -> refuse "L11c: `%s` builds a span without a `%s` slot%s" ctx.fn n (at ctx)

let base_field (ctx : ctx) (fs : (Identifier.identifier * texpr) list) : string =
  match List.assoc_opt "base" (List.map (fun (k, v) -> (name_of k, v)) fs) with
  | Some e -> (
      match projection ctx e with
      | Some p -> p
      | None ->
         refuse "L11c: `%s` builds a span whose `base` is not a projection%s"
           ctx.fn (at ctx))
  | None -> refuse "L11c: `%s` builds a span without a `base` slot%s" ctx.fn (at ctx)

let rec flatten (ctx : ctx) (s : tstmt) (acc : (string * texpr) list) :
    (string * texpr) list * texpr =
  match s with
  (* [acc] is kept in reverse and flipped once, at the [TReturn]: reversing at
     every nested call flipped the order back and forth, which made the two
     halves swap places in the rule check. *)
  | TSkip _ -> (acc, TIntConstant "0")
  | TLet (_, _, _, _, body) -> flatten ctx body acc
  | TBorrow { original; rename; body; _ } ->
     (* Not restored: the field expressions are translated *after* the walk,
        and each field read is wrapped in its own borrow (`s_tmp_ref3`,
        `s_tmp_ref4`). Keeping the aliases is what lets `s.base` be recognized
        as the same origin in both halves. *)
     ctx.aliases <- (name_of rename, resolve ctx (name_of original)) :: ctx.aliases;
     flatten ctx body acc
  | TBlock (_, a, b) ->
     let acc, _ = flatten ctx a acc in
     flatten ctx b acc
  | TInitialAssign (q, e) ->
     flatten ctx (TSkip Span.empty_span)
       ((name_of (Identifier.original_name q), e) :: acc)
  | TReturn (_, e) -> (List.rev acc, e)
  | _ ->
     refuse
       "L11c: `%s` uses a statement outside the pilot fragment; the checked \
        split must be a sequence of `let` bindings and a `return`%s"
       ctx.fn (at ctx)

let verify_split (ctx : ctx) (decls : typed_decl list) : unit =
  (* The parameter names are the *declaration's*: a pilot that reads a
     hard-coded `s`/`k` silently measures nothing for any other spelling. *)
  let decl =
    let rec go = function
      | [] -> None
      | TFunction (_, _, n, _, params, _, body, _, _) :: rest ->
         if name_of n = ctx.pilot.split_fn then Some (params, body) else go rest
      | _ :: rest -> go rest
    in
    go decls
  in
  match decl with
  | None -> ()
  | Some (params, body) ->
     ctx.fn <- ctx.pilot.split_fn;
     let whole_name, k_name =
       match params with
       | [ Type.ValueParameter (a, _); Type.ValueParameter (b, _) ] ->
          (name_of a, name_of b)
       | _ -> (
           match params with
           | Type.ValueParameter (a, _) :: rest ->
              (name_of a, match rest with Type.ValueParameter (b, _) :: _ -> name_of b | [] -> "k")
           | [] -> ("w", "k"))
     in
     let s_len = fresh ctx "s_len" and s_off = fresh ctx "s_off" in
     let k = fresh ctx "k" in
     ctx.vars <-
       [ (whole_name, VSpan { len = s_len; off = s_off }); (k_name, VInt k) ];
     let binds, ret = flatten ctx body [] in
     let halves =
       List.filter_map
         (fun (n, e) ->
           match e with
           | TRecordConstructor (ty, fs) when ty_is_named ty ctx.pilot.span_ty ->
              Some (n, { b_off = field ctx fs "off"; b_len = field ctx fs "len";
                         b_base = base_field ctx fs })
           | _ -> None)
         binds
     in
     (match halves with
      | [ (l, left); (r, right) ] ->
         let check ~what ~expected ~actual =
           match LiquidIndex.convert ctx.idx expected actual with
           | LiquidIndex.Convertible -> ()
           | _ ->
              refuse
                "L11c: `%s` builds `%s` as `%s`, but the checked split requires \
                 `%s`; the halves would not partition the input%s"
                ctx.pilot.split_fn what (LiquidIndex.canonical actual)
                (LiquidIndex.canonical expected) (at ctx)
         in
         check ~what:(l ^ ".off") ~expected:s_off ~actual:left.b_off;
         check ~what:(l ^ ".len") ~expected:k ~actual:left.b_len;
         check ~what:(r ^ ".off") ~expected:(LiquidIndex.add s_off k)
           ~actual:right.b_off;
         check ~what:(r ^ ".len") ~expected:(LiquidIndex.nat_sub s_len k)
           ~actual:right.b_len;
         if left.b_base <> right.b_base then
           refuse
             "L11c: `%s` gives the halves different origins (`%s` and `%s`); \
              the two halves must share the input's base%s"
             ctx.pilot.split_fn left.b_base right.b_base (at ctx);
         (* Contiguity and conservation are *consequences* of the rules above,
            not separate checks: `l.len = k`, `l.off = offset(w)` and
            `r.off = offset(w) + k` put the right half exactly where the left
            one ends, and `r.len = length(w) - k` with the precondition
            `k <= length(w)` makes the two lengths add up. So an overlap is
            refused by the `r.off` rule (with the precondition's arithmetic in
            hand) and a lost byte by the `r.len` rule — which is why the two
            extra conversions this pass used to run were deleted: they could
            never fire once the rules held, and dead checks read as coverage. *)
         (* Conservation is by construction, not by arithmetic: the left length
            is literally `k` and the right literally `length(s) - k`, and the
            precondition `0 <= k && k <= length(s)` says the subtraction is
            exact. Stating it as a conversion would need ring reasoning
            (`k + (n - k) = n`) that L11b deliberately does not have, and a
            prover this pilot deliberately does not call; a wrong length is
            already refused by the two rules above, which is the check that
            matters. *)
         ignore s_len;
         (* The return must hand back exactly those two halves. *)
         (match ret with
          | TUnionConstructor (_, c, fs)
            when name_of c = ctx.pilot.both_case
                 && List.length fs = 2
                 && List.for_all
                      (fun (_, e) ->
                        match e with
                        | TLocalVar (v, _) | TParamVar (v, _) ->
                           List.mem (name_of v) [ l; r ]
                        | _ -> false)
                      fs ->
             ()
          | _ ->
             refuse
               "L11c: `%s` must return the `%s` case built from the two halves \
                it just constructed%s"
               ctx.pilot.split_fn ctx.pilot.both_case (at ctx))
      | _ ->
         refuse
           "L11c: `%s` must build exactly two spans of `%s`%s"
           ctx.pilot.split_fn ctx.pilot.span_ty (at ctx))

(* ── call sites and branches ─────────────────────────────────────────────── *)

let requires_of (pragmas : Common.pragma list) : LiquidTypes.formula list =
  List.filter_map
    (function
      | Common.LiquidPragma (kind, src) when kind = "Requires" -> (
          try
            match
              (LiquidParse.parse_contract ~is_known_type:(fun _ -> false)
                 LiquidTypes.KRequires src)
                .LiquidTypes.formula
            with
            | Some f -> Some f
            | None -> None
          with Failure _ -> None)
      | _ -> None)
    pragmas

let callee_requires (decls : typed_decl list) (fn : string) :
    (string list * LiquidTypes.formula list) option =
  let rec go = function
    | [] -> None
    | TFunction (_, _, n, _, params, _, _, _, pragmas) :: rest ->
       if name_of n <> fn then go rest
       else
         let names =
           List.map
             (fun (Type.ValueParameter (p, _)) -> name_of p)
             params
         in
         Some (names, requires_of pragmas)
    | _ :: rest -> go rest
  in
  go decls

let check_call (ctx : ctx) (decls : typed_decl list) (fn : string)
    (args : texpr list) : unit =
  match callee_requires decls fn with
  | None -> ()
  | Some (params, reqs) ->
     if List.length params <> List.length args then
       refuse "L11c: `%s` calls `%s` with %d argument(s), but it takes %d%s"
         ctx.fn fn (List.length args) (List.length params) (at ctx);
     let sub = List.combine params args in
     List.iter
       (fun f ->
         List.iter
           (fun fact ->
             let what =
               Printf.sprintf "%s's precondition `%s`" fn
                 (LiquidTypes.string_of_formula f)
             in
             discharge ctx ~what fact)
           (facts_of_formula ctx ~sub f))
       reqs

let split_actuals (ctx : ctx) (args : texpr list) : split_info =
  match args with
  | [ s; k ] -> (
      match s with
      | TParamVar (n, _) | TLocalVar (n, _) -> (
          match span_of ctx (name_of n) with
          | Some (len, off) -> { si_len = len; si_off = off; si_k = term_of_expr ctx k }
          | None ->
             refuse "L11c: `%s` splits `%s`, which is not a measured span%s"
               ctx.fn (name_of n) (at ctx))
      | _ ->
         refuse "L11c: `%s` splits an expression the pilot cannot measure%s" ctx.fn
           (at ctx))
  | _ -> refuse "L11c: `%s` calls `%s` with the wrong arity%s" ctx.fn ctx.pilot.split_fn (at ctx)

let rec walk (ctx : ctx) (decls : typed_decl list) (s : tstmt) : unit =
  match s with
  | TSkip _ -> ()
  | TLet (_, _, _, _, body) -> walk ctx decls body
  | TBlock (_, a, b) ->
     walk ctx decls a;
     walk ctx decls b
  | TInitialAssign (q, e) ->
     let n = name_of (Identifier.original_name q) in
     (match e with
      | TFuncall (_, f, args, _, _) when qname f = ctx.pilot.split_fn ->
         let info = split_actuals ctx args in
         check_call ctx decls ctx.pilot.split_fn args;
         ctx.splits <- (n, info) :: ctx.splits;
         ctx.vars <- (n, VOther) :: ctx.vars
      | TFuncall (_, f, args, _, _) ->
         check_call ctx decls (qname f) args;
         ctx.vars <- (n, VOther) :: ctx.vars
      | TRecordConstructor (ty, fs) when ty_is_named ty ctx.pilot.span_ty ->
         let off = field ctx fs "off" and len = field ctx fs "len" in
         ctx.vars <- (n, VSpan { len; off }) :: ctx.vars
      | _ -> (
          match (try Some (term_of_expr ctx e) with Outside _ -> None) with
          | Some t -> ctx.vars <- (n, VInt t) :: ctx.vars
          | None -> ctx.vars <- (n, VOther) :: ctx.vars))
  | TLetTmp (id, _, e) | TAssignTmp (id, e) ->
     (* The typer hoists a call's result into a temporary and matches on that,
        so the split's identity has to be carried by the temporary's name. *)
     let n = name_of id in
     (match e with
      | TFuncall (_, f, args, _, _) when qname f = ctx.pilot.split_fn ->
         check_call ctx decls ctx.pilot.split_fn args;
         ctx.splits <- (n, split_actuals ctx args) :: ctx.splits;
         ctx.vars <- (n, VOther) :: ctx.vars
      | TFuncall (_, f, args, _, _) ->
         check_call ctx decls (qname f) args;
         ctx.vars <- (n, VOther) :: ctx.vars
      | _ -> (
          match (try Some (term_of_expr ctx e) with Outside _ -> None) with
          | Some t -> ctx.vars <- (n, VInt t) :: ctx.vars
          | None -> ctx.vars <- (n, VOther) :: ctx.vars))
  | TReturn (_, e) | TDiscarding (_, e) -> walk_expr ctx decls e
  | TCase (_, scrutinee, whens, _) ->
     let info =
       match scrutinee with
       | TFuncall (_, f, args, _, _) when qname f = ctx.pilot.split_fn ->
          check_call ctx decls ctx.pilot.split_fn args;
          Some (split_actuals ctx args)
       | TParamVar (n, _) | TLocalVar (n, _) | TTemporary (n, _) ->
          List.assoc_opt (name_of n) ctx.splits
       | _ -> None
     in
     (match info with
      | None ->
         refuse
           "L11c: `%s` matches a value the pilot did not see produced by `%s`%s"
           ctx.fn ctx.pilot.split_fn (at ctx)
      | Some info ->
         List.iter
           (fun (TypedWhen (c, bindings, body)) ->
             let saved_vars = ctx.vars in
             let saved_aliases = ctx.aliases in
             let saved_scopes = ctx.scopes in
             ctx.scopes <- [] :: ctx.scopes;
             if name_of c = ctx.pilot.both_case then begin
               let slots =
                 List.map
                   (fun (TypedBinding { name; rename; _ }) ->
                     (name_of name, name_of rename))
                   bindings
               in
               match slots with
               | [ (l, l_rename); (r, r_rename) ] ->
                  let l_len = fresh ctx "l_len" and l_off = fresh ctx "l_off" in
                  let r_len = fresh ctx "r_len" and r_off = fresh ctx "r_off" in
                  let bind_span n len off =
                    ctx.vars <- (n, VSpan { len; off }) :: ctx.vars
                  in
                  bind_span l l_len l_off;
                  bind_span r r_len r_off;
                  (* The body sees the binding through whatever rename the
                     destructuring made, so both names must resolve. *)
                  ctx.aliases <-
                    (l_rename, l) :: (r_rename, r) :: ctx.aliases;
                  let add fk fa fb = ctx.scopes <- ([ { fk; fa; fb } ] @ List.hd ctx.scopes) :: List.tl ctx.scopes in
                  add FEq l_len info.si_k;
                  add FEq l_off info.si_off;
                  add FEq r_off (LiquidIndex.add info.si_off info.si_k);
                  add FEq r_len (LiquidIndex.nat_sub info.si_len info.si_k);
                  (* Disjointness is *derived*: both sides must rewrite to the
                     same term, or the pilot does not get to assume it. *)
                  let left_end = LiquidIndex.add l_off l_len in
                  let l = rewrite ctx left_end and r = rewrite ctx r_off in
                  if LiquidIndex.convert ctx.idx l r = LiquidIndex.Convertible
                  then add FLe left_end r_off
                  else
                    refuse
                      "L11c: `%s` cannot derive that the two halves are \
                        disjoint from the checked split%s"
                      ctx.fn (at ctx)
               | _ ->
                  refuse
                    "L11c: the `%s` case of `%s` must bind exactly two spans%s"
                    ctx.pilot.both_case ctx.pilot.split_fn (at ctx)
             end;
             walk ctx decls body;
             (* Branch facts die with the branch; keep them only to explain a
                later failure. *)
             let label =
               Printf.sprintf "the `%s` branch of `%s`" (name_of c) ctx.pilot.split_fn
             in
             ctx.dropped <-
               List.map (fun f -> (label, f)) (List.concat ctx.scopes) @ ctx.dropped;
             ctx.scopes <- saved_scopes;
             ctx.aliases <- saved_aliases;
             ctx.vars <- saved_vars)
           whens)
  | TIf (_, c, t, f) ->
     walk_expr ctx decls c;
     walk ctx decls t;
     walk ctx decls f
  | TBorrow { original; rename; body; _ } ->
     let saved = ctx.aliases in
     ctx.aliases <- (name_of rename, resolve ctx (name_of original)) :: ctx.aliases;
     walk ctx decls body;
     ctx.aliases <- saved
  | _ ->
     outside
       "`%s` uses a statement outside the pilot fragment (loops, assignment and \
        borrows are not part of it)"
       ctx.fn

and walk_expr (ctx : ctx) (decls : typed_decl list) (e : texpr) : unit =
  match e with
  | TFuncall (_, f, args, _, _) ->
     List.iter (walk_expr ctx decls) args;
     check_call ctx decls (qname f) args
  | TMethodCall (_, _, _, args, _, _) -> List.iter (walk_expr ctx decls) args
  | TUnionConstructor (_, _, fs) ->
     List.iter (fun (_, v) -> walk_expr ctx decls v) fs
  | TRecordConstructor (_, fs) -> List.iter (fun (_, v) -> walk_expr ctx decls v) fs
  | TComparison (_, a, b) | TConjunction (a, b) | TDisjunction (a, b) ->
     walk_expr ctx decls a;
     walk_expr ctx decls b
  | TCast (e', _) | TNegation e' | TSlotAccessor (e', _, _)
  | TPointerSlotAccessor (e', _, _) | TDeref e' ->
     walk_expr ctx decls e'
  | TIfExpression (c, a, b) ->
     walk_expr ctx decls c;
     walk_expr ctx decls a;
     walk_expr ctx decls b
  | _ -> ()

let check_function (ctx : ctx) (decls : typed_decl list) (name : string)
    (params : Type.value_parameter list) (body : tstmt)
    (pragmas : Common.pragma list) : unit =
  ctx.fn <- name;
  ctx.vars <- [];
  ctx.scopes <- [ [] ];
  ctx.dropped <- [];
  ctx.splits <- [];
  ctx.loc <- None;
  ctx.aliases <- [];
  List.iter
    (fun (Type.ValueParameter (p, t)) ->
      let n = name_of p in
      if ty_is_named t ctx.pilot.span_ty then
        let len = fresh ctx (n ^ "_len") and off = fresh ctx (n ^ "_off") in
        ctx.vars <- (n, VSpan { len; off }) :: ctx.vars
      else if ty_is_int64 t then ctx.vars <- (n, VInt (fresh ctx n)) :: ctx.vars
      else ctx.vars <- (n, VOther) :: ctx.vars)
    params;
  (* The function's own `Requires` are its assumptions. *)
  List.iter
    (fun f ->
      List.iter
        (fun fact -> ctx.scopes <- ([ fact ] @ List.hd ctx.scopes) :: List.tl ctx.scopes)
        (facts_of_formula ctx ~sub:[] f))
    (requires_of pragmas);
  walk ctx decls body

let check_module (m : typed_module) : string option =
  let (TypedModule (_, decls)) = m in
  match recognize decls with
  | None -> None
  | Some pilot ->
     let ctx =
       { idx = LiquidIndex.empty; pilot; vars = []; scopes = [ [] ]; dropped = [];
         fn = ""; splits = []; loc = None; aliases = [] }
     in
     try
       (try verify_split ctx decls
        with Outside msg ->
          refuse
            "L11c: the checked split `%s` uses a construct the pilot cannot \
             read (%s)" pilot.split_fn msg);
       List.iter
         (fun d ->
           match d with
           | TFunction (_, _, n, _, params, _, body, _, pragmas) ->
              (try check_function ctx decls (name_of n) params body pragmas
               with
               | Outside _ when name_of n <> pilot.split_fn ->
                  (* Skipped, not checked: the pass is inert for a function it
                     cannot read. A real refusal is a [Refuse] and reaches the
                     user — the distinction is the whole reason [Outside]
                     exists. *)
                  ()
               | Outside msg ->
                  raise
                    (Refuse
                       (Printf.sprintf
                          "L11c: `%s` is the checked split but uses a construct \
                           the pilot cannot read (%s)" pilot.split_fn msg)))
           | _ -> ())
         decls;
       None
     with Refuse msg -> Some msg
