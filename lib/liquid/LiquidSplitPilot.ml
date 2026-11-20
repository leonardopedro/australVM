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

   ## L11d.1: the law is held out, the evidence is reflected

   The shape the pass checks is stated once, above [verify_split], as
   [held_out_split_law]: a reviewed statement that a checked split returns
   `[off, off+k)` and `[off+k, off+n)`, that the lengths add to `n`, that no
   element belongs to both halves, and that byte addresses are element indices
   scaled by a checked element width. Nothing derives it from the source; the
   analyser's reading of the source (its structured evidence) is compared
   against it, and a split whose evidence cannot be read into the law is
   refused at the same place a wrong length was always refused.

   The evidence is also exposed: [evidence_of_module] returns the resolved
   function/field declarations, the canonical binders, the measured operands,
   the numeric interpretation (a checked literal or an unbounded index) and the
   span pair actually constructed, and `LiquidCheck` writes it to the path in
   `AUSTRAL_LIQUID_EVIDENCE`. Binder identities in that output are canonical
   (`w.len`, `w.off`, `k`), not the allocator's `v0`, so a proof subject does
   not move when an unrelated binder is introduced.

   `left.len + right.len = n` and the disjointness of the halves are *stated*
   by the law and recorded as `[stated]` obligations: L11d.1 reflects and holds
   them out; L11d.2 is where the arithmetic gets a proof.

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
  union_ty : string;
  both_slots : string list;   (** the case's slots, in declaration order *)
  record_id : int;
  split_id : int;
  len_id : int;
  off_id : int;
  width : int option;         (** an `element_width` declaration, when present *)
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
    (string * string * string list) option =
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
        | Some (Stages.Linked.LCase (_, cn, slots)) ->
           Some (name_of un, name_of cn, List.map slot_name slots)
        | None -> go rest)
    | _ :: rest -> go rest
  in
  go decls

