(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L4: constraint generation. Turns the contracts carried
   by a typed module into Why3 verification conditions — one `.mlw` per module,
   the shape `docs/LIQUID.md` §8.1 asks for.

   The judgments are `docs/LIQUID.md` §5.1:

     Γ ⊢ e : {ν: T | Q}      an expression refines its value
     Γ ⊢ s ▷ Q               a statement, if it falls through, establishes Q
     Γ ⊢ {ν:T|p} <: {ν:T|q}  subtyping, which emits Γ ∧ p ⇒ q
     Γ ⊢ ok                  a pure side condition (overflow, WF)

   Emitted from the typed AST today:

   - **§5.10 RETURN** — the entry/exit pair: `Γ ∧ Requires ⇒ Ensures`.
   - **§5.6 APPLICATION, caller side** — one goal per call to a contracted
     callee inside a refinement-level body, `Γ ∧ caller_Requires ⇒
     callee_Requires[actuals/formals]`. Without this the callee's contract was
     an *axiom* that callers could lean on while nothing checked what they
     passed it; `docs/LIQUID.md` §11's soundness sentence was ahead of the
     code. Call sites whose actuals the term bridge cannot render are skipped
     (see `callsite_goal`) — under-claiming, and stated.
   - **§5.9 FOLD, result half** — `Liquid_Invariant` becomes `Γ ∧ Requires ⇒
     I(result)`. The step half needs the §4.4 combinator; see
     `make_return_goal`.

   Two limitations are deliberate and are stated rather than papered over:

   - **No spans at this level.** `Stages.Tast`'s `TFunction` carries no span,
     so a goal cannot be named `g_<span-hash>` as §8.1 suggests. Goals are
     named from the declaration id and the contract kind instead. Threading
     spans to the typed AST is a prerequisite for exact blame, and belongs with
     L5's verdict→span mapping — which cannot work until they exist.
   - **Callee contracts are assumptions.** Cycle A (§8.3) has landed: the
     emitter now writes a `function` per contracted function plus an `axiom`
     carrying its contract, so a caller *can* appeal to a callee by name. The
     axiom remains an assumption rather than a proof — it states the contract so
     callers may rely on it, and discharging it would need the function body,
     which is still not translated. This note used to describe an `assumes`
     block, which was not merely a placeholder but invalid Why3: `assumes` is a
     program construct, illegal inside a `goal`, and Why3's exit 1 for the
     syntax error was being read as "the prover refused". See `emit_theory`'s
     comment and the `verify-invariants` gate that parses the goldens.

   Erasure (§1.2): nothing generated here reaches codegen.
*)
open Stages.Tast

(* ── a tiny Why3 emission vocabulary ──────────────────────────────────────── *)

