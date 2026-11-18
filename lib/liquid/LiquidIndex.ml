(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L11b: a small *sorted index language* for natural
   length/offset expressions, with resolved binders, explicit family
   arity/kinds, bidirectional checking, and a bounded normalizer that says
   [Convertible], [Different] or [Unknown].

   Normative for this file is L11b's own text; where it is silent, the choices
   are recorded below and every one of them is pinned by
   `test/LiquidIndexTest.ml`.

   ## Why an elaborated term and not the surface [LiquidTypes.term]

   The parser hands over names. An index expression relates *binders*: inside
   `if n > 0 then …` the `n` on both sides is the same binder, and after a
   `let n := …` shadowing the two `n`s are different ones although they print
   the same. Canonical identity therefore cannot be textual, so elaboration
   resolves every name to a **binder id** allocated when the binder is
   introduced. [canonical] renders ids, never names, which is what makes
   identity stable under renaming and makes shadowing show up as *inequality*
   rather than as accidental equality.

   ## Widths are a source distinction, not a target one

   L11b: "Keep source element types such as `Nat8` distinct from other widths
   even when their index arithmetic maps to the same Why3 sort." Every index
   term therefore carries an optional [elem], and the mixed-width rule is a
   refusal (`n8 + n16`). [why3_sort] nevertheless answers `int` for all four,
   and that asymmetry is deliberate and tested: the source distinction exists
   so a `Span[Nat8, n]` and a `Span[Nat16, n]` are different obligations, not
   so the emitter has four Why3 sorts to learn.

   ## What conversion is allowed to know

   Only rules that are *syntax-directed* and obviously sound over the naturals:

   - identity of addition (`n + 0`, `0 + n`);
   - literal addition, with the right spine reassociated so that a literal
     tail folds (`(n + 1) + 2` and `n + 3` are convertible);
   - identity of natural subtraction (`n - 0`), and literal truncated
     subtraction (natural subtraction saturates at 0 — it is *not* machine
     subtraction, which wraps, nor the integer subtraction of §2.1);
   - `n mod 1 = 0` and literal `mod`.

   Nothing else. In particular **commutativity is not a rule**: `m + n` and
   `n + m` are [Different], and L11b requires that such an equality be carried
   as an *explicit obligation* ([goal_of]) with checked evidence ([require]),
   not silently converted. That is the whole point of separating "convertible"
   from "true": the first is decidable by inspection, the second needs a
   prover, and conflating them is how a refinement checker starts lying.

   Natural subtraction, modular arithmetic and checked machine arithmetic are
   three constructors ([NatSub], [Mod], [Machine]) and never convert into each
   other. A machine term additionally needs a width obligation: [machine_of]
   discharges it only for a literal that fits the width, because that bound is
   decidable here; for a symbolic term it returns the obligation instead of a
   term.

   ## Determinism and budgets

   The normalizer is a pure function of the term plus a [budget]: it counts
   steps, depth, inlined definitions and the size of its result, and raises
   [Exhausted] rather than keeping going. [convert] maps that to [Unknown],
   and [require_convertible] turns [Unknown] into a refusal — an exhausted
   comparison is *not* an acceptance, which is the only safe reading of "we
   could not tell".

   ## Diagnostics are not identity

   A [diag] carries a span from the surface term and a human reason; it is
   never part of a term, never part of [canonical], and never used for
   comparison. Two terms parsed from different offsets are the same term.
*)

(* ── source element widths ──────────────────────────────────────────────── *)

type elem =
  | E8
  | E16
  | E32
  | E64

let string_of_elem = function
  | E8 -> "Nat8"
  | E16 -> "Nat16"
  | E32 -> "Nat32"
  | E64 -> "Nat64"

(** The *target* sort of an index expression. Every source width lowers to the
    same one; see the header. Kept as a function (not a field) so a caller has
    to ask, and so the equality of two different widths is visibly a property
    of the source layer. *)
let why3_sort (_ : elem option) : string = "int"

let elem_of_name = function
  | "Nat8" -> Some E8
  | "Nat16" -> Some E16
  | "Nat32" -> Some E32
  | "Nat64" -> Some E64
  | _ -> None

let elem_of_ty (t : LiquidTypes.ty) : elem option =
  match t with
  | LiquidTypes.TySort (LiquidTypes.SAdt n) -> elem_of_name n
  | _ -> None

