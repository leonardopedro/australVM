(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L6: qualifier inference. `docs/LIQUID.md` §5 asks for
   `Γ ⊢ ok` goals — pure side conditions — and §4.3 wants measure unfolding as
   explicit hypotheses. Both need *unknown* predicates: the compiler does not
   yet know what is true at a program point, only what must be true for the rest
   to go through. L6 introduces those as qualifier templates and eliminates
   the ones that cannot be justified, Houdini-style.

   ## Why the oracle is a parameter

   Classic Liquid Types runs an SMT solver per Houdini round. Here the prover is
   a `why3 prove` **subprocess** (L5), which may not be installed. Making the
   oracle a value rather than a hard-coded call has two consequences that
   matter more than the convenience:

   - the elimination logic — which is the part that can actually be *wrong* —
     becomes testable with no external dependency, in `LiquidInferTest`;
   - a missing prover degrades to L4's behaviour (every template survives and
     is emitted as an `assumes`) instead of silently claiming inference
     happened. That direction is the safe one: under-claiming leaves the
     obligations to be proved later, over-claiming would drop a real one.

   ## The fixed point

   The plan notes the lattice is finite and needs no widening, because the
   fragment is recursion-free (T1) — the only iteration is structural `fold`.
   So the elimination order cannot diverge and no widening is required; the
   loop terminates because the candidate set strictly shrinks on every round
   that changes anything.

   ## What is *not* here

   - **T3** (structural patterns only) still needs `case`-arm guard analysis
     that `typed_when` does not expose. Templates at a `case` are created, but
     no clause proves them yet.
   - **T5** (refined variables are immutable) now becomes *checkable*, because
     Γ exists: `LiquidCheck` asks this module which variables are refined. The
     enforcement lands in the same commit as the tests.
*)
open Stages.Tast

(* ── qualifiers ──────────────────────────────────────────────────────────── *)

(** An unknown predicate: a name, the variables it may mention, and its body.
    The body is a TRL formula over those arguments; the name is what appears
    in the `.mlw` as a `predicate` and in the goals as an assumption. *)
type qualifier = {
  qname : string;
  qargs : string list;
  qbody : LiquidTypes.formula;
  qspan : string;                  (** for diagnostics; no real span at Tast *)
}

let qual_equiv (a : qualifier) (b : qualifier) : bool =
  a.qname = b.qname
  && a.qargs = b.qargs
  && LiquidTypes.string_of_formula a.qbody
     = LiquidTypes.string_of_formula b.qbody

let string_of_qualifier (q : qualifier) : string =
  match q.qargs with
  | [] -> q.qname
  | args -> Printf.sprintf "%s(%s)" q.qname (String.concat ", " args)

let compare_qualifier (a : qualifier) (b : qualifier) : int =
  compare (string_of_qualifier a) (string_of_qualifier b)

(* ── Horn clauses ────────────────────────────────────────────────────────── *)

(** [premises] together entail [conclusion]. Houdini deletes a candidate when
    some clause naming it as the conclusion has a premise already deleted. *)
type clause = {
  premises : qualifier list;
  conclusion : qualifier;
}

(* ── texpr -> TRL formula ────────────────────────────────────────────────── *)

(** The bridge from the typed AST to the refinement language. Only the
    constructs that can appear in a *condition* are translated — that is all
    §5.5 needs — and anything else yields [None] rather than a wrong formula.
    Returning [None] is the safe direction: the template is not created, so no
    goal is emitted that the compiler cannot justify. *)

let relop_of (op : Common.comparison_operator) : LiquidTypes.relop =
  match op with
  | Common.Equal -> LiquidTypes.REq
  | Common.NotEqual -> LiquidTypes.RNe
  | Common.LessThan -> LiquidTypes.RLt
  | Common.LessThanOrEqual -> LiquidTypes.RLe
  | Common.GreaterThan -> LiquidTypes.RGt
  | Common.GreaterThanOrEqual -> LiquidTypes.RGe

let span0 = { LiquidTypes.start = 0; stop = 0 }