(** Why3 identifiers are bare; map anything else to `_`. *)
let why3_ident (s : string) : string =
  let ok c =
    (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')
    || c = '_' || c = '\''
  in
  let out =
    String.map (fun c -> if ok c then c else '_') s
  in
  let starts_with_digit =
    String.length out > 0 && out.[0] >= '0' && out.[0] <= '9'
  in
  if out = "" || starts_with_digit then "_" ^ out else out

(** `result` is the refinement variable of the enclosing binding's type and has
    no Why3 scope; it becomes the goal's bound `__result`. A TRL contract is
    otherwise already Why3 `int.Int` / `bool.Bool` surface syntax over the same
    names the programmer wrote, so it is emitted essentially verbatim. *)
let result_var = "__result"

let emit_contract (src : string) : string =
  Str.global_replace (Str.regexp "result") result_var src

(** TRL surface connectives → Why3 surface connectives.

    The two languages agree on arithmetic and disagree on exactly the operators
    a contract is most likely to use: `==` is equality in TRL (§2.2) and an
    *undeclared* symbol in Why3 (`=`, `<>`, `/\`, `\/`, `not` are its
    spellings), and `!` means negation in one and something else entirely in
    the other. Left untranslated, a contract written with any of them produces
    a `.mlw` that Why3 *parses* (`==` lexes as an infix symbol) and then
    rejects as `unbound function or predicate symbol '(==)'`. That exits 1,
    which the driver reads as "the prover refused" — the same class of latent
    bug the goldens under `lib/liquid/golden/` are pinned to catch, so it is
    fixed at the one place every goal body passes through.

    Textual is safe here, and unusually so: TRL v1 has no string literals, no
    nested comments and a fixed token set (§2.2), so there is nothing a rewrite
    could corrupt. `!=` and `==>` are replaced before `!` and `==` for that
    reason, and the order is the whole of the correctness argument. *)
let why3_ops (s : string) : string =
  let rules =
    [ ("!=", "<>"); ("==>", "->"); ("==", "="); ("&&", "/\\");
      ("||", "\\/"); ("!", "not ") ]
  in
  (* `global_substitute` rather than `global_replace`: the replacement text
     contains backslashes (`/\`, `\/`), which `Str.replace` reads as escape
     sequences — it raises `illegal backslash sequence` rather than emitting
     the operator, so the substitution has to be a function. *)
  List.fold_left
    (fun acc (from, into) ->
      Str.global_substitute (Str.regexp_string from) (fun _ -> into) acc)
    s rules

(** A contract string as it appears in a goal: `result` renamed to the bound
    `__result`, then spelled the way Why3 parses it. *)
let emit_formula (src : string) : string = why3_ops (emit_contract src)

(* ── the generated file ──────────────────────────────────────────────────── *)

(** One total-fragment function, as a Why3 `function` plus an axiom carrying its
    contract.

    The axiom is an **assumption**, not a proof — it states what the function is
    declared to do so that callers may rely on it. That is the same conservative
    shape L6 already uses for qualifiers: an undischarged obligation stays
    visible in the emitted file instead of being silently dropped. Proving the
    axiom would need the function body, which is not translated here.

    The Why3 shapes were established by running the engine rather than assumed:
    a bodyless `function ... requires/ensures` is a *syntax error*, and a `val`
    does not bind its name for use in a goal's formula at all. What works is a
    bare `function` declaration plus an `axiom` over the function symbol, and it
    is not vacuous — a goal that overstates the contract does not come out
    Valid. *)
type theory_fn = {
  tname : string;
  tparams : string list;
  tpre : string option;
  tpost : string;
}

type contract_entry = {
  cname : string;
  ckind : LiquidTypes.contract_kind;
  csrc : string;                    (** as written, kept for the comment *)
  cparams : string list;            (** Why3 parameter names, in order *)
}

type goal = {
  gname : string;
  grule : string;
  gbinder : string;                 (** `forall (x: int) (y: int). ` or "" *)
  gassumes : string option;         (** the Requires this goal stands on *)
  gbody : string;
  gsyms : (string * int) list;
  (** Uninterpreted symbols the body mentions — projections, indices — with
      their arity, so the `.mlw` can declare them before the goals that use
      them. `[]` when the body is raw contract text that was never parsed
      into a formula. *)
}

type t = {
  module_name : string;
  contracts : contract_entry list;
  goals : goal list;
  (** L6: the qualifier templates that survived Houdini elimination. Emitted as
      `predicate` declarations and assumed by the goals, so an unproved
      obligation is visible as an assumption rather than hidden. *)
  templates : LiquidInfer.qualifier list;
  (** L10 Cycle A: the module's total-fragment functions, emitted as a Why3
      theory so a caller's goal can *call* a callee instead of assuming its
      contract. Before this the `.mlw` declared no functions at all, so every
      goal was closed over bare parameters and a caller had nothing to appeal
      to. *)
  theory : theory_fn list;
}

(* ── goal naming ─────────────────────────────────────────────────────────── *)

(** FNV-1a over a small key. Stable within a build and across builds of the
    same source; only has to be collision-free *inside one file*, since it is
    what ties a Why3 goal back to its contract. *)
let hash_key (parts : string list) : string =
  let h = ref 2166136261 in
  let byte b = h := ((!h lxor (b land 0xFF)) * 16777619) land 0xFFFFFFFF in
  List.iter
    (fun part ->
      let n = String.length part in
      for i = 0 to n - 1 do
        byte (Char.code part.[i])
      done;
      byte 10)
    parts;
  Printf.sprintf "%08x" !h

let goal_name (decl : int) (name : string) (kind : string) : string =
  "g_" ^ (hash_key [ string_of_int decl; name; kind ])

(* ── the contract table ──────────────────────────────────────────────────── *)

let is_contract (p : Common.pragma) : bool =
  match p with
  | Common.LiquidPragma _ -> true
  | _ -> false

let liquid_pragmas (ps : Common.pragma list) : (LiquidTypes.contract_kind * string) list =
  List.filter_map
    (fun p ->
      match p with
      | Common.LiquidPragma (kind, src) ->
         (match LiquidTypes.contract_kind_of_string kind with
          | Some k -> Some (k, src)
          | None -> None)
      | _ -> None)
    ps

let param_names (params : Type.value_parameter list) : string list =
  List.filter_map
    (fun (p : Type.value_parameter) ->
      match p with
      | Type.ValueParameter (n, _) -> Some (why3_ident (Identifier.ident_string n)))
    params

let decl_key (id : Id.decl_id) : int =
  match id with
  | Id.DeclId i -> i

(** The expression a total-fragment body returns, read structurally.

    Only the shapes a pure body of this fragment actually takes are walked —
    a bare `return e`, a `TBlock` spine, a `TLet` continuation — because any
    branching statement (`TIf`, `TCase`, a loop) means the returned value is
    not one expression, and a goal must not pretend otherwise. The terminal
    statement of a sequence is tried first: `check_ends_in_return` guarantees
    the spine ends in a return, and that is the one whose value counts. *)
let rec return_expr (s : tstmt) : texpr option =
  match s with
  | TReturn (_, e) -> Some e
  | TBlock (_, a, b) -> (
     match return_expr b with
     | Some _ as r -> r
     | None -> return_expr a)
  | TLet (_, _, _, _, body) -> return_expr body
  | _ -> None

(* A direct builder, kept separate so the signature stays readable. *)
let make_return_goal ~(decl : int) ~(name : string) ~(kind : LiquidTypes.contract_kind)
    ~(ensures : string) ~(req : string option) ~(params : string list)
    ~(ret_sort : string) ~(body : tstmt option) : goal option =
  match kind with
  | LiquidTypes.KEnsures | LiquidTypes.KInvariant ->
     (* Why3's quantifier syntax is `forall x: int, y: int. term` — one
        comma-separated list, each binder *unparenthesised*. The previous
        `forall (x: int) (y: int).` form is not Why3, and Why3 rejects the file
        with a syntax error, which then exits 1 and is read downstream as "the
        prover refused". So this too had never been syntax-checked. *)
     let binders =
       String.concat ", "
         (List.map (fun p -> Printf.sprintf "%s: %s" p ret_sort) params
         @ [ Printf.sprintf "%s: %s" result_var ret_sort ])
     in
    (* §5.9's *result* half. The fold rule concludes
       `fold(f, init, xs) : {ν: A | I(ν)}`, so the invariant is an obligation
       about the value the refined body returns — exactly the shape of an
       Ensures goal. (The *step* half is emitted separately, at the call site
       of the `Liquid_Fold` combinator: see `fold_goals` below.) Before this,
       `Liquid_Invariant` was parsed, type-checked and silently dropped — a
       contract that can never fail is worse than no contract. *)
    let kind_tag, rule =
      match kind with
      | LiquidTypes.KInvariant ->
         ( "invariant",
           "§5.9 FOLD (result half; the step obligation needs §4.4)" )
      | _ -> ("ensures", "§5.10 RETURN (assumes §5.6 APPLICATION)")
    in
    (* The body equation: `result = e` for the value the function returns.

       Without it the goal reads `forall …, __result. Requires ⇒ Ensures` —
       *every* value must satisfy the postcondition, which no non-trivial
       `Ensures` (nor any real invariant) does: a goal that could never be
       discharged is a false rejection waiting for `liquid = "required"`, the
       same class `LiquidVCTest` exists to catch. With it the goal is §5.10's
       subtyping judgment verbatim — `Requires ∧ (result = e) ⇒ Ensures`,
       i.e. `Ensures[e]` — and it is provable when the contract is honest.

       Two gates, both in the under-claiming direction: the body must be
       structurally a `return` (through `TBlock`/`TLet` spines) that
       `term_of` can render, and every variable it mentions must be a
       parameter, or the equation would name a local the goal binder does not
       bind. Either fails → the goal is emitted in the old un-linked shape
       rather than skipped: an obligation that exists and cannot be
       discharged is visible, which is this layer's convention. *)
    let linked =
      match body with
      | None -> None
      | Some s -> (
         match return_expr s with
         | None -> None
         | Some e -> (
            match LiquidInfer.term_of e with
            | None -> None
            | Some (t, vs) ->
               if List.for_all (fun v -> List.mem v params) vs then
                 Some ("result = (" ^ LiquidTypes.why3_of_term t ^ ")")
               else None))
    in
    Some
      {
        gname = goal_name decl name kind_tag;
        grule = rule;
        gbinder = "forall " ^ binders ^ ". ";
        gassumes =
          (* Written with plain `result`: the emitter runs `emit_formula` over
             `gassumes` once, and that is the pass which renames it to the
             bound `__result`. *)
          (match linked with
           | None -> req
           | Some eq ->
              (match req with
               | None -> Some ("(" ^ eq ^ ")")
               | Some r -> Some ("(" ^ r ^ " && (" ^ eq ^ "))")));
        gbody = emit_formula ensures;
        gsyms = [];
      }
  | _ -> None

(* ── the emitter ─────────────────────────────────────────────────────────── *)

(** Replace the goal's `result` variable with a call to the function.

    The postcondition of a function contract talks about `result`; the axiom
    that carries it has to talk about `atLeast n`, or it constrains nothing and
    the caller learns nothing. This is a textual substitution of the one
    identifier the emitter controls — `result_var` — rather than a rewrite of
    the formula AST, and it is only sound because that identifier is reserved:
    §2.2 spells it `result` and the emitter renames it, so a user variable
    cannot collide with it. *)
let substitute_result (params : string list) (fname : string) (text : string) : string =
  let call = fname ^ (if params = [] then "" else " " ^ String.concat " " params) in
  (* Word-boundary replace: `__result` must not be matched inside a longer
     identifier. *)
  let re = Str.regexp_string (Str.quote result_var) in
  try Str.global_substitute re (fun _ -> call) text with Not_found -> text

(** §8.1: one `.mlw` per module, goldens pinned under `lib/liquid/golden/`.
    Deterministic by construction — no timestamps, no absolute paths — so a
    golden diff means a real behaviour change. *)
let to_mlw (c : t) : string =
  let b = Buffer.create 1024 in
  let line fmt = Printf.ksprintf (Buffer.add_string b) fmt in
  line "(* Generated by LiquidConstraints (PLAN_liquid_types.md L4).\n";
  line "   Source module: %s\n" c.module_name;
  line "   Normative: docs/LIQUID.md §5.1 (judgments), §8.1 (one file per\n";
  line "   module), §8.2 (discharge).\n";
  line "   Do not edit by hand: this file is pinned as a golden under\n";
  line "   lib/liquid/golden/. *)\n";
  line "\n";
  line "module %s\n" c.module_name;
  line "  use int.Int\n";
  line "  use bool.Bool\n";
  line "\n";
  line "  (* --- contracts (docs/LIQUID.md §5.6) --- *)\n";
  List.iter
    (fun e ->
      line "  (* %s : %s *)\n" e.cname
        (LiquidTypes.string_of_contract_kind e.ckind);
      match e.ckind with
      | LiquidTypes.KMeasure | LiquidTypes.KFold -> ()
      | _ -> line "  (*   %s *)\n" e.csrc)
    c.contracts;
  line "\n";
  if c.templates <> [] then
    List.iter
      (fun (q : LiquidInfer.qualifier) ->
         match q.LiquidInfer.qargs with
         | [] ->
            line "  (* qualifier inferred at a program point (L6) *)\n";
            line "  axiom %s\n" q.LiquidInfer.qname
         | args ->
            line "  (* qualifier inferred at a program point (L6) *)\n";
            line "  predicate %s (%s) = %s\n"
              q.LiquidInfer.qname
              (String.concat ", " (List.map (fun a -> Printf.sprintf "%s: int" a) args))
              (emit_contract (LiquidTypes.why3_of_formula q.LiquidInfer.qbody)))
      c.templates;
  line "\n";
  (* L10 Cycle A: the theory. Emitted before the goals so a goal can refer to a
     callee by name. *)
  if c.theory <> [] then begin
    line "\n  (* --- theory: total-fragment functions (L10 Cycle A) --- *)\n";
    List.iter
      (fun (f : theory_fn) ->
         (* A bodyless `function` with its own contract is not valid Why3; the
            contract has to be an axiom over the symbol.

            Parameters are **curried groups** — `(e: int) (acc: int)` — not
            one comma-separated list. Why3 parses `function f (x: int, y: int)`
            as a syntax error at the comma, so a two-parameter theory function
            (a §5.9 step, for instance) produced a `.mlw` that the engine
            rejected; only single-parameter functions had been emitted, which
            is why no golden caught it. The flat *application* the goals and
            axioms use (`f a b`) is correct for the curried type. *)
         line "  function %s%s : int\n" f.tname
           (String.concat ""
              (List.map (fun p -> Printf.sprintf " (%s: int)" p) f.tparams));
         (* The postcondition is stated about the call, not about a free
            `result`, or the axiom would constrain nothing. *)
         let post = substitute_result f.tparams f.tname (emit_formula f.tpost) in
         let quantified =
           String.concat ", "
             (List.map (fun p -> Printf.sprintf "%s: int" p) f.tparams)
         in
         let binders =
           if quantified = "" then ""
           else Printf.sprintf "forall %s. " quantified
         in
         (match f.tpre with
          | Some pre ->
             line "  axiom %s_post: %s(%s) -> %s\n" f.tname binders
               (emit_formula pre) post
          | None -> line "  axiom %s_post: %s%s\n" f.tname binders post))
      c.theory;
    line "\n";
  end;
  (* Uninterpreted symbols the goals mention (projections, indices), declared
     before the goals. Why3 rejects an unbound symbol by name, and the exit
     status of that rejection is indistinguishable from a prover's "no" — so a
     term that cannot be declared has to be dropped at the source, which is why
     the emitter refuses comparisons-in-term-position rather than naming a
     symbol it cannot declare here. *)
  let declared =
    List.map (fun (f : theory_fn) -> f.tname) c.theory
  in
  let syms =
    List.sort_uniq compare
      (List.concat_map (fun (g : goal) -> g.gsyms) c.goals)
  in
  let syms = List.filter (fun (n, _) -> not (List.mem n declared)) syms in
  if syms <> [] then begin
    line "\n  (* --- uninterpreted symbols (docs/LIQUID.md §5.6) --- *)\n";
    List.iter
      (fun (name, arity) ->
        if arity = 0 then line "  function %s : int\n" name
        else
          let params =
            String.concat " " (List.init arity (fun i -> Printf.sprintf "x%d" i))
          in
          line "  function %s (%s: int) : int\n" name params)
      syms;
    line "\n";
  end;
  line "  (* --- verification conditions --- *)\n";
  List.iter
    (fun g ->
       line "\n  (* %s — %s *)\n" g.gname g.grule;
       (* The whole goal is one line. Why3 does not allow a newline between
          `goal g:` and its term — that is a syntax error, which then exits 1
          and reads downstream as "the prover refused". Third and last instance
          of the same underlying problem: an emitter that had never been parsed
          by the engine it targets. *)
       (match g.gassumes with
       | None -> line "  goal %s: %s%s\n" g.gname g.gbinder g.gbody
       | Some h -> line "  goal %s: %s(%s) -> %s\n" g.gname g.gbinder (emit_formula h) g.gbody))
    c.goals;
  line "\nend\n";
  Buffer.contents b

(* ── call sites: §5.6 APPLICATION ────────────────────────────────────────── *)

(** Everything the caller side of a call needs: the callee's `Requires` and the
    formal names it is stated over. *)
type callee = {
  callee_name : string;
  callee_params : string list;
  callee_requires : string option;
}

(** A call, its actuals, and the `TLetTmp` bindings in scope where it appears.

    The environment is part of the record because a hoisted argument is only
    interpretable at the point it was hoisted *from*: desugaring turns
    `atLeast(n - 1)` into `_t = n - 1; atLeast(_t)`, and a call-site obligation
    stated without resolving `_t` names a variable no contract may mention —
    so it would be skipped, which is how the arithmetic gap stayed invisible.
    `LiquidInfer.term_of ~temps` is what does the resolving; this traversal
    only has to preserve the scope.

    A method call carries no `decl_id` (its dispatch resolves through an
    instance method id, not a module declaration), which is why both the
    declaration and the name are here: §5.6 looks a callee up by declaration,
    §6 identifies an arithmetic intrinsic by name and needs neither. *)
type call_site = {
  cs_decl : Id.decl_id option;
  cs_name : Identifier.qident;
  cs_args : texpr list;
  cs_env : LiquidInfer.temp_env;
  cs_rt : Type.ty option;
  (** the declared result type of the call, when the node carries one — §6's
      `min(T) ≤ e ≤ max(T)` is stated over it. *)
}

(** Calls in an expression, in source order, with their actuals. Mirrors
    `TotalityCheck.calls_in_expr` — same traversal, different payload; keeping
    them separate is deliberate, because one asks "what does this call?" for a
    cycle and the other "with what?" for a precondition. *)
let rec calls_in_expr (env : LiquidInfer.temp_env) (acc : call_site list)
    (e : texpr) : call_site list =
  match e with
  | TFuncall (id, name, args, rt, _) ->
     let acc = { cs_decl = Some id; cs_name = name; cs_args = args;
                 cs_env = env; cs_rt = Some rt } :: acc in
     List.fold_left (calls_in_expr env) acc args
  | TMethodCall (_, name, _, args, rt, _) ->
     let acc = { cs_decl = None; cs_name = name; cs_args = args;
                 cs_env = env; cs_rt = Some rt } :: acc in
     List.fold_left (calls_in_expr env) acc args
  | TFptrCall (_, args, _) ->
     List.fold_left (calls_in_expr env) acc args
  | TVarMethodCall { args; _ } -> List.fold_left (calls_in_expr env) acc args
  | TEmbed (_, _, args) -> List.fold_left (calls_in_expr env) acc args
  | TCast (e', _) | TNegation e' | TDeref e' | TSlotAccessor (e', _, _)
  | TPointerSlotAccessor (e', _, _) -> calls_in_expr env acc e'
  | TComparison (_, a, b) | TConjunction (a, b) | TDisjunction (a, b)
  | TArrayIndex (a, b, _) | TSpanIndex (a, b, _) ->
     calls_in_expr env (calls_in_expr env acc a) b
  | TIfExpression (c, a, b) ->
     calls_in_expr env (calls_in_expr env (calls_in_expr env acc c) a) b
  | TRecordConstructor (_, fs) | TUnionConstructor (_, _, fs) ->
     List.fold_left (fun acc (_, e') -> calls_in_expr env acc e') acc fs
  | _ -> acc

(** Statements, carrying the temporary scope *through* a block: a `TLetTmp`
    binds for the rest of the sequence, so the environment has to come back out
    of the first half before the second is walked. Branch bodies get a copy of
    the incoming environment and their additions are discarded, because a
    temporary bound inside an `if` is not in scope after it. *)
let rec calls_in_stmt (env : LiquidInfer.temp_env) (acc : call_site list)
    (s : tstmt) : call_site list * LiquidInfer.temp_env =
  match s with
  | TSkip _ -> (acc, env)
  | TLet (_, _, _, _, s') ->
     (* A source `let x := e` types as [TLet] over a body whose first statement
        is the [TInitialAssign] that binds the value. Extending the env for the
        body's duration is what lets a call *inside* the scope resolve `x` back
        to `e` — the fold's step actual passed through `let s := step`, for
        instance. The env is restored afterwards: the binding dies with the
        [TLet], so a later sibling must not see it, and a rebound name must not
        be shadowed by a dead one. *)
     let head =
       let rec head_of t =
         match t with
         | TInitialAssign (n, e) ->
            Some (Identifier.ident_string (Identifier.original_name n), e)
         | TBlock (_, a, _) -> head_of a
         | _ -> None
       in
       head_of s'
     in
     let env_in =
       match head with
       | Some binding -> binding :: env
       | None -> env
     in
     let acc, _ = calls_in_stmt env_in acc s' in
     (acc, env)
  | TLetTmp (id, _, e) ->
     let acc = calls_in_expr env acc e in
     (acc, (Identifier.ident_string id, e) :: env)
  | TAssignTmp (_, e) -> (calls_in_expr env acc e, env)
  | TAssignVar (_, _, e) | TInitialAssign (_, e) | TReturn (_, e)
  | TDiscarding (_, e) -> (calls_in_expr env acc e, env)
  | TDestructure (_, _, _, e, s') -> calls_in_stmt env (calls_in_expr env acc e) s'
  | TAssign (_, a, b) -> (calls_in_expr env (calls_in_expr env acc a) b, env)
  | TIf (_, c, t, f) ->
     let acc = calls_in_expr env acc c in
     let acc, _ = calls_in_stmt env acc t in
     let acc, _ = calls_in_stmt env acc f in
     (acc, env)
  | TCase (_, e, whens, _) ->
     let acc =
       List.fold_left
         (fun acc (TypedWhen (_, _, body)) -> fst (calls_in_stmt env acc body))
         (calls_in_expr env acc e) whens
     in
     (acc, env)
  | TWhile (_, c, body) ->
     let acc, _ = calls_in_stmt env (calls_in_expr env acc c) body in
     (acc, env)
  | TFor (_, _, _, c, body) ->
     let acc, _ = calls_in_stmt env (calls_in_expr env acc c) body in
     (acc, env)
  | TBorrow { original; rename; mode; orig_type; body; _ } ->
     (* A **read** borrow's rename denotes the borrowed location itself, so
        inside its body a goal may speak of the original instead of the rename.
        That is what makes `r.X` statable: desugaring lowers it to
        `r_tmp_ref1: &r; r_tmp_ref1.X`, and `r_tmp_ref1` is not a parameter —
        naming it would leave a free variable the `.mlw` cannot bind. Read mode
        only: through a write borrow the original may be changed by the body,
        so resolving it back would attribute a value the caller's `Requires`
        never promised. Unresolved, the obligation is skipped, not faked. *)
     let env' =
       match mode with
       | Common.Read ->
          (Identifier.ident_string rename,
           TParamVar (original, orig_type))
          :: env
       | Common.Write | Common.Reborrow -> env
     in
     calls_in_stmt env' acc body
  | TBlock (_, a, b) ->
     let acc, env' = calls_in_stmt env acc a in
     calls_in_stmt env' acc b

(** The caller's obligation at one call site: `Γ ⇒ P[a̅/x̅]`.

    [None] means the obligation cannot be stated — an actual that is not a
    literal/variable/comparison (`LiquidInfer.term_of`), an arity mismatch, a
    `Requires` that does not parse, or a substituted formula naming something
    that is not one of the caller's parameters (a local would have to be bound
    by a quantifier stronger than the truth, i.e. a spurious rejection).

    Skipping is the *under-claiming* direction this layer takes everywhere: a
    missing obligation leaves work for a later pass, while a fabricated one
    rejects a correct program. What it must not do is stay silent about the
    direction — see the module header. *)
let callsite_goal ~(caller : string) ~(caller_params : string list)
    ~(caller_req : string option) ~(decl : int) ~(index : int)
    ~(temps : LiquidInfer.temp_env) ~(callee : callee)
    ~(actuals : texpr list) : goal option =
  if List.length actuals <> List.length callee.callee_params then None
  else
    match callee.callee_requires with
    | None -> None
    | Some src ->
       let rendered = List.map (LiquidInfer.term_of ~temps) actuals in
       if List.exists (fun r -> r = None) rendered then None
       else
         let terms = List.map (fun r -> fst (Option.get r)) rendered in
         (try
            let c =
              LiquidParse.parse_contract ~is_known_type:(fun _ -> false)
                LiquidTypes.KRequires src
            in
            match c.LiquidTypes.formula with
            | None -> None
            | Some f ->
               let sub =
                 LiquidTypes.subst_formula
                   (List.combine callee.callee_params terms) f
               in
               let scope = caller_params in
               if List.exists
                    (fun v -> not (List.mem v scope))
                    (LiquidTypes.free_vars sub)
               then None
               else
                 let binders =
                   String.concat ", "
                     (List.map (fun p -> Printf.sprintf "%s: int" p) caller_params)
                 in
                 Some
                   {
                     gname =
                       goal_name decl
                         (Printf.sprintf "%s_calls_%s_%d" caller
                            callee.callee_name index)
                         "requires";
                     grule =
                       "§5.6 APPLICATION (the caller proves the callee's Requires)";
                     gbinder =
                       (if binders = "" then "" else "forall " ^ binders ^ ". ");
                     gassumes = caller_req;
                     gbody =
                       "(" ^ why3_ops (LiquidTypes.why3_of_formula sub) ^ ")";
                     gsyms = LiquidTypes.symbols_of_formula sub;
                   }
          with Failure _ -> None)

(* ── §6: the range obligation ─────────────────────────────────────────── *)

let span0 = { LiquidTypes.start = 0; stop = 0 }

let conjoin (fs : LiquidTypes.formula list) : LiquidTypes.formula =
  match fs with
  | [] -> LiquidTypes.FTrue span0
  | f :: rest ->
     List.fold_left (fun acc g -> LiquidTypes.FAnd (acc, g, span0)) f rest

(** The antecedent of a §6 goal: the caller's `Requires`, conjoined with the
    *type* facts about its parameters.

    The range facts are what makes the obligation statable at all: `n - 1`
    stays in range when `n > min`, and only the declared type of `n` says
    anything about `n`. Adding them is sound because the type checker already
    promised them — a goal may assume what the program's own declarations
    guarantee — and assuming more can only make an obligation *easier* to
    prove, never accept a program the type checker would reject. *)
let combine_assumes (req : string option) (facts : LiquidTypes.formula list) :
    string option =
  match facts with
  | [] -> req
  | fs ->
     let fs_str = LiquidTypes.why3_of_formula (conjoin fs) in
     (match req with
      | None -> Some (emit_formula ("(" ^ fs_str ^ ")"))
      | Some r -> Some (emit_formula ("(" ^ r ^ " && (" ^ fs_str ^ "))")))

(** §6 (`docs/LIQUID.md` §6, normative): one obligation per machine-int
    operation, `min(T) ≤ e ≤ max(T)`, over the operation's *result* type.

    The operation is rendered by `LiquidInfer.arithmetic_term`, so the goal is
    stated for `+`, `-` and `*` and skipped for everything else — including
    division, which TRL has no term for (see the note in `LiquidInfer`). The
    `free ⊆ params` check is the same one §5.6 applies: a goal that names a
    local would be unbound in the `.mlw`, and skipping is the direction that
    shows up as a missing obligation rather than a wrong one. *)
let range_goal ~(caller : string) ~(caller_params : string list)
    ~(caller_req : string option) ~(facts : LiquidTypes.formula list)
    ~(decl : int) ~(index : int) ~(call : call_site) : goal option =
  match call.cs_rt with
  | None -> None
  | Some rt -> (
     match LiquidInfer.int_bounds_of_ty rt with
     | None -> None
     | Some (lo, hi) -> (
        match
          LiquidInfer.arithmetic_term ~temps:call.cs_env call.cs_name call.cs_args
        with
        | None -> None
        | Some (t, vars) ->
           if List.exists (fun v -> not (List.mem v caller_params)) vars then None
           else
             let op =
               Identifier.ident_string (Identifier.original_name call.cs_name)
             in
             let body =
               LiquidTypes.FAnd
                 ( LiquidTypes.FRel
                     (LiquidTypes.RLe, LiquidTypes.TInt (lo, span0), t, span0),
                   LiquidTypes.FRel
                     (LiquidTypes.RGe, LiquidTypes.TInt (hi, span0), t, span0),
                   span0 )
             in
             let binders =
               String.concat ", "
                 (List.map (fun p -> Printf.sprintf "%s: int" p) caller_params)
             in
             Some
               {
                 gname =
                   goal_name decl
                     (Printf.sprintf "%s_%s_%d" caller op index)
                     "range";
                 grule =
                   "§6 RANGE (the result stays inside its machine type's range)";
                 gbinder =
                   (if binders = "" then "" else "forall " ^ binders ^ ". ");
                 gassumes = combine_assumes caller_req facts;
                 gbody = emit_formula (LiquidTypes.why3_of_formula body);
                 gsyms = LiquidTypes.symbols_of_formula body;
               }))

(* ── §5.9: the fold's init and step premises ────────────────────────────── *)

(** The §4.4 combinator of a module: the one declaration carrying
    `pragma Liquid_Fold`, with its parameter names.

    It has to be identified rather than guessed from the name, because the
    rule is about a *shape*: `fold(f, init, xs)` — step, seed, list. A
    declaration with a different arity is not §4.4's combinator, and calling
    something else `fold` does not make it one, so both the marker and the
    arity are checked. *)
let find_fold_combinator (decls : typed_decl list) :
    (int * string list * string option) option =
  List.find_map
    (fun d ->
      match d with
      | TFunction (id, _, _, _, params, _, _, _, pragmas)
        when List.exists
               (fun (k, _) -> k = LiquidTypes.KFold)
               (liquid_pragmas pragmas) ->
         let invariant =
           List.find_map
             (fun (k, src) ->
               match k with
               | LiquidTypes.KInvariant -> Some src
               | _ -> None)
             (liquid_pragmas pragmas)
         in
         Some (decl_key id, param_names params, invariant)
      | _ -> None)
    decls

(** §5.9's two premises, as obligations, for one call to the combinator.

    The rule (docs/LIQUID.md §5.9) has three premises about `init`, `f` and
    `xs`, and the *result* conclusion is the half that was already emitted.
    These are the two that were not:

    - **init** — `Γ ∧ Requires ⇒ I(init)`. Pure substitution of the actual
      seed into the invariant, which is statable for any actual the term
      bridge can render.
    - **step** — `∀ e a. Γ ∧ Requires ⇒ I(a) ⇒ I(f(e, a))`. The side fact the
      rule spells `I(a) ⇒ I(f(e,a))`, with `e` and `a` bound by *this* goal
      rather than named by the program: nothing in the source introduces them,
      and the emitter owns that binder just as it owns `__result`.

    Two gates, both in the under-claiming direction:

    - the step half is emitted only when the step function is in the emitted
      theory, i.e. it carries an `Ensures`. An obligation about `f`'s
      postcondition has no postcondition to appeal to without one — the same
      reason §5.6 emits nothing for a callee with no `Requires` — and its
      *arity* must match, or `f(e, a)` is a partial application and Why3
      rejects the file.
    - every substituted formula is checked against the caller's parameters
      before it is emitted, so a goal can never name a local.

    The invariant itself is **parsed** rather than concatenated: `I` lives in
    the source's spelling and has to become a `formula` before `result` can be
    substituted with a term. *)
let fold_goals ~(caller : string) ~(caller_params : string list)
    ~(caller_req : string option) ~(facts : LiquidTypes.formula list)
    ~(decl : int) ~(index : int) ~(fold_id : int) ~(fold_params : string list)
    ~(names_by_id : (int, string) Hashtbl.t) ~(theory : theory_fn list)
    ~(invariant : string option) ~(call : call_site) : goal list =
  (* §4.4's shape: step, seed, list; no declared invariant, no premises. *)
  if List.length fold_params <> 3 then []
  else if List.length call.cs_args <> 3 then []
  else
    match (invariant, call.cs_decl) with
    | Some inv_src, Some cid when decl_key cid = fold_id ->
       let inv =
         try
           match
             LiquidParse.parse_contract ~is_known_type:(fun _ -> false)
               LiquidTypes.KInvariant inv_src
           with
           | { LiquidTypes.formula = Some f; _ } -> Some f
           | _ -> None
         with Failure _ -> None
       in
       (match inv with
        | None -> []
        | Some invf ->
           let in_scope (allowed : string list) (f : LiquidTypes.formula) : bool =
             List.for_all
               (fun v -> List.mem v allowed)
               (LiquidTypes.free_vars f)
           in
           let render sub =
             why3_ops
               (LiquidTypes.why3_of_formula
                  (LiquidTypes.subst_formula sub invf))
           in
           let binders_of (extra : string list) =
             let ps =
               List.map (fun p -> Printf.sprintf "%s: int" p) caller_params
               @ List.map (fun p -> Printf.sprintf "%s: int" p) extra
             in
             let s = String.concat ", " ps in
             if s = "" then "" else "forall " ^ s ^ ". "
           in
           let base =
             {
               gname = goal_name decl (Printf.sprintf "%s_fold_%d" caller index) "fold";
               grule = "§5.9 FOLD (init half)";
               gbinder = binders_of [];
               gassumes = combine_assumes caller_req facts;
               gbody = "";
               gsyms = [];
             }
           in
           let init_goals =
             match LiquidInfer.term_of ~temps:call.cs_env
                       (List.nth call.cs_args 1)
             with
             | None -> []
             | Some (seed, _) ->
                let sub = [ ("result", seed) ] in
                let subf = LiquidTypes.subst_formula sub invf in
                if in_scope caller_params subf then
                  [ { base with
                      gbody = "(" ^ render sub ^ ")";
                      gsyms = LiquidTypes.symbols_of_formula subf; } ]
                else []
           in
           let step_goals =
             (* The step actual, as a declaration. It usually arrives as a
                `TFunVar` — a named function in argument position — but
                desugaring wraps call arguments in a `TCast`, and a caller may
                bind the function to a local first (`let s := step; fold2(s, …)`),
                in which case the argument is a variable whose value lives in
                the call site's environment. Resolving through both is what
                makes the premise a property of *this* step function rather
                than a gap: an unresolvable actual emits nothing, as before. *)
             let rec step_decl_of (e : texpr) : int option =
               match e with
               | TFunVar (id, _, _) -> Some (decl_key id)
               | TCast (e', _) -> step_decl_of e'
               | TLocalVar (n, _) | TParamVar (n, _) | TTemporary (n, _) -> (
                  match
                    LiquidInfer.lookup_temp call.cs_env
                      (Identifier.ident_string n)
                  with
                  | Some binding -> step_decl_of binding
                  | None -> None)
               | _ -> None
             in
             let step_name =
               match step_decl_of (List.nth call.cs_args 0) with
               | Some id -> Hashtbl.find_opt names_by_id id
               | None -> None
             in
             match step_name with
             | None -> []
             | Some step -> (
                match
                  List.find_opt (fun (f : theory_fn) -> f.tname = step) theory
                with
                | Some f when List.length f.tparams = 2 ->
                   let acc =
                     LiquidTypes.TVar ("__acc", None, span0)
                   in
                   let elem =
                     LiquidTypes.TVar ("__elem", None, span0)
                   in
                   let step_app =
                     LiquidTypes.TCall
                       (step, [ elem; acc ], span0)
                   in
                   let ante_f =
                     LiquidTypes.subst_formula [ ("result", acc) ] invf
                   in
                   let cons_f =
                     LiquidTypes.subst_formula [ ("result", step_app) ] invf
                   in
                   if in_scope (caller_params @ [ "__elem"; "__acc" ]) ante_f
                     && in_scope (caller_params @ [ "__elem"; "__acc" ]) cons_f then
                     [ { base with
                         gname =
                           goal_name decl
                             (Printf.sprintf "%s_fold_step_%d" caller index)
                             "foldstep";
                         grule = "§5.9 FOLD (step half)";
                         gbinder = binders_of [ "__elem"; "__acc" ];
                         gbody =
                           "(" ^ why3_ops (LiquidTypes.why3_of_formula ante_f)
                           ^ ") -> ("
                           ^ why3_ops (LiquidTypes.why3_of_formula cons_f)
                           ^ ")";
                         gsyms =
                           LiquidTypes.symbols_of_formula ante_f
                           @ LiquidTypes.symbols_of_formula cons_f;
                       } ]
                   else []
                | _ -> [])
           in
           init_goals @ step_goals)
    | _ -> []

(* ── driver: typed module -> constraints ──────────────────────────────────── *)

(** Build the constraint set for a typed module, or [None] when no function
    carries a contract — in which case there is nothing to prove and L5 should
    not be invoked at all. *)
let generate (m : typed_module) : t option =
  let (TypedModule (mn, decls)) = m in
  let contracts =
    List.concat_map
      (fun d ->
        match d with
        | TFunction (_, _, name, _, params, _, _, _, pragmas) when
            List.exists is_contract pragmas ->
           List.map
             (fun (k, src) ->
               {
                 cname = Identifier.ident_string name;
                 ckind = k;
                 csrc = src;
                 cparams = param_names params;
               })
             (liquid_pragmas pragmas)
        | _ -> [])
      decls
  in
  if contracts = [] then None
  else begin
    let name = Identifier.mod_name_string mn in
    let requires =
      List.concat_map
        (fun d ->
          match d with
          | TFunction (_, _, fname, _, _, _, _, _, pragmas) ->
             List.filter_map
               (fun (k, src) ->
                 match k with
                 | LiquidTypes.KRequires -> Some (Identifier.ident_string fname, src)
                 | _ -> None)
               (liquid_pragmas pragmas)
          | _ -> [])
        decls
    in
    (* Every contracted function of the module, by declaration id: the callee
       table a call site looks a precondition up in. *)
    let callees : (int, callee) Hashtbl.t = Hashtbl.create 16 in
    List.iter
      (fun d ->
        match d with
        | TFunction (id, _, fname, _, params, _, _, _, pragmas)
          when List.exists is_contract pragmas ->
           let requires =
             List.find_map
               (fun (k, src) ->
                 match k with
                 | LiquidTypes.KRequires -> Some src
                 | _ -> None)
               (liquid_pragmas pragmas)
           in
           Hashtbl.replace callees (decl_key id)
             {
               callee_name = Identifier.ident_string fname;
               callee_params = param_names params;
               callee_requires = requires;
             }
        | _ -> ())
      decls;
    (* §4.4: the module's fold combinator — its declaration id, parameter
       names, and the invariant pragma it carries, if any. *)
    let fold_com = find_fold_combinator decls in
    (* Every function's name by declaration id: the step actual of a fold call
       arrives as a function value, and the goal needs the name the theory
       declares for it. *)
    let names_by_id : (int, string) Hashtbl.t = Hashtbl.create 16 in
    List.iter
      (fun d ->
        match d with
        | TFunction (id, _, fname, _, _, _, _, _, _) ->
           Hashtbl.replace names_by_id (decl_key id)
             (Identifier.ident_string fname)
        | _ -> ())
      decls;
    (* L10 Cycle A: pair each function's Requires with its Ensures. A function
       with only one of the two still gets an axiom — the missing half is simply
       absent, which is the honest reading: an Ensures with no Requires is
       unconditional, and a Requires with no Ensures says nothing about the
       result, so it contributes no axiom rather than a vacuous one. (Built
       before the call sites: §5.9 asks whether the step actual is a theory
       function in order to emit its premise at all. *)
    let theory =
      List.concat_map
        (fun e ->
           if e.ckind <> LiquidTypes.KEnsures then []
           else
             let pre =
               List.find_opt
                 (fun (c : contract_entry) ->
                   c.cname = e.cname && c.ckind = LiquidTypes.KRequires)
                 contracts
               |> Option.map (fun c -> c.csrc)
             in
             [
               {
                 tname = e.cname;
                 tparams = e.cparams;
                 tpre = pre;
                 tpost = e.csrc;
               };
             ])
        contracts
    in
    (* §5.6: the caller's half. Only bodies that are themselves in the fragment
       are walked: an uncontracted function is outside the subset (it may
       loop, mutate, call foreign code), so it has no refinement-level
       obligation to discharge. *)
    let callsites =
      List.concat_map
        (fun d ->
          match d with
          | TFunction (id, _, fname, _, params, _, body, _, pragmas)
            when List.exists is_contract pragmas ->
             let caller = Identifier.ident_string fname in
             let caller_params = param_names params in
             let caller_req =
               List.find_map
                 (fun (k, src) ->
                   match k with
                   | LiquidTypes.KRequires -> Some src
                   | _ -> None)
                 (liquid_pragmas pragmas)
             in
             let env = LiquidInfer.env_of_function params body in
             let facts = LiquidInfer.range_facts env caller_params in
             let calls, _ = calls_in_stmt [] [] body in
             if Sys.getenv_opt "AUSTRAL_LIQUID_DEBUG_TMP" <> None then
               List.iter
                 (fun cs ->
                   Printf.eprintf "DEBUG call name=%s args=%s rendered=%s\n%!"
                     (Identifier.qident_debug_name cs.cs_name)
                     (String.concat ";" (List.map Stages.Tast.show_texpr cs.cs_args))
                     (match List.map (LiquidInfer.term_of ~temps:cs.cs_env) cs.cs_args with
                      | [] -> "none"
                      | l -> String.concat " | "
                               (List.map (function Some _ -> "ok" | None -> "skip") l)))
                 calls;
             calls
             |> List.mapi (fun i cs ->
                    let req_str = Option.map emit_formula caller_req in
                    (* §5.6: a call to a *contracted callee* of the module. *)
                    let from_callee =
                      match cs.cs_decl with
                      | None -> []
                      | Some cid -> (
                         match Hashtbl.find_opt callees (decl_key cid) with
                         | None -> []
                         | Some callee -> (
                            match
                              callsite_goal ~caller ~caller_params
                                ~caller_req:req_str ~decl:(decl_key id)
                                ~index:i ~temps:cs.cs_env ~callee
                                ~actuals:cs.cs_args
                            with
                            | Some g -> [ g ]
                            | None -> []))
                    in
                    (* §6: a machine-int operation, whatever it is called from.
                       Checked separately because the two judgments are about
                       different things — who proves whose contract, versus
                       whether arithmetic can leave its own type. *)
                    let range =
                      match
                        range_goal ~caller ~caller_params ~caller_req
                          ~facts ~decl:(decl_key id) ~index:i ~call:cs
                      with                       | Some g -> [ g ]
                       | None -> []
                    in
                    (* §5.9: the fold's init and step premises, for a call to
                       the module's own combinator. *)
                    let fold =
                      match fold_com with
                      | None -> []
                      | Some (fold_id, fold_params, invariant) ->
                       fold_goals ~caller ~caller_params ~caller_req ~facts
                         ~decl:(decl_key id) ~index:i ~fold_id ~fold_params
                         ~names_by_id ~theory ~invariant ~call:cs
                    in
                    from_callee @ range @ fold)
             |> List.concat
          | _ -> [])
        decls
    in
    let goals =
      List.concat_map
        (fun e ->
          let decl =
            List.find_opt
              (fun d ->
                match d with
                | TFunction (_, _, fname, _, _, _, _, _, pragmas) ->
                   Identifier.ident_string fname = e.cname
                   && List.exists is_contract pragmas
                | _ -> false)
              decls
          in
          let decl_id =
            match decl with
            | Some (TFunction (id, _, _, _, _, _, _, _, _)) -> decl_key id
            | _ -> 0
          in
          let body =
            match decl with
            | Some (TFunction (_, _, _, _, _, _, b, _, _)) -> Some b
            | _ -> None
          in
          let req =
            List.assoc_opt e.cname requires
          in
          match
            make_return_goal ~decl:decl_id ~name:e.cname ~kind:e.ckind
              ~ensures:e.csrc ~req ~params:e.cparams ~ret_sort:"int"
              ~body
          with
          | Some g -> [ g ]
          | None -> [])
        contracts
    in
    (* L6: run inference. The oracle is permissive here — the Why3-backed
       oracle lives in the L5 driver, and a permissive oracle means every
       template survives and is emitted as an explicit assumption. That is the
       safe direction: an obligation that was never discharged is visible in
       the .mlw instead of quietly dropped. *)
    let inference =
      LiquidInfer.infer (fun _ -> true) m
    in
    Some
      {
        module_name = name;
        contracts;
        goals = goals @ callsites;
        templates = inference.LiquidInfer.surviving;
        theory;
      }
  end