(** The element width the law's byte relation is stated over. A module may pin
    it with `function element_width(): Int64 is return 8; end;`; when it does
    not, the width is a layout premise supplied by the host and the evidence
    says so instead of inventing one. A declaration that is *not* a positive
    literal is a refusal: a byte relation over a width nobody can compute is a
    fiction, and the pilot's job is not to invent premises. *)
let element_width_of (decls : typed_decl list) : int option =
  let rec go = function
    | [] -> None
    | TFunction (_, _, n, _, params, _, body, _, _) :: rest ->
       if name_of n <> "element_width" then go rest
       else begin
         match (params, body) with
         | [], TReturn (_, TIntConstant s) -> (
             match int_of_string_opt s with
             | Some w when w > 0 && w <= LiquidIndex.max_value LiquidIndex.E64 -> Some w
             | _ ->
                refuse
                  "L11d.1: `element_width` must return a positive literal that \
                   fits a machine offset; the law's byte relation is stated over \
                   a width the module can compute")
         | _ ->
            refuse
              "L11d.1: `element_width` must be `function element_width(): Int64 \
               is return <literal>; end;` so the byte relation has a real width"
       end
    | _ :: rest -> go rest
  in
  go decls

let decl_id (Id.DeclId n) = n

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

let find_decl_id (decls : typed_decl list) (fn : string) : int option =
  List.find_map
    (function
      | TFunction (id, _, n, _, _, _, _, _, _) when name_of n = fn -> Some (decl_id id)
      | _ -> None)
    decls

let find_record_id (decls : typed_decl list) (name : string) : int option =
  List.find_map
    (function
      | TRecord (id, _, n, _, _, _, _) when name_of n = name -> Some (decl_id id)
      | _ -> None)
    decls

let recognize (decls : typed_decl list) : pilot option =
  match find_record decls with
  | None -> None
  | Some (span_ty, _) -> (
      match find_union_case decls span_ty with
      | None -> None
      | Some (union_ty, both_case, both_slots) ->
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
           match
             ( find_record_id decls span_ty, find_decl_id decls "split",
               find_decl_id decls "length", find_decl_id decls "offset" )
           with
           | Some record_id, Some split_id, Some len_id, Some off_id ->
              Some
                { span_ty; split_fn = "split"; both_case; len_fn = "length";
                  off_fn = "offset"; union_ty; both_slots; record_id; split_id;
                  len_id; off_id; width = element_width_of decls }
           | _ -> None
         else None)

(* ── facts ───────────────────────────────────────────────────────────────── *)

type fact_kind = FEq | FLe

type fact = { fk : fact_kind; fa : LiquidIndex.t; fb : LiquidIndex.t }

type vinfo =
  | VSpan of { len : LiquidIndex.t; off : LiquidIndex.t }
  | VInt of LiquidIndex.t
  | VOther

type split_info = { si_len : LiquidIndex.t; si_off : LiquidIndex.t; si_k : LiquidIndex.t }

(* ── the held-out law (L11d.1) ─────────────────────────────────────────────

   Stated here, by hand, independently of anything the analyser derives. The
   law's terms are a tiny explicit language over the three binders the pilot
   reasons about (`off`, `n`, `k`), so the comparison against the constructed
   spans is a structural instantiation and not a re-run of the analyser's own
   arithmetic. *)

type law_term = LOff | LLen | LK | LAdd of law_term * law_term | LSub of law_term * law_term

type law_span = { lp_off : law_term; lp_len : law_term }

type split_law = {
  law_id : string;
  law_owner : string;
  law_binders : string list;
  law_premises : string list;
  law_left : law_span;
  law_right : law_span;
  law_conservation : string;
  law_disjoint : string;
  law_bytes : string;
}

(** The reviewed statement L11d.1 holds out. `off`, `n`, `k` are natural
    element units; the span's measured `len` is `n`. *)
let held_out_split_law : split_law =
  { law_id = "liquid-split.extent-conservation";
    law_owner = "L11d.1";
    law_binders = [ "off"; "n"; "k" ];
    law_premises =
      [ "k <= n";
        "the extent [off, off+n) fits the validated allocation domain";
        "an element is `width` bytes, with width > 0 (layout premise)" ];
    law_left = { lp_off = LOff; lp_len = LK };
    law_right = { lp_off = LAdd (LOff, LK); lp_len = LSub (LLen, LK) };
    law_conservation = "left.len + right.len = n";
    law_disjoint = "no element index belongs to both halves";
    law_bytes = "byte address = element index * width, checked to fit Int64" }

let rec instantiate_law ~(off : LiquidIndex.t) ~(len : LiquidIndex.t)
    ~(k : LiquidIndex.t) (t : law_term) : LiquidIndex.t =
  match t with
  | LOff -> off
  | LLen -> len
  | LK -> k
  | LAdd (a, b) ->
     LiquidIndex.add (instantiate_law ~off ~len ~k a) (instantiate_law ~off ~len ~k b)
  | LSub (a, b) ->
     LiquidIndex.nat_sub (instantiate_law ~off ~len ~k a)
       (instantiate_law ~off ~len ~k b)

(* ── structured evidence (L11d.1) ───────────────────────────────────────── *)

type span_ev = { se_off : LiquidIndex.t; se_len : LiquidIndex.t; se_base : string }

type call_evidence = {
  ce_in : string;                (** the function the call appears in *)
  ce_off : LiquidIndex.t;
  ce_len : LiquidIndex.t;
  ce_k : LiquidIndex.t;
}

type binder_ev = { be_role : string; be_name : string }

type evidence = {
  ev_law : string;
  ev_module : string;
  ev_span_ty : string;
  ev_fields : string list;
  ev_union : string;
  ev_case : string;
  ev_case_slots : string list;
  ev_ids : (string * int) list;
  ev_split : string;
  ev_names : (int * string) list;  (** binder id → canonical name, for the subject *)
  ev_binders : binder_ev list;
  ev_premises : string list;
  ev_left : span_ev;
  ev_right : span_ev;
  ev_calls : call_evidence list;
  ev_width : int option;
}

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
  module_name : string;
  mutable names : (int * string) list;  (** binder id → canonical evidence name *)
  mutable calls : call_evidence list;
  mutable ev : evidence option;
}

(** A binder allocated for the *analysis*, with a canonical name for the
    evidence. The name is the proof subject (`w.len`, `w.off`, `k`, …), so it is
    stable under binder renumbering: two runs that agree on the source agree on
    the evidence, whatever order the allocator handed out ids in. *)
let fresh_named (ctx : ctx) ~(name : string) (hint : string) : LiquidIndex.t =
  let env, id = LiquidIndex.bind ctx.idx ~name:hint ~sort:LiquidTypes.SNat in
  ctx.idx <- env;
  ctx.names <- (id, name) :: ctx.names;
  LiquidIndex.var id

let fresh (ctx : ctx) (hint : string) : LiquidIndex.t = fresh_named ctx ~name:hint hint

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

let verify_split (ctx : ctx) (decls : typed_decl list) : unit =
  (* The parameter names are the *declaration's*: a pilot that reads a
     hard-coded `s`/`k` silently measures nothing for any other spelling. *)
  let decl =
    let rec go = function
      | [] -> None
      | TFunction (_, _, n, _, params, _, body, _, pragmas) :: rest ->
         if name_of n = ctx.pilot.split_fn then Some (params, body, pragmas)
         else go rest
      | _ :: rest -> go rest
    in
    go decls
  in
  match decl with
  | None -> ()
  | Some (params, body, pragmas) ->
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
     let s_len = fresh_named ctx ~name:(whole_name ^ ".len") "s_len" in
     let s_off = fresh_named ctx ~name:(whole_name ^ ".off") "s_off" in
     let k = fresh_named ctx ~name:k_name "k" in
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
         (* The expected terms come from the *held-out law*, instantiated at the
            declaration's own binders — not from a second copy of the shape
            written next to the check. `n` is the span's measured length. *)
         let law = held_out_split_law in
         let left_law =
           { b_off = instantiate_law ~off:s_off ~len:s_len ~k law.law_left.lp_off;
             b_len = instantiate_law ~off:s_off ~len:s_len ~k law.law_left.lp_len;
             b_base = left.b_base }
         in
         let right_law =
           { b_off = instantiate_law ~off:s_off ~len:s_len ~k law.law_right.lp_off;
             b_len = instantiate_law ~off:s_off ~len:s_len ~k law.law_right.lp_len;
             b_base = right.b_base }
         in
         check ~what:(l ^ ".off") ~expected:left_law.b_off ~actual:left.b_off;
         check ~what:(l ^ ".len") ~expected:left_law.b_len ~actual:left.b_len;
         check ~what:(r ^ ".off") ~expected:right_law.b_off ~actual:right.b_off;
         check ~what:(r ^ ".len") ~expected:right_law.b_len ~actual:right.b_len;
         if left.b_base <> right.b_base then
           refuse
             "L11c: `%s` gives the halves different origins (`%s` and `%s`); \
              the two halves must share the input's base%s"
             ctx.pilot.split_fn left.b_base right.b_base (at ctx);
         (* Contiguity and conservation are *consequences* of the rules above,
            not separate checks: the law's right half starts where its left one
            ends, and with the precondition `k <= n` the two lengths add up. So
            an overlap is refused by the `r.off` rule and a lost byte by the
            `r.len` rule. Conservation itself (`k + (n - k) = n`) needs ring
            reasoning that L11b deliberately does not have; L11d.2 is where the
            stated obligation gets a proof, and until then the evidence records
            it as `[stated]` rather than claiming it. *)
         (* The return must hand back exactly those two halves, *in the case's
            own slot order*: `Both(left => r, right => l)` hands a caller a
            left half at the right offset, so a reversed field is refused here
            rather than silently accepted because both names were present. *)
         let ordered fs =
           List.map (fun (k, _) -> name_of k) fs = ctx.pilot.both_slots
           && List.filter_map
                (fun (_, e) ->
                  match e with
                  | TLocalVar (v, _) | TParamVar (v, _) -> Some (name_of v)
                  | _ -> None)
                fs
              = [ l; r ]
         in
         (match ret with
          | TUnionConstructor (_, c, fs)
            when name_of c = ctx.pilot.both_case && List.length fs = 2
                 && ordered fs ->
             ()
          | _ ->
             refuse
               "L11c: `%s` must return the `%s` case built from the two halves \
                it just constructed, in the case's slot order%s"
               ctx.pilot.split_fn ctx.pilot.both_case (at ctx));
         (* Reflect what the analyser actually read, so a caller can replay the
            obligation against the law instead of trusting a green exit. *)
         ctx.ev <-
           Some
             { ev_law = law.law_id;
               ev_module = ctx.module_name;
               ev_span_ty = ctx.pilot.span_ty;
               ev_fields = span_slots;
               ev_union = ctx.pilot.union_ty;
               ev_case = ctx.pilot.both_case;
               ev_case_slots = ctx.pilot.both_slots;
               ev_ids =
                 [ ("record", ctx.pilot.record_id); ("split", ctx.pilot.split_id);
                   ("length", ctx.pilot.len_id); ("offset", ctx.pilot.off_id) ];
               ev_split = ctx.pilot.split_fn;
               ev_names = ctx.names;
               ev_binders =
                 [ { be_role = "span"; be_name = whole_name };
                   { be_role = "number"; be_name = k_name } ];
               ev_premises =
                 List.map LiquidTypes.string_of_formula (requires_of pragmas)
                 @ law.law_premises;
               ev_left = { se_off = left.b_off; se_len = left.b_len; se_base = left.b_base };
               ev_right = { se_off = right.b_off; se_len = right.b_len; se_base = right.b_base };
               ev_calls = [];
               ev_width = ctx.pilot.width }
      | _ ->
         refuse
           "L11c: `%s` must build exactly two spans of `%s`%s"
           ctx.pilot.split_fn ctx.pilot.span_ty (at ctx))

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

let record_call (ctx : ctx) (info : split_info) : unit =
  ctx.calls <-
    { ce_in = ctx.fn; ce_off = info.si_off; ce_len = info.si_len; ce_k = info.si_k }
    :: ctx.calls

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
         record_call ctx info;
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
         let info = split_actuals ctx args in
         record_call ctx info;
         ctx.splits <- (n, info) :: ctx.splits;
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
          let info = split_actuals ctx args in
          record_call ctx info;
          Some info
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
                  let l_len = fresh_named ctx ~name:(l ^ ".len") "l_len" in
                  let l_off = fresh_named ctx ~name:(l ^ ".off") "l_off" in
                  let r_len = fresh_named ctx ~name:(r ^ ".len") "r_len" in
                  let r_off = fresh_named ctx ~name:(r ^ ".off") "r_off" in
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
        let len = fresh_named ctx ~name:(n ^ ".len") (n ^ "_len") in
        let off = fresh_named ctx ~name:(n ^ ".off") (n ^ "_off") in
        ctx.vars <- (n, VSpan { len; off }) :: ctx.vars
      else if ty_is_int64 t then
        ctx.vars <- (n, VInt (fresh_named ctx ~name:n n)) :: ctx.vars
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

(* ── evidence: rendering and numeric interpretation (L11d.1) ─────────────── *)

let rec eval (t : LiquidIndex.t) : int option =
  match t.LiquidIndex.node with
  | LiquidIndex.Lit n -> Some n
  | LiquidIndex.Add (a, b) -> (
      match (eval a, eval b) with
      | Some x, Some y when y >= 0 && x <= max_int - y -> Some (x + y)
      | _ -> None)
  | LiquidIndex.NatSub (a, b) -> (
      match (eval a, eval b) with
      | Some x, Some y -> Some (max 0 (x - y))
      | _ -> None)
  | LiquidIndex.Var _ | LiquidIndex.Mod _ | LiquidIndex.Def _ | LiquidIndex.Machine _ -> None

type range = { rg_start : int; rg_len : int }

let range_of (off : LiquidIndex.t) (len : LiquidIndex.t) : range option =
  match (eval off, eval len) with Some o, Some l -> Some { rg_start = o; rg_len = l } | _ -> None

(** Byte addresses are element indices scaled by the width, and the scaling is
    *checked*: an extent that would overflow the host integer is [None], not a
    wrapped range. *)
let byte_range ~(width : int) (r : range) : range option =
  if width <= 0 then None
  else if r.rg_len > max_int / width || r.rg_start > max_int / width then None
  else Some { rg_start = r.rg_start * width; rg_len = r.rg_len * width }

(** How the evidence reads a term: a decidable literal *is* a checked machine
    offset, and anything else stays an unbounded index rather than being
    given a number the pilot did not derive. *)
let interpretation (t : LiquidIndex.t) : string =
  match eval t with Some n -> Printf.sprintf "checked %d" n | None -> "unbounded"

let rec render_term ?(names = []) (t : LiquidIndex.t) : string =
  let r = render_term ~names in
  match t.LiquidIndex.node with
  | LiquidIndex.Var id -> (
      match List.assoc_opt id names with
      | Some n -> n
      | None -> LiquidIndex.canonical t)
  | LiquidIndex.Lit n -> string_of_int n
  | LiquidIndex.Add (a, b) -> Printf.sprintf "(+ %s %s)" (r a) (r b)
  | LiquidIndex.NatSub (a, b) -> Printf.sprintf "(- %s %s)" (r a) (r b)
  | LiquidIndex.Mod (a, b) -> Printf.sprintf "(mod %s %s)" (r a) (r b)
  | LiquidIndex.Def (f, args) ->
     Printf.sprintf "(%s%s)" f
       (String.concat "" (List.map (fun a -> " " ^ r a) args))
  | LiquidIndex.Machine (w, op, a, b) ->
     Printf.sprintf "(#%s %s %s %s)" (LiquidIndex.string_of_elem w)
       (match op with LiquidIndex.MAdd -> "+" | LiquidIndex.MSub -> "-")
       (r a) (r b)

let render_range (r : range) : string =
  Printf.sprintf "[%d,%d) len=%d" r.rg_start (r.rg_start + r.rg_len) r.rg_len

(** Deterministic, line-oriented: a caller diffs this against a stored subject
    instead of parsing a tree. *)
let render_evidence (e : evidence) : string =
  let term = render_term ~names:e.ev_names in
  let buf = Buffer.create 1024 in
  let line fmt = Printf.ksprintf (fun s -> Buffer.add_string buf (s ^ "\n")) fmt in
  line "# liquid split evidence (L11d.1)";
  line "law: %s [%s]" e.ev_law held_out_split_law.law_owner;
  line "module: %s" e.ev_module;
  line "record: %s fields %s" e.ev_span_ty (String.concat "," e.ev_fields);
  line "union: %s case %s slots %s" e.ev_union e.ev_case
    (String.concat "," e.ev_case_slots);
  List.iter (fun (what, id) -> line "decl: %s id=%d" what id) e.ev_ids;
  List.iter (fun b -> line "binder: %s %s" b.be_role b.be_name) e.ev_binders;
  List.iter (fun p -> line "premise: %s" p) e.ev_premises;
  (match e.ev_width with
   | Some w -> line "width: %d (module declaration)" w
   | None -> line "width: host-supplied (no `element_width` declaration)");
  (* The constructed spans, in the law's own vocabulary (`w.off`, `k`, `w.len`),
     so a reader compares them to the law rather than to the allocator's ids. *)
  line "obligation %s: left = (%s, %s)" held_out_split_law.law_id
    (term e.ev_left.se_off) (term e.ev_left.se_len);
  line "obligation %s: right = (%s, %s)" held_out_split_law.law_id
    (term e.ev_right.se_off) (term e.ev_right.se_len);
  line "obligation left.base = right.base = %s [checked against the law]"
    e.ev_left.se_base;
  (* Conservation and disjointness are `[stated]`: L11d.2 owns their proof and
     nothing in this line claims one. *)
  line "obligation conservation: %s [stated: L11d.2]"
    held_out_split_law.law_conservation;
  line "obligation disjoint: %s [stated: L11d.2]" held_out_split_law.law_disjoint;
  line "obligation bytes: %s [premise: width]" held_out_split_law.law_bytes;
  List.iter
    (fun c ->
      line "call: %s calls %s" c.ce_in e.ev_split;
      line "  operands: off=%s n=%s k=%s" (term c.ce_off) (term c.ce_len)
        (term c.ce_k);
      line "  interpretation: off=%s n=%s k=%s"
        (interpretation c.ce_off) (interpretation c.ce_len) (interpretation c.ce_k);
      match (range_of c.ce_off c.ce_len, eval c.ce_k) with
      | Some whole, Some k when k <= whole.rg_len ->
         let left = { rg_start = whole.rg_start; rg_len = k } in
         let right = { rg_start = whole.rg_start + k; rg_len = whole.rg_len - k } in
         line "  element ranges: left %s, right %s" (render_range left)
           (render_range right);
         line "  conservation: %d + %d = %d [checked]" left.rg_len right.rg_len
           whole.rg_len;
         line "  disjoint: %d <= %d [checked]" (left.rg_start + left.rg_len)
           right.rg_start;
         (match e.ev_width with
          | Some w -> (
              match (byte_range ~width:w left, byte_range ~width:w right) with
              | Some bl, Some br ->
                 line "  byte ranges: left %s, right %s" (render_range bl)
                   (render_range br)
              | _ ->
                 line
                   "  byte ranges: [gap] the element extent does not fit a \
                    checked machine offset at width %d" w)
          | None -> line "  byte ranges: [premise] width supplied by the host")
      | Some whole, Some k ->
         line "  ranges: [REFUSED] k=%d exceeds the measured extent %s" k
           (render_range whole)
      | _ -> line "  ranges: symbolic (unbounded index arithmetic)")
    (List.rev e.ev_calls);
  Buffer.contents buf

let check_module_evidence (m : typed_module) : string option * evidence option =
  let (TypedModule (mod_name, decls)) = m in
  match recognize decls with
  | None -> (None, None)
  | Some pilot ->
     let ctx =
       { idx = LiquidIndex.empty; pilot; vars = []; scopes = [ [] ]; dropped = [];
         fn = ""; splits = []; loc = None; aliases = [];
         module_name = Identifier.mod_name_string mod_name; names = [];
         calls = []; ev = None }
     in
     match
       (try
          (try verify_split ctx decls
           with Outside msg ->
             refuse
               "L11c: the checked split `%s` uses a construct the pilot cannot \
                read (%s)" pilot.split_fn msg);
          `Ok ()
        with Refuse msg -> `Refused msg)
     with
     | `Refused msg -> (Some msg, None)
     | `Ok () ->
        (try
           List.iter
             (fun d ->
               match d with
               | TFunction (_, _, n, _, params, _, body, _, pragmas) ->
                  (try check_function ctx decls (name_of n) params body pragmas
                   with
                   | Outside _ when name_of n <> pilot.split_fn ->
                      (* Skipped, not checked: the pass is inert for a function
                         it cannot read. A real refusal is a [Refuse] and
                         reaches the user. *)
                      ()
                   | Outside msg ->
                      raise
                        (Refuse
                           (Printf.sprintf
                              "L11c: `%s` is the checked split but uses a \
                               construct the pilot cannot read (%s)"
                              pilot.split_fn msg)))
               | _ -> ())
             decls;
           let ev =
             match ctx.ev with
             | None -> None
             | Some e ->
                (* `names` is filled by the per-function walk too, so the final
                   evidence carries every binder the calls mention. *)
                Some { e with ev_calls = List.rev ctx.calls; ev_names = ctx.names }
           in
           (None, ev)
         with Refuse msg -> (Some msg, None))

let check_module (m : typed_module) : string option = fst (check_module_evidence m)

let evidence_of_module (m : typed_module) : evidence option =
  snd (check_module_evidence m)