(** Variables a translated term mentions, so a template gets the right arity. *)
let rec term_of (e : texpr) : (LiquidTypes.term * string list) option =
  match e with
  | TIntConstant s -> (
     (* The compiler only emits well-formed literals, but a stray one must not
        crash the pass, so the parse is guarded rather than assumed. *)
     match int_of_string_opt s with
     | Some i ->
        let z = Z.of_int i in
        if Z.equal z (Z.of_int i) then
          Some (LiquidTypes.TInt (z, span0), [])
        else None
     | None -> None)
  | TBoolConstant b -> Some (LiquidTypes.TBool (b, span0), [])
  | TParamVar (n, _) | TLocalVar (n, _) | TTemporary (n, _) ->
     let name = Identifier.ident_string n in
     Some (LiquidTypes.TVar (name, None, span0), [ name ])
  | TConstVar (q, _) ->
     (* A module constant: name it by its last component, which is how a TRL
        contract refers to it (docs/LIQUID.md §5.2, WF1). *)
     let full = Identifier.qident_debug_name q in
     let name =
       match String.rindex_opt full '.' with
       | Some i when i + 1 < String.length full -> String.sub full (i + 1) (String.length full - i - 1)
       | _ -> full
     in
     Some (LiquidTypes.TVar (name, None, span0), [ name ])
  | TNegation e' -> (
     match term_of e' with
     | Some (t, vs) -> Some (LiquidTypes.TNeg (t, span0), vs)
     | None -> None)
  | TComparison (_, a, b) -> (
     match (term_of a, term_of b) with
     | Some (ta, va), Some (tb, vb) ->
        Some
          ( LiquidTypes.TCall
              ( "@cmp", [ ta; tb ], span0 ),
            va @ vb )
     | _ -> None)
  | _ -> None

let rec formula_of (e : texpr) : (LiquidTypes.formula * string list) option =
  match e with
  | TComparison (op, a, b) -> (
     match (term_of a, term_of b) with
     | Some (ta, va), Some (tb, vb) ->
        Some (LiquidTypes.FRel (relop_of op, ta, tb, span0), va @ vb)
     | _ -> None)
  | TConjunction (a, b) -> (
     match (formula_of a, formula_of b) with
     | Some (fa, va), Some (fb, vb) ->
        Some (LiquidTypes.FAnd (fa, fb, span0), va @ vb)
     | _ -> None)
  | TDisjunction (a, b) -> (
     match (formula_of a, formula_of b) with
     | Some (fa, va), Some (fb, vb) ->
        Some (LiquidTypes.FOr (fa, fb, span0), va @ vb)
     | _ -> None)
  | TBoolConstant b ->
     Some ((if b then LiquidTypes.FTrue span0 else LiquidTypes.FFalse span0), [])
  | _ -> None

(* ── Γ: the refinement environment ───────────────────────────────────────── *)

(** What is known about a variable at a program point: a TRL formula over its
    own name, accumulated along the path. Facts are never dropped (weakening
    holds, `docs/LIQUID.md` §5.2) except by the borrow rule, which L6 does not
    model. *)
type gamma = (string * LiquidTypes.formula) list