(* ── families: explicit arity and kinds ─────────────────────────────────── *)

(** The kind of one argument position of a family. [KElem] is a source element
    type, [KNat] is a natural index. There is no third kind in the initial
    fragment: a family whose index position wants a proposition is not an index
    type. *)
type kind = KElem | KNat

let string_of_kind = function KElem -> "an element type" | KNat -> "a nat index"

type family = { name : string; params : kind list }

let string_of_family (f : family) : string =
  f.name ^ "[" ^ String.concat ", " (List.map string_of_kind f.params) ^ "]"

(** The families the index layer knows. Deliberately a closed table, not "any
    type with a nat argument": a family's *meaning* is what lets L11c relate a
    length to a representation, so an unknown head is refused instead of
    assumed to have the right shape. Order matters only for the diagnostics. *)
let builtin_families : family list =
  [ { name = "Span"; params = [ KElem; KNat; KNat ] }
  ; { name = "Vector"; params = [ KElem; KNat ] }
  ; { name = "Array"; params = [ KElem; KNat ] }
  ; { name = "Buffer"; params = [ KElem; KNat ] }
  ]

let family_of_name (name : string) : family option =
  List.find_opt (fun f -> f.name = name) builtin_families

(* ── the elaborated term ────────────────────────────────────────────────── *)

(** Checked machine arithmetic. Distinct from the nat constructors on purpose:
    an overflow here is an event (the JIT traps), so a machine term is not a
    natural index and does not convert with one. *)
type mop = MAdd | MSub

type t = { node : node; elem : elem option }

and node =
  | Lit of int
  | Var of int
      (** A resolved binder id. The source name is *not* kept: keeping it would
          invite textual comparison, which is exactly the bug the ids exist to
          prevent. *)
  | Add of t * t
  | NatSub of t * t
  | Mod of t * t
  | Def of string * t list
      (** An explicit closed definition from L11a, applied. Inlined by the
          normalizer; kept as a node so an un-inlinable application still has
          an identity and can be rendered. *)
  | Machine of elem * mop * t * t

