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

   - **T3** (structural patterns only) needs no work here: `typed_when` carries
     a constructor tag and binders and no guard field, so the rule holds by
     construction by the time any pass sees the tree. Templates at a `case`
     are still created, and no clause proves them yet — that is a template-
     generation gap, not a T3 violation.
   - **T5** (refined variables are immutable) is enforced in `TotalityCheck`,
     not here: it is a property of the body, and the set it quantifies over
     (contract-named parameters) is known from the declaration, not from Γ.
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

(** A temporary bound by `TLetTmp`, with the expression it stands for.

    The desugarer hoists a compound argument into a temporary before the call
    (`atLeast(n - 1)` becomes `let _t = n - 1; atLeast(_t)`), so a call-site
    obligation whose actual is arithmetic would otherwise be *unstatable*: the
    actual is a temporary, and a temporary is not one of the caller's
    parameters. Resolving it here — rather than at each use site — keeps
    `term_of` the single place that knows what a `texpr` denotes. *)
type temp_env = (string * texpr) list

let rec lookup_temp (env : temp_env) (name : string) : texpr option =
  match env with
  | [] -> None
  | (n, e) :: rest -> if n = name then Some e else lookup_temp rest name

(** Variables a translated term mentions, so a template gets the right arity.

    [temps] is the `TLetTmp` environment in scope at the point the expression
    appears; `term_of e` with no environment is what most callers want. *)