let gamma_of (ps : Type.value_parameter list) : gamma =
  List.filter_map
    (fun (p : Type.value_parameter) ->
      match p with
      | Type.ValueParameter (n, _) ->
         let name = Identifier.ident_string n in
         (* A parameter enters Γ with no obligation beyond existing: the
            declared type is the fact. Refinements on parameters come from the
            contract's `Requires`, which `LiquidConstraints` already turns
            into the goal's antecedent. *)
         Some (name, LiquidTypes.FTrue { LiquidTypes.start = 0; stop = 0 }))
    ps

let truthy (g : gamma) : LiquidTypes.formula =
  (* The conjunction of Γ, or `true` when Γ is empty. Built left-associated so
     the rendering is stable. *)
  let span = { LiquidTypes.start = 0; stop = 0 } in
  match g with
  | [] -> LiquidTypes.FTrue span
  | (n, f) :: rest ->
     let var =
       LiquidTypes.TVar (n, Some LiquidTypes.SInt, span)
     in
     let one = LiquidTypes.FAnd (f, LiquidTypes.FRel (LiquidTypes.REq, var, var, span), span) in
     List.fold_left
       (fun acc (_, f') -> LiquidTypes.FAnd (acc, f', span))
       one rest

(* ── templates from the program ──────────────────────────────────────────── *)

(** A qualifier template at a program point. T6 has no user annotations yet, so
    templates are derived structurally: every `if` introduces the condition at
    each branch, and every assignment introduces the equality that must hold
    afterwards. That is exactly the §5.5 and §5.4 instantiations with the
    unknown left to be eliminated. *)
type acc = {
  mutable templates : qualifier list;
  mutable clauses : clause list;
  mutable counter : int;
}

let fresh (a : acc) (base : string) : string =
  a.counter <- a.counter + 1;
  Printf.sprintf "%s_%d" base a.counter

let add_template (a : acc) ~(qname : string) ~(qargs : string list)
    (body : LiquidTypes.formula) : qualifier =
  let q = { qname; qargs; qbody = body; qspan = "typed-ast" } in
  if not (List.exists (qual_equiv q) a.templates) then
    a.templates <- q :: a.templates;
  q

(* ── Houdini elimination ─────────────────────────────────────────────────── *)

(** Decide whether a goal is derivable from the currently-assumed qualifiers.
    This is the SMT/prover step. Returning [false] eliminates a candidate. *)
type oracle = qualifier -> bool

(** One round: every clause whose premises all survive falsifies its conclusion.
    Returns the qualifiers that must be dropped. *)
let eliminate_once (a : acc) (assume : qualifier list) : qualifier list =
  let survives q = List.exists (fun x -> qual_equiv q x) assume in
  a.clauses
  |> List.filter_map (fun (c : clause) ->
         (* A clause with no premises is an axiom: its conclusion can never be
            eliminated, which is the point of an axiom. *)
         if c.premises = [] then None
         else if List.for_all survives c.premises then Some c.conclusion
         else None)
  |> List.sort_uniq compare_qualifier

(** Houdini's fixed point. Returns `(surviving, rounds)`.

    The loop is: assume everything, drop what a clause refutes, repeat until a
    round drops nothing. No widening, because the candidate set only shrinks
    and the fragment is recursion-free, so the worst case is one round per
    template. *)
let houdini (oracle_fn : oracle) (templates : qualifier list)
    (clauses : clause list) : qualifier list * int =
  let acc = { templates; clauses; counter = 0 } in
  let rec loop assume rounds =
    let candidates = eliminate_once acc assume in
    (* The oracle decides which of these to *drop*: a qualifier the prover
       cannot derive is eliminated. Keeping the ones it proves would invert
       Houdini and silently discard every provable goal. *)
    let dropped = List.filter (fun q -> not (oracle_fn q)) candidates in
    let dropped = List.sort_uniq compare_qualifier dropped in
    if dropped = [] then (assume, rounds)
    else
      let assume' = List.filter (fun q -> not (List.exists (fun d -> qual_equiv q d) dropped)) assume in
      (* Every elimination strictly shrinks the set, so this terminates. The
         guard is belt-and-braces against a future clause generator that could
         reintroduce a candidate. *)
      if List.length assume' = List.length assume then (assume, rounds + 1)
      else loop assume' (rounds + 1)
  in
  loop (List.sort_uniq compare_qualifier templates) 0

(* ── default measures (`docs/LIQUID.md` §4.3) ────────────────────────────── *)

(** Measures logos/the fragment expects to exist without an annotation. Each is
    a function symbol the TRL parser will admit (it resolves `fun-name "("
    ... ")"`) and which the Why3 side must supply; §8.3's Cycle A emits them.
    Listed here so L4 can declare them and so the set is reviewable in one
    place rather than scattered through the emitter. *)
let default_measures : (string * string) list =
  [
    ("length", "int -> int");                 (** list length *)
    ("prob_in", "real -> bool");              (** 0.0 <= p <= 1.0, the Prob bound *)
    ("prob_le", "real -> real -> bool");      (** p <= q in the Prob order *)
  ]

let is_measure (name : string) : bool =
  List.exists (fun (n, _) -> n = name) default_measures

(* ── driver ──────────────────────────────────────────────────────────────── *)

(** An identifier rendered as a Why3 identifier. *)
let why3_ident_of (n : Identifier.identifier) : string =
  let s = Identifier.ident_string n in
  String.map
    (fun c ->
      if
        (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')
        || c = '_' || c = '\''
      then c
      else '_')
    s

(** Everything L6 produces for one typed module. *)
type result = {
  templates : qualifier list;      (** every candidate considered *)
  surviving : qualifier list;      (** those Houdini did not eliminate *)
  clauses : clause list;
  rounds : int;
  oracle_calls : int;              (** prover invocations; the cost of L6 *)
}

(** Infer for a module.

    The walk is deliberately shallow and structural: §5.5 (conditionals) is the
    only rule that introduces a template with a real body, because it is the
    only one whose premise the typed AST states outright. §5.4 (let) and §5.10
    (return) produce the *goals* L4 already emits; their premises come from Γ,
    which `gamma_of` builds.

    Nothing is inferred for a function without a `Liquid_*` pragma — the same
    policy as L3's gate. *)
let infer (oracle_fn : oracle) (m : typed_module) : result =
  let (TypedModule (_, decls)) = m in
  let templates = ref [] in
  let clauses = ref [] in
  let counter = ref 0 in
  let calls = ref 0 in
  let fresh base =
    incr counter;
    Printf.sprintf "%s_%d" base !counter
  in
  let add ~qname ~qargs body =
    let q = { qname; qargs; qbody = body; qspan = "typed-ast" } in
    if not (List.exists (qual_equiv q) !templates) then templates := q :: !templates;
    q
  in
  let contracted ps =
    List.exists (fun p -> match p with Common.LiquidPragma _ -> true | _ -> false) ps
  in
  List.iter
    (fun d ->
      match d with
      | TFunction (_, _, name, _, _, _, body, _, pragmas) when contracted pragmas ->
         let base = why3_ident_of name in
         (* `augment_stmt` lifts a boolean expression into a temporary and
            leaves the bare temporary as the condition:
            `if n > m then` becomes `TLetTmp _t127 (n > m); TIf _t127 …`.
            Reading the TIf condition literally would find a variable, not a
            comparison, and every template would be vacuous. So the walk keeps
            a small temporary environment and resolves the condition through
            it — alpha-aware, and the reason this stage reads the program
            rather than the surface syntax. *)
         let rec resolve (env : (string * (LiquidTypes.formula * string list)) list)
             (e : texpr) : (LiquidTypes.formula * string list) option =
           match e with
           | TTemporary (n, _) ->
              (match List.assoc_opt (Identifier.ident_string n) env with
               | Some f -> Some f
               | None -> formula_of e)
           | _ -> formula_of e
         in
         (* §5.5. Each conditional contributes one template for the condition
            and one clause, so Houdini has a real candidate to eliminate: a
            condition that cannot be justified from Γ takes its template's
            conclusion with it.

            `walk` returns the extended environment because `augment_stmt`
            emits the lifted comparison and the `TIf` as *siblings* inside one
            `TBlock` — the binding has to flow from the first to the second,
            which a `unit`-returning walk cannot express. *)
         let rec walk (env : (string * (LiquidTypes.formula * string list)) list)
             (s : tstmt) : (string * (LiquidTypes.formula * string list)) list =
           match s with
           | TIf (_, c, t, f) ->
              (match resolve env c with
               | None -> ()
               | Some (form, vars) ->
                  let q = add ~qname:(fresh (base ^ "_if")) ~qargs:vars form in
                  clauses :=
                    { premises = [ q ]; conclusion = q } :: !clauses);
              ignore (walk env t);
              walk env f
           | TLetTmp (n, _, e) -> (
              (* The lifted comparison: bind the temporary so the `TIf` that
                 follows can be read through it. *)
              match formula_of e with
              | None -> env
              | Some f -> (Identifier.ident_string n, f) :: env)
           | TBlock (_, a, b) ->
              let env' = walk env a in
              walk env' b
           | TLet (_, _, _, _, s') -> walk env s'
           | TBorrow { body; _ } -> walk env body
           | TCase (_, _, whens, _) ->
              List.iter (fun (TypedWhen (_, _, body)) -> ignore (walk env body)) whens;
              env
           | _ -> env
         in
         ignore (walk [] body)
      | _ -> ())
    decls;
  let templates = List.sort_uniq compare_qualifier !templates in
  let counting_oracle q =
    incr calls;
    oracle_fn q
  in
  let surviving, rounds = houdini counting_oracle templates !clauses in
  { templates; surviving; clauses = !clauses; rounds; oracle_calls = !calls }