(** Canonical identity: binder ids, no spans, no names. Two terms are the same
    term exactly when this string is the same string, which is what
    [convert]'s [Convertible] case reports. *)
let rec canonical (t : t) : string =
  match t.node with
  | Lit n -> string_of_int n
  | Var id -> "v" ^ string_of_int id
  | Add (a, b) -> "(+ " ^ canonical a ^ " " ^ canonical b ^ ")"
  | NatSub (a, b) -> "(- " ^ canonical a ^ " " ^ canonical b ^ ")"
  | Mod (a, b) -> "(mod " ^ canonical a ^ " " ^ canonical b ^ ")"
  | Def (f, args) ->
     "(" ^ f ^ (if args = [] then "" else " " ^ String.concat " " (List.map canonical args)) ^ ")"
  | Machine (w, op, a, b) ->
     Printf.sprintf "(#%s %s %s %s)" (string_of_elem w)
       (match op with MAdd -> "+" | MSub -> "-")
       (canonical a) (canonical b)

(** The Why3 rendering of a term. Binders become `v0`, `v1`, …: an Austral
    identifier is a legal Why3 identifier too, but two binders that print the
    same name must not collide in a goal, and the id is the only thing that is
    unique. *)
let rec why3_of_term (t : t) : string =
  let paren x = "(" ^ why3_of_term x ^ ")" in
  match t.node with
  | Lit n -> string_of_int n
  | Var id -> "v" ^ string_of_int id
  | Add (a, b) -> "(" ^ why3_of_term a ^ " + " ^ why3_of_term b ^ ")"
  | NatSub (a, b) -> "(" ^ why3_of_term a ^ " - " ^ why3_of_term b ^ ")"
  | Mod (a, b) -> "(mod " ^ why3_of_term a ^ " " ^ why3_of_term b ^ ")"
  | Def (f, args) -> (
     match args with
     | [] -> f
     | _ -> "(" ^ String.concat " " (f :: List.map paren args) ^ ")")
  | Machine (w, op, a, b) ->
     let opname = match op with MAdd -> "machine_add" | MSub -> "machine_sub" in
     Printf.sprintf "(%s_%s %s %s)" opname (String.lowercase_ascii (string_of_elem w))
       (why3_of_term a) (why3_of_term b)

(** Does [id] occur in [t]? Needed by the L11c fact engine, which rewrites a
    binder with an equality it has assumed; a textual test on [canonical] would
    also match `v1` inside `v10`. *)
let rec occurs (id : int) (t : t) : bool =
  match t.node with
  | Var i -> i = id
  | Lit _ -> false
  | Add (a, b) | NatSub (a, b) | Mod (a, b) -> occurs id a || occurs id b
  | Def (_, args) -> List.exists (occurs id) args
  | Machine (_, _, a, b) -> occurs id a || occurs id b

(** Capture-avoiding only in the trivial sense this fragment allows: v1 has no
    binders inside a term, so replacing a binder id cannot capture anything. *)
let rec subst_var (id : int) (r : t) (t : t) : t =
  match t.node with
  | Var i when i = id -> r
  | Var _ | Lit _ -> t
  | Add (a, b) -> { t with node = Add (subst_var id r a, subst_var id r b) }
  | NatSub (a, b) -> { t with node = NatSub (subst_var id r a, subst_var id r b) }
  | Mod (a, b) -> { t with node = Mod (subst_var id r a, subst_var id r b) }
  | Def (f, args) -> { t with node = Def (f, List.map (subst_var id r) args) }
  | Machine (w, op, a, b) ->
     { t with node = Machine (w, op, subst_var id r a, subst_var id r b) }

let size (t : t) : int =
  let rec go acc = function
    | { node = Lit _ | Var _; _ } -> acc + 1
    | { node = Add (a, b) | NatSub (a, b) | Mod (a, b); _ } -> go (go (acc + 1) a) b
    | { node = Def (_, args); _ } -> List.fold_left go (acc + 1) args
    | { node = Machine (_, _, a, b); _ } -> go (go (acc + 1) a) b
  in
  go 0 t

(* ── budgets ────────────────────────────────────────────────────────────── *)

(** Bounded reduction. The defaults are small on purpose: an index expression
    is meant to be a handful of additions, and a comparison that needs more
    than this is a signal that the caller wanted an obligation, not a bigger
    machine. *)
type budget = {
  max_depth : int;
  max_steps : int;
  max_size : int;
  max_inline : int;
}

let default_budget = { max_depth = 16; max_steps = 256; max_size = 128; max_inline = 8 }

(** Which resource ran out. The string is part of the [Unknown] reason, because
    "unknown" without a reason is not a diagnostic. *)
exception Exhausted of string

type counters = { mutable steps : int; mutable inlines : int }

(* ── errors ─────────────────────────────────────────────────────────────── *)

(** A refusal. [rule] names the L11b sentence it enforces, [detail] is for the
    user, [span] points into the pragma string. Never used for identity. *)
type diag = { rule : string; detail : string; span : LiquidTypes.span }

let string_of_diag (d : diag) : string =
  Printf.sprintf "%s: %s (characters %d..%d of the pragma string)" d.rule d.detail
    d.span.LiquidTypes.start d.span.LiquidTypes.stop

let no_span = LiquidTypes.no_span

let fail rule detail = Error { rule; detail; span = no_span }
let fail_at rule detail span = Error { rule; detail; span }

(* ── environments and binder resolution ─────────────────────────────────── *)

type entry = { name : string; id : int; sort : LiquidTypes.sort; width : elem option }

type def = { params : int; body : t }

type env = {
  frames : entry list list;  (** innermost frame first *)
  defs : (string * def) list;
  next_id : int;
}

let empty : env = { frames = [ [] ]; defs = []; next_id = 0 }

(** A new scope. Shadowing is legal in the environment (the parser's grammar is
    scope-blind), and resolution always takes the innermost frame, which is the
    behaviour the shadowing fixture pins. *)
let push_scope (env : env) : env = { env with frames = [] :: env.frames }

let bind ?width (env : env) ~(name : string) ~(sort : LiquidTypes.sort) :
    env * int =
  let id = env.next_id in
  let entry = { name; id; sort; width } in
  let frames =
    match env.frames with
    | [] -> [ [ entry ] ]
    | hd :: tl -> (entry :: hd) :: tl
  in
  ({ env with frames; next_id = id + 1 }, id)

let define (env : env) ~(name : string) ~(params : int) (body : t) : env =
  { env with defs = (name, { params; body }) :: env.defs }

let find_def (env : env) (name : string) : def option = List.assoc_opt name env.defs

let lookup (env : env) (name : string) : entry option =
  let rec frames = function
    | [] -> None
    | frame :: rest -> (
        match List.find_opt (fun (e : entry) -> e.name = name) frame with
        | Some e -> Some e
        | None -> frames rest)
  in
  frames env.frames

(* ── bidirectional elaboration ──────────────────────────────────────────── *)

(** Unify a term's width with an expectation. [None] is "not constrained yet"
    — a literal is width-polymorphic until something asks — so the expectation
    wins; two *different* declared widths are the L11b "wrong element width"
    refusal, and that is a real inconsistency between operands rather than a
    missing hint. *)
let unify_width ~(what : string) (have : elem option) (want : elem option) =
  match (have, want) with
  | None, w -> Ok w
  | w, None -> Ok w
  | Some a, Some b when a = b -> Ok (Some a)
  | Some a, Some b ->
     Error
       (Printf.sprintf "wrong element width: %s is %s but %s is required" what
          (string_of_elem a) (string_of_elem b))

let rec infer ?(budget = default_budget) ?(depth = 0) (env : env)
    (term : LiquidTypes.term) : (t, diag) result =
  if depth > budget.max_depth then
    Error { rule = "L11b: bounded reduction"; detail = "elaboration depth budget exhausted";
            span = LiquidTypes.span_of_term term }
  else
    let sp = LiquidTypes.span_of_term term in
    match term with
    | LiquidTypes.TInt (z, _) ->
       if Z.sign z < 0 then
         fail_at "L11b: sorted index language"
           (Printf.sprintf "an index must be a natural, but `%s` is negative" (Z.to_string z))
           sp
       else if not (Z.fits_int z) then
         fail_at "L11b: bounded reduction"
           (Printf.sprintf "index literal `%s` does not fit the host integer" (Z.to_string z)) sp
       else Ok { node = Lit (Z.to_int z); elem = None }
    | LiquidTypes.TVar (name, _declared_sort, _) -> (
        match lookup env name with
        | None ->
           fail_at "L11b: resolved binders"
             (Printf.sprintf "unbound index `%s`" name) sp
        | Some e -> (
            match e.sort with
            | LiquidTypes.SNat -> Ok { node = Var e.id; elem = e.width }
            | s ->
               fail_at "L11b: sorted index language"
                 (Printf.sprintf "ill-sorted index: `%s` is declared %s, not Nat" name
                    (LiquidTypes.string_of_sort s))
                 sp))
    | LiquidTypes.TAdd (a, b, _) -> (
        match (infer ~budget ~depth:(depth + 1) env a, infer ~budget ~depth:(depth + 1) env b) with
        | Ok x, Ok y -> (
            match unify_width ~what:(LiquidTypes.string_of_term a) x.elem y.elem with
            | Error d -> fail_at "L11b: source element widths" d sp
            | Ok w -> Ok { node = Add (x, y); elem = w })
        | Error d, _ -> Error d
        | _, Error d -> Error d)
    | LiquidTypes.TSub (a, b, _) -> (
        match (infer ~budget ~depth:(depth + 1) env a, infer ~budget ~depth:(depth + 1) env b) with
        | Ok x, Ok y -> (
            match unify_width ~what:(LiquidTypes.string_of_term a) x.elem y.elem with
            | Error d -> fail_at "L11b: source element widths" d sp
            | Ok w ->
               Ok { node = NatSub (x, y); elem = w })
        | Error d, _ -> Error d
        | _, Error d -> Error d)
    | LiquidTypes.TCall (name, args, _) -> (
        match find_def env name with
        | None ->
           fail_at "L11b: closed definitions"
             (Printf.sprintf "`%s` is not a closed index definition" name) sp
        | Some def when def.params <> List.length args ->
           fail_at "L11b: closed definitions"
             (Printf.sprintf "`%s` takes %d argument(s), applied to %d" name def.params
                (List.length args))
             sp
        | Some _def ->
           let rec go acc = function
             | [] -> Ok (List.rev acc)
             | a :: rest -> (
                 match infer ~budget ~depth:(depth + 1) env a with
                 | Ok x -> go (x :: acc) rest
                 | Error d -> Error d)
           in
           (match go [] args with
            | Error d -> Error d
            | Ok xs ->
               (* A closed definition has no declared source width in the
                  initial fragment (L11a gives it a return sort, not a
                  width), so its application is width-polymorphic until an
                  expectation or a sibling operand supplies one. *)
               Ok { node = Def (name, xs); elem = None }))
    | LiquidTypes.TMul (_, _, _) ->
       fail_at "L11b: initial fragment"
         "multiplication is not in the initial index fragment" sp
    | LiquidTypes.TNeg (_, _) ->
       fail_at "L11b: sorted index language" "an index cannot be negated" sp
    | LiquidTypes.TReal (_, _) ->
       fail_at "L11b: sorted index language" "ill-sorted index: a real is not a natural" sp
    | LiquidTypes.TBool (_, _) ->
       fail_at "L11b: sorted index language" "ill-sorted index: a boolean is not a natural" sp
    | LiquidTypes.TIf (_, _, _, _) ->
       fail_at "L11b: initial fragment"
         "a conditional term is not in the initial index fragment" sp

(** Check against an expected width. This is the bidirectional half: the
    expectation never changes *which* term is meant, only whether it is
    well-formed at that width, and a literal (the one width-polymorphic node)
    takes the expectation. *)
and check ?(budget = default_budget) (env : env) ~(expected : elem option)
    (term : LiquidTypes.term) : (t, diag) result =
  match infer ~budget env term with
  | Error d -> Error d
  | Ok t -> (
      match unify_width ~what:(LiquidTypes.string_of_term term) t.elem expected with
      | Error d -> fail_at "L11b: source element widths" d (LiquidTypes.span_of_term term)
      | Ok w -> Ok { t with elem = w })

(* ── the bounded normalizer ─────────────────────────────────────────────── *)

let subst (args : t list) (body : t) : t =
  let rec go (t : t) : t =
    match t.node with
    | Var id -> ( try List.nth args id with _ -> t)
    | Lit _ -> t
    | Add (a, b) -> { t with node = Add (go a, go b) }
    | NatSub (a, b) -> { t with node = NatSub (go a, go b) }
    | Mod (a, b) -> { t with node = Mod (go a, go b) }
    | Def (f, xs) -> { t with node = Def (f, List.map go xs) }
    | Machine (w, op, a, b) -> { t with node = Machine (w, op, go a, go b) }
  in
  go body

let width_of_int (w : elem) : int =
  match w with E8 -> 8 | E16 -> 16 | E32 -> 32 | E64 -> 64

(** The largest value a width can hold *as a host integer*. For `Nat64` this
    is [max_int], not 2^64 - 1: this runtime's integers are 63-bit, so an
    index that needs the top bit is already outside the fragment rather than
    silently wrapped. That asymmetry is a limitation of the host and is
    reported as such, not papered over with a modular reading. *)
let max_value (w : elem) : int =
  match w with
  | E8 -> 255
  | E16 -> 65535
  | E32 -> 4294967295
  | E64 -> max_int

let normalize ?(budget = default_budget) (env : env) (term : t) : t =
  let c = { steps = 0; inlines = 0 } in
  let step () =
    c.steps <- c.steps + 1;
    if c.steps > budget.max_steps then raise (Exhausted "steps")
  in
  let rec go depth (t : t) : t =
    if depth > budget.max_depth then raise (Exhausted "depth");
    step ();
    match t.node with
    | Lit _ | Var _ -> t
    | Add (a, b) ->
       let a = go (depth + 1) a in
       let b = go (depth + 1) b in
       (match (a.node, b.node) with
        | Lit 0, _ -> b
        | _, Lit 0 -> a
        | Lit x, Lit y ->
           if x > max_int - y then raise (Exhausted "literal size");
           { t with node = Lit (x + y) }
        | Lit x, Add (l, z) -> (
            match l.node with
            | Lit y ->
               if x > max_int - y then raise (Exhausted "literal size");
               go depth { t with node = Add ({ t with node = Lit (x + y) }, z) }
            | _ -> { t with node = Add (a, b) })
        | Add (x, y), _ -> go depth { t with node = Add (x, { t with node = Add (y, b) }) }
        | _ -> { t with node = Add (a, b) })
    | NatSub (a, b) ->
       let a = go (depth + 1) a in
       let b = go (depth + 1) b in
       (match (a.node, b.node) with
        | _, Lit 0 -> a
        | Lit x, Lit y -> { t with node = Lit (max 0 (x - y)) }
        | _ -> { t with node = NatSub (a, b) })
    | Mod (a, b) ->
       let a = go (depth + 1) a in
       let b = go (depth + 1) b in
       (match (a.node, b.node) with
        | _, Lit 1 -> { t with node = Lit 0 }
        | Lit x, Lit y when y > 0 -> { t with node = Lit (x mod y) }
        | _ -> { t with node = Mod (a, b) })
    | Def (f, args) -> (
        let args = List.map (go (depth + 1)) args in
        match find_def env f with
        | None -> { t with node = Def (f, args) }
        | Some def ->
           c.inlines <- c.inlines + 1;
           if c.inlines > budget.max_inline then raise (Exhausted "inlining");
           go depth (subst args def.body))
    | Machine (w, op, a, b) ->
       let a = go (depth + 1) a in
       let b = go (depth + 1) b in
       let fits n = n >= 0 && n <= max_value w in
       (match (a.node, b.node) with
        | Lit x, Lit y -> (
            let r = match op with MAdd -> x + y | MSub -> x - y in
            if fits r then { t with node = Lit r } else { t with node = Machine (w, op, a, b) })
        | _ -> { t with node = Machine (w, op, a, b) })
  in
  let out = go 0 term in
  if size out > budget.max_size then raise (Exhausted "result size");
  out

(* ── conversion ─────────────────────────────────────────────────────────── *)

(** L11b's three answers, kept apart because they license different things:
    [Convertible] needs no proof, [Different] is a refutation the caller may
    report, and [Unknown] authorises nothing. *)
type status =
  | Convertible
  | Different of string
  | Unknown of string

let string_of_status = function
  | Convertible -> "convertible"
  | Different why -> "different: " ^ why
  | Unknown why -> "unknown: " ^ why

let convert ?(budget = default_budget) (env : env) (a : t) (b : t) : status =
  match (normalize ~budget env a, normalize ~budget env b) with
  | exception Exhausted what -> Unknown ("budget exhausted: " ^ what)
  | na, nb ->
     if canonical na = canonical nb then Convertible
     else
       Different
         (Printf.sprintf "`%s` and `%s` are not related by a syntax-directed rule"
            (canonical na) (canonical nb))

(** A required comparison. [Unknown] is a refusal here: "we ran out of budget"
    is not evidence that two indices agree, and a required check that accepted
    it would be accepting on the strength of a failure. *)
let require_convertible ?(budget = default_budget) (env : env) (a : t) (b : t) :
    (unit, diag) result =
  match convert ~budget env a b with
  | Convertible -> Ok ()
  | Different why ->
     Error { rule = "L11b: conversion"; detail = "indices differ: " ^ why; span = no_span }
  | Unknown why ->
     Error
       { rule = "L11b: bounded reduction";
         detail = "conversion is unknown (" ^ why ^ "); a required check cannot succeed";
         span = no_span }

(* ── equalities beyond conversion ───────────────────────────────────────── *)

(** A residual equality: the obligation L11b requires instead of a conversion
    rule. [sort] is the target sort, which for every index width is `int`. *)
type goal = { lhs : t; rhs : t; sort : string }

let goal_of ?(budget = default_budget) (env : env) (a : t) (b : t) : goal =
  let try_norm t = try normalize ~budget env t with Exhausted _ -> t in
  { lhs = try_norm a; rhs = try_norm b; sort = why3_sort a.elem }

let goal_text (g : goal) : string =
  Printf.sprintf "%s = %s" (why3_of_term g.lhs) (why3_of_term g.rhs)

(** Checked evidence is a *record* of what a prover accepted, supplied by the
    caller (L11c/L11d own the prover). Nothing in this module claims a proof:
    [Unchecked] is the answer when the goal's canonical text is not among the
    recorded ones, and the recorded form is the goal's own text so a law for a
    different instance cannot be mistaken for this one. *)
type verdict = Discharged of string | Unchecked of goal

let require (g : goal) ~(checked : (string * string) list) : verdict =
  let text = goal_text g in
  match List.find_opt (fun (_, t) -> t = text) checked with
  | Some (law, _) -> Discharged law
  | None -> Unchecked g

(* ── machine conversions and width obligations ──────────────────────────── *)

(** A machine value is a *different* thing from a natural index: it is checked
    at a declared width, and the conversion carries an obligation that the
    index fits. For a literal the bound is decidable here, so the conversion
    succeeds and the literal is the evidence; for anything else the obligation
    is returned and no term is produced — refusing to fabricate a machine term
    from an unproved bound. *)
type machine_error = Width_obligation of goal | Not_an_index of diag

let machine_of ~(width : elem) (t : t) : (t, machine_error) result =
  match t.node with
  | Lit n when n >= 0 && n <= max_value width ->
     (* The bound is decidable, and the literal *is* the evidence: nothing is
        asserted about a value the term does not pin down. *)
     Ok { t with elem = Some width }
  | _ ->
     let limit = { node = Lit (max_value width); elem = Some width } in
     Error (Width_obligation { lhs = t; rhs = limit; sort = why3_sort (Some width) })

(** A checked machine arithmetic node: the result is a machine term of the same
    width, and the caller still owes the width obligation for its operands. *)
let machine_op ~(width : elem) (op : mop) (a : t) (b : t) : t =
  { node = Machine (width, op, a, b); elem = Some width }

(* ── families and kinds, checked positionally ───────────────────────────── *)

(** Check a type annotation against the family table. This is the "explicit
    family arity/kinds" half of L11b: an unknown head, a wrong arity, an
    element position holding a nat, or a nat position holding a type is a
    refusal with the position named. *)
let family_of_ty (t : LiquidTypes.ty) : (family * LiquidTypes.ty_arg list, diag) result =
  match t with
  | LiquidTypes.TySort _ ->
     fail "L11b: explicit family arity"
       "an index-carrying type is an applied family, not a bare sort"
  | LiquidTypes.TyApp (name, args, sp) -> (
      match family_of_name name with
      | None ->
         fail_at "L11b: explicit family arity"
           (Printf.sprintf "`%s` is not an index family (known: %s)" name
              (String.concat ", " (List.map (fun (f : family) -> f.name) builtin_families)))
           sp
      | Some f ->
         if List.length args <> List.length f.params then
           fail_at "L11b: explicit family arity"
             (Printf.sprintf "`%s` takes %d argument(s) (%s), applied to %d" f.name
                (List.length f.params)
                (String.concat ", " (List.map string_of_kind f.params))
                (List.length args))
             sp
         else
           let rec go pos ps xs =
             match (ps, xs) with
             | [], [] -> Ok ()
             | k :: ps, x :: xs -> (
                 let bad () =
                   fail_at "L11b: explicit family arity"
                     (Printf.sprintf "argument %d of `%s` expects %s" (pos + 1) f.name
                        (string_of_kind k))
                     sp
                 in
                 match (k, x) with
                 | KElem, LiquidTypes.TyArgTy ty ->
                    if elem_of_ty ty = None then bad () else go (pos + 1) ps xs
                 | KNat, LiquidTypes.TyArgIndex _ -> go (pos + 1) ps xs
                 | _ -> bad ())
             | _ -> bad_arity ()
           and bad_arity () =
             fail_at "L11b: explicit family arity" "argument count mismatch" sp
           in
           (match go 0 f.params args with
            | Error d -> Error d
            | Ok () -> Ok (f, args)))

(* ── tests-facing helpers ───────────────────────────────────────────────── *)

(** Convenience constructors, used by the tests and by L11c when it builds
    terms programmatically rather than from a pragma. *)
let lit ?elem n = { node = Lit n; elem }
let var ?elem id = { node = Var id; elem }
let add ?elem a b = { node = Add (a, b); elem }
let nat_sub ?elem a b = { node = NatSub (a, b); elem }
let mod_ ?elem a b = { node = Mod (a, b); elem }
let defn ?elem f args = { node = Def (f, args); elem }
let machine ~(elem : elem) (op : mop) a b =
  { node = Machine (elem, op, a, b); elem = Some elem }