let rec term_of ?(temps : temp_env = []) (e : texpr)
  : (LiquidTypes.term * string list) option =
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
  | TTemporary (n, _) -> (
     (* A hoisted argument: resolve it to the expression it names, when the
        caller walked past the binding. Otherwise it is just a variable — and
        one that no contract may mention, since it is not a parameter, so the
        §5.6 scope check will skip the obligation. *)
     match lookup_temp temps (Identifier.ident_string n) with
     | Some binding -> term_of ~temps binding
     | None ->
        let name = Identifier.ident_string n in
        Some (LiquidTypes.TVar (name, None, span0), [ name ]))
  | TParamVar (n, _) ->
     let name = Identifier.ident_string n in
     Some (LiquidTypes.TVar (name, None, span0), [ name ])
  | TLocalVar (n, _) -> (
     (* A local resolves to its binding when the traversal walked past it — the
        same treatment `TTemporary` gets, and for the same reason: desugaring
        turns `r.X` into a borrow-and-load whose local (`r_tmp_ref1`) is not a
        parameter, so a goal naming it would be unbound. Resolution is scoped to
        exactly the bindings the traversal put in [temps] (`TLetTmp`, plus a
        read borrow's rename), so an unrelated local stays a local and falls
        through to the scope check in the caller. *)
     let name = Identifier.ident_string n in
     (match lookup_temp temps name with
      | Some binding -> term_of ~temps binding
      | None -> Some (LiquidTypes.TVar (name, None, span0), [ name ])))
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
     match term_of ~temps e' with
     | Some (t, vs) -> Some (LiquidTypes.TNeg (t, span0), vs)
     | None -> None)
  | TCast (e', _) ->
     (* Desugaring wraps call arguments and returns in `TCast` at the callee's
        parameter type, so *every* actual arrives cast — without this the
        §5.6 call-site obligation could never be stated for a parameter whose
        type was written down. Transparent for the same reason `docs/LIQUID.md`
        §5.10 gives: TRL models machine integers as mathematical integers, so a
        cast between integer sorts that the type checker accepted adds no fact
        and loses none. A cast that changes the *sort* (Int → Real) is
        likewise an injection, not a rounding, but it is left to §6's range
        obligations rather than modelled here. *)
     term_of ~temps e'
  | TComparison (_, a, b) -> (
     (* Deliberately **unstatable**, and dropped rather than approximated.

        A comparison in term position (`f(a < b)`) denotes a `Bool`, and the
        `.mlw` declares only int-sorted functions: rendering it as a function
        symbol needs a `predicate`, which means a second sort through the
        emitter, the theory section and the goal binder. Before this note the
        case rendered `@cmp(a, b)` — `@` is not a Why3 identifier, so the file
        did not parse, and a parse failure exits 1, which the driver reads as
        "the prover refused": a fabricated rejection, the one direction this
        layer never takes. Skipping leaves the obligation for a later pass. *)
     ignore (a, b);
     None)
  | TFuncall (_, name, args, _, _) | TMethodCall (_, name, _, args, _, _) ->
     arithmetic_term ~temps name args
  | TSlotAccessor (rec', slot, _) -> (
     (* A projection: `r.count`. TRL has no projection *syntax* — the parser
        cannot yet read `r.count` in a contract — so this can only make the
        term available to a goal that needs it, not to a contract that names
        it. Rendered as an uninterpreted function of the record, which is
        exactly the information the prover has: it may not assume anything
        about `@slot_count(r)` unless a contract says so, and a contract
        cannot yet say so. That is the under-claiming direction, and it is
        why the contract-side half of this is listed as a language change in
        docs/cas.md §4. *)
     match term_of ~temps rec' with
     | Some (tr, vs) ->
        let field = Identifier.ident_string slot in
        Some (LiquidTypes.TCall ("__slot_" ^ field, [ tr ], span0), vs)
     | None -> None)
  | TDeref e' ->
     (* Desugaring lowers `r.X` to a load: `TDeref (TSlotAccessor …)`. The load
        reads the value *at this program point*, which is exactly what the
        source expression denotes, so the deref adds no fact and loses none —
        the same argument `TCast` makes below. Without this the projection
        case above is unreachable from real source, which is how the record
        argument silently failed to get an obligation. *)
     term_of ~temps e'
  | TArrayIndex (a, i, _) | TSpanIndex (a, i, _) -> (
     match (term_of ~temps a, term_of ~temps i) with
     | Some (ta, va), Some (ti, vi) ->
        Some (LiquidTypes.TCall ("__index", [ ta; ti ], span0), va @ vi)
     | _ -> None)
  | _ -> None

(** §6.3 arithmetic desugars to a `pervasive` intrinsic (`TypeCheckExpr
    .augment_arithmetic`: `trappingAdd`, `trappingSubtract`,
    `trappingMultiply`, `trappingDivide`), with both operands wrapped in a cast.
    Rendering the first three lets a call site whose actual is `n - 1` state its
    §5.6 obligation instead of being skipped — the gap `term_of`'s `_ -> None`
    used to leave.

    Both shapes are matched because desugaring picks between them: a bare
    `f(x)` becomes `TFuncall`, while `n - 1` arrives as a `TMethodCall` on
    `Austral.Pervasive` (the arithmetic operators are written as methods).
    Leaving one out is exactly the kind of half-fix that reads as working.

    Two deliberate omissions:

    - **`trappingDivide`** is not a division term: TRL has none, and machine
      division truncates, so rendering it as mathematical division would be a
      *different* claim rather than a translation. `TSub`/`TAdd`/`TMul` are
      exact in the fragment because TRL's integers are mathematical and no
      range obligation is emitted (§11).
    - **The module must be `Austral.Pervasive`**, not merely the name: a user
      function called `trappingAdd` is theirs, and reading it as `+` would
      fabricate a fact about arbitrary code. *)
and arithmetic_term ?(temps : temp_env = []) (name : Identifier.qident)
    (args : texpr list) : (LiquidTypes.term * string list) option =
  let is_pervasive =
    Identifier.equal_module_name
      (Identifier.source_module_name name)
      BuiltIn.pervasive_module_name
  in
  if not is_pervasive then None
  else
    match (Identifier.ident_string (Identifier.original_name name), args) with
    | op, [ a; b ] -> (
       match (op, term_of ~temps a, term_of ~temps b) with
       | "trappingAdd", Some (ta, va), Some (tb, vb) ->
          Some (LiquidTypes.TAdd (ta, tb, span0), va @ vb)
       | "trappingSubtract", Some (ta, va), Some (tb, vb) ->
          Some (LiquidTypes.TSub (ta, tb, span0), va @ vb)
       | "trappingMultiply", Some (ta, va), Some (tb, vb) ->
          Some (LiquidTypes.TMul (ta, tb, span0), va @ vb)
       | _ -> None)
    | _ -> None

(* ── the sort environment (the §6 prerequisite) ─────────────────────────── *)

(** What a machine type means in TRL: the sort a variable is bound to, and —
    for the machine integers — the exact range its values live in.

    This is the environment §6 asks for: a range obligation `min(T) ≤ e ≤ max(T)`
    needs `T` from somewhere, and until now nothing in the pipeline carried a
    variable's type into the emitter. It also supplies the *facts* side of the
    same judgment: a goal about an `Int64` parameter may assume `min ≤ n ≤ max`,
    which is what makes a range VC about `n + 1` provable at all. *)

(** The TRL sort of a typed expression, when it has one.

    [None] is the honest answer for the types TRL has no word for — references,
    spans, function pointers, regions — and a caller of this function must
    treat [None] as "cannot be stated" rather than as "sort is int". *)
let sort_of_ty (t : Type.ty) : LiquidTypes.sort option =
  match t with
  | Type.Integer _ -> Some LiquidTypes.SInt
  | Type.Boolean -> Some LiquidTypes.SBool
  | Type.SingleFloat | Type.DoubleFloat -> Some LiquidTypes.SReal
  | Type.NamedType (q, _, _) ->
     Some (LiquidTypes.SAdt (Identifier.ident_string (Identifier.local_name q)))
  | _ -> None

(** The exact value range of a machine integer type.

    [WidthByteSize] and [WidthIndex] are deliberately [None]: their width is a
    property of the host ABI rather than of the type, and guessing 64 would
    make the range facts wrong on a 32-bit target. No bound, no obligation —
    the under-claiming direction. *)
let int_bounds_of_ty (t : Type.ty) : (Z.t * Z.t) option =
  let width w =
    match (w : Type.integer_width) with
    | Type.Width8 -> 8
    | Type.Width16 -> 16
    | Type.Width32 -> 32
    | Type.Width64 -> 64
    | Type.WidthByteSize | Type.WidthIndex -> raise Exit
  in
  match t with
  | Type.Integer (Type.Unsigned, w) -> (
     try
       let bits = width w in
       let max = Z.pred (Z.shift_left Z.one bits) in
       Some (Z.zero, max)
     with Exit -> None)
  | Type.Integer (Type.Signed, w) -> (
     try
       let bits = width w in
       let top = Z.shift_left Z.one (bits - 1) in
       Some (Z.neg top, Z.pred top)
     with Exit -> None)
  | _ -> None

(** Every variable a body mentions, with its type, keyed by name.

    Read off the variable *nodes* rather than off the bindings: every `texpr`
    variable carries its own `ty`, so there is no need to thread a scope through
    the traversal — and a node cannot disagree with its own type. A name bound
    twice (which the compiler's fresh temporaries allow) keeps its first
    occurrence, which is the binding every use after hoisting refers to. *)
let env_of_function (params : Type.value_parameter list) (body : tstmt) :
    (string * Type.ty) list =
  let add name ty acc =
    if List.exists (fun (n, _) -> n = name) acc then acc else (name, ty) :: acc
  in
  let collected = ref [] in
  List.iter
    (fun (Type.ValueParameter (n, ty)) ->
      collected := add (Identifier.ident_string n) ty !collected)
    params;
  let with_expr (e : texpr) =
    let rec go (e : texpr) =
      match e with
      | TParamVar (n, ty) | TLocalVar (n, ty) | TTemporary (n, ty) ->
         collected := add (Identifier.ident_string n) ty !collected
      | TCast (e', _) | TNegation e' | TDeref e' | TSlotAccessor (e', _, _)
      | TPointerSlotAccessor (e', _, _) -> go e'
      | TFuncall (_, _, args, _, _) | TMethodCall (_, _, _, args, _, _)
      | TFptrCall (_, args, _) -> List.iter go args
      | TVarMethodCall { args; _ } -> List.iter go args
      | TEmbed (_, _, args) -> List.iter go args
      | TComparison (_, a, b) | TConjunction (a, b) | TDisjunction (a, b)
      | TArrayIndex (a, b, _) | TSpanIndex (a, b, _) ->
         go a; go b
      | TIfExpression (c, a, b) ->
         go c; go a; go b
      | TRecordConstructor (_, fs) | TUnionConstructor (_, _, fs) ->
         List.iter (fun (_, e') -> go e') fs
      | _ -> ()
    in
    go e
  in
  let rec go_stmt (s : tstmt) =
    let with_var (n : Identifier.identifier) (ty : Type.ty) =
      collected := add (Identifier.ident_string n) ty !collected
    in
    match s with
    | TSkip _ -> ()
    | TLet (_, _, n, ty, s') ->
       with_var n ty;
       go_stmt s'
    | TLetTmp (n, ty, e) ->
       with_expr e;
       with_var n ty
    | TAssignTmp (_, e) | TAssignVar (_, _, e) | TInitialAssign (_, e)
    | TReturn (_, e) | TDiscarding (_, e) -> with_expr e
    | TDestructure (_, _, _, e, s') ->
       with_expr e;
       go_stmt s'
    | TAssign (_, a, b) ->
       with_expr a; with_expr b
    | TIf (_, c, t, f) ->
       with_expr c; go_stmt t; go_stmt f
    | TCase (_, e, whens, _) ->
       with_expr e;
       List.iter (fun (TypedWhen (_, _, body)) -> go_stmt body) whens
    | TWhile (_, c, body) | TFor (_, _, _, c, body) ->
       with_expr c; go_stmt body
    | TBorrow { body; _ } -> go_stmt body
    | TBlock (_, a, b) ->
       go_stmt a; go_stmt b
  in
  go_stmt body;
  !collected

(** The range facts a goal may assume about the names in [scope]: `min ≤ x ∧
    x ≤ max` for each integer-typed one.

    These are *type* facts, not refinements: they are what the declared type
    already promises, so assuming them in an obligation cannot accept a program
    the type checker would reject. Non-integer and unknown-width types
    contribute nothing. *)
let range_facts (env : (string * Type.ty) list) (scope : string list) :
    LiquidTypes.formula list =
  let span = span0 in
  List.filter_map
    (fun (name, ty) ->
      if not (List.mem name scope) then None
      else
        match int_bounds_of_ty ty with
        | None -> None
        | Some (lo, hi) ->
           let v = LiquidTypes.TVar (name, Some LiquidTypes.SInt, span) in
           Some
             (LiquidTypes.FAnd
                ( LiquidTypes.FRel
                    (LiquidTypes.RLe, LiquidTypes.TInt (lo, span), v, span),
                  LiquidTypes.FRel
                    (LiquidTypes.RGe, LiquidTypes.TInt (hi, span), v, span),
                  span ))
    )
    env

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
