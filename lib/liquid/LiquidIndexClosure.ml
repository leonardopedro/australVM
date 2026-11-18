(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L11a: audit admissibility over the reachable typed
   closure of an *index function*.

   `docs/LIQUID.md` §2.3 WF3 says every function symbol a refinement applies
   must be total-fragment-admissible. L3 enforced that only for functions that
   carry a `Liquid_*` pragma themselves; a helper an *index expression* calls
   was never looked at, and an unannotated helper may contain anything the
   language allows — a `while`, an `@embed`, a foreign call, or a call back to
   itself. This pass closes that hole at the typed seam: starting from each
   function symbol a contract applies, it walks the transitive closure of
   callees in the module and refuses the contract when any member is not an
   index definition.

   The policy is deliberately *closed*: the walk enumerates every `tstmt` and
   `texpr` constructor and refuses the ones it does not admit, rather than
   wildcarding them into "fine". A constructor added to the AST later therefore
   fails this pass until someone decides what an index definition may do with
   it, which is the safe direction for a checker whose output feeds Why3.

   Admitted (an "index definition"):
     - statements: `skip`, immutable-style `let`/`let tmp`, `:rest`, `if`,
       `case`, blocks, `return`, discarding a result, and at most one
       initialization per local (`var x := e` is a term binding, not state);
     - expressions: integer/boolean/nil literals, the module's own constants
       whose initializers are themselves admissible, parameters, locals,
       temporaries, casts, comparisons, boolean connectives, `if`-expressions,
       `sizeof`, calls to same-module functions, and the standard
       `trappingAdd`/`trappingSubtract`/`trappingMultiply` primitives.
     - reads that the type checker wraps a record parameter in: an immutable
       borrow, the deref that reads through it, and a slot projection. These
       were refused until L11c, and that refusal made every measure over a
       record inadmissible — `length(w) = w.len` is exactly a read borrow plus
       a projection, so a wrapper's length could not be a term at all. With
       mutation, foreign calls and pointers still refused, a read here is a
       read of a value nothing in the fragment can change.

   Refused: loops, re-assignment, mutable borrows, `@embed`, foreign functions,
   calls into another module, function values, function pointers, pointer slot
   access, buffer access, record/union construction, float/string literals,
   acyclic violations (cycles, named by path), unknown symbols, measure names
   that are not declared (except the §4.3 default measures), and closure/depth
   blowups.

   `trappingDivide` is *not* on the allowlist on purpose: it traps on a zero
   divisor, and the fragment has no partial operations. Division in an index
   needs the divisor's non-zeroness as a premise, which L11b's sorted index
   language is the place to state.

   Coverage shape: the GPU subset walk (`lib/gpu/Gpu_subset.ml`) was read for
   ideas — a `visiting` list for cycles, a depth cap, a closure-size cap, and a
   refusal that names the enclosing call — but its policy is not reused: that
   walk admits Nat32/Boolean kernels and refuses every borrow, while this one is
   about terms over integers and admits `if`/`case`/`let`. Sharing the policy
   would couple two fragments that only happen to look alike.

   Span reporting: `Tast` statements carry spans, so a refusal names the exact
   `file:line:column` of the statement that contains the offending construct,
   plus the typed call path. Pragmas themselves carry no span, so the call site
   is reported as the declaration and the character range inside the pragma
   string, which is the same convention `LiquidCheck` uses for contract errors.
*)
open Stages.Tast

let decl_key (id : Id.decl_id) : int =
  match id with Id.DeclId i -> i

exception Refused of string

(* ── the module's declaration tables ─────────────────────────────────────── *)

type signature = {
  sig_name : string;
  sig_foreign : bool;
  sig_body : tstmt option;
}

type env = {
  env_module : string;
  sigs : (int, signature) Hashtbl.t;
  by_name : (string, int) Hashtbl.t;
  constants : (string, texpr) Hashtbl.t;
}

let build (TypedModule (mname, decls)) : env =
  let env =
    {
      env_module = Identifier.mod_name_string mname;
      sigs = Hashtbl.create 32;
      by_name = Hashtbl.create 32;
      constants = Hashtbl.create 8;
    }
  in
  List.iter
    (fun d ->
      match d with
      | TFunction (id, _, name, _, _, _, body, _, _) ->
         let n = Identifier.ident_string name in
         Hashtbl.replace env.sigs (decl_key id)
           { sig_name = n; sig_foreign = false; sig_body = Some body };
         Hashtbl.replace env.by_name n (decl_key id)
      | TForeignFunction (id, _, name, _, _, _, _, _) ->
         let n = Identifier.ident_string name in
         Hashtbl.replace env.sigs (decl_key id)
           { sig_name = n; sig_foreign = true; sig_body = None };
         (* Registered by name too, so a contract naming a foreign function is
            refused as "foreign", not as "unknown" — the two defects read very
            differently to whoever wrote the contract. *)
         Hashtbl.replace env.by_name n (decl_key id)
      | TConstant (_, _, name, _, init, _) ->
         Hashtbl.replace env.constants (Identifier.ident_string name) init
      | _ -> ())
    decls;
  env

(* ── refusal reporting ───────────────────────────────────────────────────── *)

let max_depth = 32

let max_closure = 64

let position_string (Span.Position { line; column }) : string =
  Printf.sprintf "%d:%d" line column

(** `file:line:column` for a statement's span. The file is the compiled
    source's own name, so a reader can jump straight there. *)
let span_site (Span.Span { filename; startp; _ }) : string =
  Printf.sprintf "%s:%s" filename (position_string startp)

type ctx = {
  env : env;
  visiting : int list; (* call stack, innermost first *)
  finished : (int, unit) Hashtbl.t;
  inits : (string, unit) Hashtbl.t;
  consts : (string, unit) Hashtbl.t;
  count : int ref;
  loc : string option; (* innermost enclosing statement, `file:line:col` *)
  site : string; (* the contract that reached this point *)
  entry : string; (* the declaration whose contract is being audited *)
}

(** Is this a span the parser actually observed?

    Not every node in the typed tree carries one. The desugarer wraps the body
    of `let x: T := e;` in a `TBlock` whose span is [Span.empty_span], and a
    walk that took every span at face value would report `at :0:0` for an
    offence inside such a block — a location worse than none, because it looks
    like a place. So a location is only *replaced* when the span is real, and
    the nearest enclosing real statement is what gets reported. *)
let span_is_real (Span.Span { filename; _ }) : bool = filename <> ""

let with_loc (ctx : ctx) (sp : Span.span) : ctx =
  if span_is_real sp then { ctx with loc = Some (span_site sp) } else ctx

let current ctx : string =
  match ctx.visiting with
  | [] -> ctx.entry
  | id :: _ -> (
     match Hashtbl.find_opt ctx.env.sigs id with
     | Some s -> s.sig_name
     | None -> Printf.sprintf "decl#%d" id)

let path_string ctx : string =
  match List.rev ctx.visiting with
  | [] -> ""
  | ids ->
     String.concat " -> "
       (List.map
          (fun id ->
            match Hashtbl.find_opt ctx.env.sigs id with
            | Some s -> s.sig_name
            | None -> Printf.sprintf "decl#%d" id)
          ids)

(** One refusal format, so every message carries the same three facts: what the
    definition did, which contract asked for it, and where in the source. *)
let refuse ctx fmt : 'a =
  Printf.ksprintf
    (fun body ->
      let lines = [ Printf.sprintf "WF3 (§2.3): `%s` %s" (current ctx) body ] in
      let lines = lines @ [ "  contract: " ^ ctx.site ] in
      let lines =
        match path_string ctx with
        | "" -> lines
        | p -> lines @ [ "  typed call path: " ^ p ]
      in
      let lines =
        match ctx.loc with
        | None -> lines
        | Some site -> lines @ [ "  at " ^ site ]
      in
      raise (Refused (String.concat "\n" lines)))
    fmt

(* ── the walk ────────────────────────────────────────────────────────────── *)

let name_of (q : Identifier.qident) : string =
  Identifier.original_name q |> Identifier.ident_string

let module_of (q : Identifier.qident) : string =
  Identifier.source_module_name q |> Identifier.mod_name_string

(** The typed tree's integer arithmetic is the pervasive `trapping*` methods,
    not operators: `n + 1` is a `TMethodCall` whose name is `trappingAdd`. Those
    are the small allowlist of §L11a "standard primitives" — the machine's
    arithmetic, whose bodies (which abort on overflow, and so are not total in
    the source sense) are deliberately not audited as if they were index
    definitions. *)
let primitive_methods : string list =
  [ "trappingAdd"; "trappingSubtract"; "trappingMultiply" ]

let rec stmt (ctx : ctx) (s : tstmt) : unit =
  match s with
  | TSkip _ -> ()
  | TLet (sp, _, _, _, body) ->
     stmt (with_loc ctx sp) body
  | TDestructure (sp, _, _, e, body) ->
     let ctx = with_loc ctx sp in
     expr ctx e;
     stmt ctx body
  | TAssign (sp, _, _) ->
     refuse (with_loc ctx sp)
       "assigns through a slot; an index definition binds a value once \
        instead of mutating it"
  | TAssignVar (sp, q, _) ->
     refuse (with_loc ctx sp)
       "re-assigns `%s`; an index definition may initialize a local once, and \
        re-assignment makes it state rather than a term"
       (name_of q)
  | TInitialAssign (q, e) ->
     let n = name_of q in
     if Hashtbl.mem ctx.inits n then
       refuse ctx "initializes `%s` more than once" n
     else begin
       Hashtbl.replace ctx.inits n ();
       expr ctx e
     end
  | TIf (sp, c, t, f) ->
     let ctx = with_loc ctx sp in
     expr ctx c;
     stmt ctx t;
     stmt ctx f
  | TCase (sp, e, whens, _) ->
     let ctx = with_loc ctx sp in
     expr ctx e;
     List.iter (fun (TypedWhen (_, _, body)) -> stmt ctx body) whens
  | TWhile (sp, _, _) ->
     refuse (with_loc ctx sp)
       "uses `while`; only `fold` over lists and structural `match` iterate in \
        the Total Austral Subset"
  | TFor (sp, _, _, _, _) ->
     refuse (with_loc ctx sp)
       "uses `for`; only `fold` over lists and structural `match` iterate in \
        the Total Austral Subset"
  | TBorrow { span; mode = Common.Read; body; _ } ->
     (* An immutable borrow is how the type checker passes a record to a
        function, so a measure such as `length(w) = w.len` is *written* as a
        read borrow plus a slot projection. Refusing both made every measure
        over a record inadmissible, which L11c's range wrapper needs; a read
        borrow adds no state, so admitting it keeps the fragment's promise. A
        write or reborrow still is state and still is refused. *)
     stmt (with_loc ctx span) body
  | TBorrow { span; _ } ->
     refuse (with_loc ctx span)
       "borrows a value mutably; an index definition has no state to write"
  | TBlock (sp, a, b) ->
     let ctx = with_loc ctx sp in
     stmt ctx a;
     stmt ctx b
  | TDiscarding (sp, e) | TReturn (sp, e) ->
     expr (with_loc ctx sp) e
  | TLetTmp (_, _, e) -> expr ctx e
  | TAssignTmp (_, e) -> expr ctx e

(** Arguments are walked *before* the call itself, so a refusal names the
    innermost offending expression rather than the outer call that contains it
    — the plan asks for a forbidden expression nested under an argument. *)
and expr (ctx : ctx) (e : texpr) : unit =
  match e with
  | TNilConstant | TBoolConstant _ | TIntConstant _ | TParamVar _
  | TLocalVar _ | TTemporary _ | TSizeOf _ ->
     ()
  | TFloatConstant _ ->
     refuse ctx "uses a float literal; the index fragment is over integers and \
                 booleans"
  | TStringConstant _ ->
     refuse ctx "uses a string literal; the index fragment has no strings"
  | TConstVar (q, _) -> constant ctx q
  | TFunVar _ ->
     refuse ctx "takes a function as a value; an index definition applies its \
                 callees directly"
  | TFuncall (id, q, args, _, _) ->
     List.iter (expr ctx) args;
     call ctx id q
  | TMethodCall (_, q, _, args, _, _) ->
     List.iter (expr ctx) args;
     method_call ctx q
  | TVarMethodCall { method_name; args; _ } ->
     List.iter (expr ctx) args;
     refuse ctx "calls `%s` through a typeclass, whose implementation is \
                 selected at runtime and cannot be audited here"
       (name_of method_name)
  | TFptrCall (_, _, _) ->
     refuse ctx "calls a function pointer; an index definition has no \
                 higher-order calls"
  | TCast (e', _) -> expr ctx e'
  | TComparison (_, a, b) -> expr ctx a; expr ctx b
  | TConjunction (a, b) | TDisjunction (a, b) -> expr ctx a; expr ctx b
  | TNegation e' -> expr ctx e'
  | TIfExpression (c, a, b) ->
     expr ctx c;
     expr ctx a;
     expr ctx b
  | TRecordConstructor _ ->
     refuse ctx "constructs a record; the index fragment has no ADT values"
  | TUnionConstructor _ ->
     refuse ctx "constructs a union case; the index fragment has no ADT values"
  | TSlotAccessor (e', _, _) ->
     (* A projection out of a parameter or local is pure and total; it is the
        read side of the record measures L11c measures a wrapper with. The
        pointed-to form below stays refused, because following a pointer is
        not a term over the fragment's values. *)
     expr ctx e'
  | TPointerSlotAccessor (_, _, _) ->
     refuse ctx "reads a slot through a pointer; the index fragment has no \
                 pointers"
  | TArrayIndex (_, _, _) ->
     refuse ctx "indexes an array; buffer access is not part of an index \
                 definition"
  | TSpanIndex (_, _, _) ->
     refuse ctx "indexes a span; buffer access is not part of an index \
                 definition"
  | TEmbed (_, text, args) ->
     List.iter (expr ctx) args;
     refuse ctx "uses `@embed(\"%s\")`, whose effect is unmediated by the \
                 fragment" text
  | TDeref e' ->
     (* Reading through a reference is how a borrowed record's slot is reached,
        and this fragment admits no assignment, no foreign call and no method
        with a body to write through it — so a deref here is a read of a value
        nobody in the fragment can change. The *pointer* forms stay refused:
        following an `Address` is not a term over these values. *)
     expr ctx e'

and constant (ctx : ctx) (q : Identifier.qident) : unit =
  let n = name_of q in
  if module_of q <> ctx.env.env_module then begin
    let ctx = { ctx with loc = None } in
    refuse ctx "refers to constant `%s` from module `%s`; an index definition \
                 may only read this module's constants" n (module_of q)
  end
  else
    match Hashtbl.find_opt ctx.env.constants n with
    | None ->
       refuse ctx "refers to `%s`, which is not a constant of this module" n
    | Some init ->
       if not (Hashtbl.mem ctx.consts n) then begin
         Hashtbl.replace ctx.consts n ();
         expr ctx init
       end

and method_call (ctx : ctx) (q : Identifier.qident) : unit =
  let n = name_of q in
  if List.mem n primitive_methods then ()
  else
    refuse ctx "calls `%s`, which is not on the index fragment's primitive \
                allowlist (%s); standard arithmetic is admitted, everything \
                else needs a contract of its own"
      n (String.concat ", " primitive_methods)

and call (ctx : ctx) (id : Id.decl_id) (q : Identifier.qident) : unit =
  let n = name_of q in
  if module_of q <> ctx.env.env_module then
    refuse ctx "calls `%s` from module `%s`; an index definition may only call \
                functions of this module" n (module_of q)
  else
    match Hashtbl.find_opt ctx.env.sigs (decl_key id) with
    | None ->
       refuse ctx "calls `%s`, which is not a function declared in this module" n
    | Some s when s.sig_foreign ->
       refuse ctx "calls the foreign function `%s`; a foreign call is an \
                   unmediated effect" n
    | Some { sig_body = None; _ } ->
       refuse ctx "calls `%s`, which has no body in this module" n
    | Some { sig_body = Some body; _ } -> enter ctx (decl_key id) body

and enter (ctx : ctx) (id : int) (body : tstmt) : unit =
  if Hashtbl.mem ctx.finished id then ()
  else if List.mem id ctx.visiting then begin
    (* Name the cycle, not just the back edge: the reader needs every function
       on it. *)
    (* `visiting` is innermost-first, so the call order is its reverse; the
       cycle is the tail of that order starting at the repeated node. *)
    let rec drop = function
      | [] -> []
      | x :: rest -> if x = id then x :: rest else drop rest
    in
    let cycle = drop (List.rev ctx.visiting) @ [ id ] in
    let rendered =
      String.concat " -> "
        (List.map
           (fun k ->
             match Hashtbl.find_opt ctx.env.sigs k with
             | Some s -> s.sig_name
             | None -> Printf.sprintf "decl#%d" k)
           cycle)
    in
    refuse ctx "is in an index call cycle: %s" rendered
  end
  else if List.length ctx.visiting >= max_depth then
    refuse ctx "reaches more than %d nested index definitions; a closure this \
                deep is not a shape the fragment admits" max_depth
  else if !(ctx.count) >= max_closure then
    refuse ctx "reaches more than %d index definitions; a closure this large \
                is not a shape the fragment admits" max_closure
  else begin
    incr ctx.count;
    let inner =
      { ctx with visiting = id :: ctx.visiting;
                 inits = Hashtbl.create 8;
                 loc = None }
    in
    stmt inner body;
    Hashtbl.replace ctx.finished id ()
  end

(* ── entry: the function symbols a contract applies ──────────────────────── *)

(** Every `TCall` name in a formula, with the span of the call inside the
    pragma string, first-use order and deduplicated by name. `LiquidTypes`
    has a similar walk for `.mlw` declarations, but it drops the span and that
    is exactly what a rejection has to name. *)
let calls_of_formula (f : LiquidTypes.formula) : (string * LiquidTypes.span) list
    =
  let seen = ref [] in
  let add name sp =
    if not (List.exists (fun (n, _) -> n = name) !seen) then
      seen := (name, sp) :: !seen
  in
  let rec term (t : LiquidTypes.term) =
    match t with
    | LiquidTypes.TInt _ | LiquidTypes.TReal _ | LiquidTypes.TBool _
    | LiquidTypes.TVar _ ->
       ()
    | LiquidTypes.TAdd (a, b, _) | LiquidTypes.TSub (a, b, _)
    | LiquidTypes.TMul (a, b, _) ->
       term a;
       term b
    | LiquidTypes.TNeg (a, _) -> term a
    | LiquidTypes.TIf (c, a, b, _) ->
       term c;
       term a;
       term b
    | LiquidTypes.TCall (name, args, sp) ->
       add name sp;
       List.iter term args
  in
  let rec go (g : LiquidTypes.formula) =
    match g with
    | LiquidTypes.FTrue _ | LiquidTypes.FFalse _ -> ()
    | LiquidTypes.FRel (_, a, b, _) ->
       term a;
       term b
    | LiquidTypes.FNot (h, _) -> go h
    | LiquidTypes.FAnd (a, b, _) | LiquidTypes.FOr (a, b, _)
    | LiquidTypes.FImplies (a, b, _) ->
       go a;
       go b
    | LiquidTypes.FType (e, _, _) -> term e
  in
  go f;
  List.rev !seen

(* ── the pass ────────────────────────────────────────────────────────────── *)

let kind_name (k : LiquidTypes.contract_kind) : string =
  match k with
  | LiquidTypes.KRequires -> "Liquid_Requires"
  | LiquidTypes.KEnsures -> "Liquid_Ensures"
  | LiquidTypes.KInvariant -> "Liquid_Invariant"
  | LiquidTypes.KTrusted -> "Liquid_Trusted"
  | LiquidTypes.KMeasure -> "Liquid_Measure"
  | LiquidTypes.KFold -> "Liquid_Fold"

(** Audit one contract. A parse failure yields `None`: `LiquidCheck` has
    already rejected it, and reporting the same defect twice as two different
    errors would only confuse. *)
let audit_contract (env : env) ~(decl : string)
    ~(is_known_type : string -> bool) (kind : LiquidTypes.contract_kind)
    (src : string) : unit =
  match
    try Some (LiquidParse.parse_contract ~is_known_type kind src)
    with Failure _ -> None
  with
  | None -> ()
  | Some contract -> (
     match contract.LiquidTypes.formula with
     | None -> ()
     | Some f ->
        List.iter
          (fun (name, sp) ->
            let base =
              {
                env;
                visiting = [];
                finished = Hashtbl.create 8;
                inits = Hashtbl.create 8;
                consts = Hashtbl.create 8;
                count = ref 0;
                loc = None;
                site =
                  Printf.sprintf "on `%s` (%s, characters %d..%d of the pragma \
                                  string)"
                    decl (kind_name kind) sp.LiquidTypes.start sp.LiquidTypes.stop;
                entry = name;
              }
            in
            if LiquidInfer.is_measure name then ()
            else
              match Hashtbl.find_opt env.by_name name with
              | None ->
                 refuse base
                   "is applied as an index function, but no function of that \
                    name is declared in this module (v1 index terms have no \
                    imports)"
              | Some id -> (
                 match Hashtbl.find_opt env.sigs id with
                 | None -> ()
                 | Some s when s.sig_foreign ->
                    refuse base
                      "is applied as an index function, but it is a foreign \
                       function: a foreign call is an unmediated effect"
                 | Some { sig_body = None; _ } ->
                    refuse base
                      "is applied as an index function, but it has no body in \
                       this module"
                 | Some { sig_body = Some body; _ } -> enter base id body))
          (calls_of_formula f))

(** The module-level audit: every contract, every applied symbol, every
    reachable callee. Returns `None` when the module is clean or the first
    refusal otherwise. *)
let audit_module (m : typed_module) : string option =
  let env = build m in
  let is_known_type (n : string) =
    List.mem n (LiquidTypes.declared_type_names m)
  in
  let (TypedModule (_, decls)) = m in
  let audit_decl (d : typed_decl) : unit =
    let pragmas =
      match d with
      | TFunction (_, _, _, _, _, _, _, _, ps) -> ps
      | TForeignFunction (_, _, _, _, _, _, _, ps) -> ps
      | _ -> []
    in
    let decl_name =
      match d with
      | TFunction (_, _, name, _, _, _, _, _, _)
      | TForeignFunction (_, _, name, _, _, _, _, _) ->
         Identifier.ident_string name
      | TConstant (_, _, name, _, _, _) -> Identifier.ident_string name
      | TRecord (_, _, name, _, _, _, _) -> Identifier.ident_string name
      | TUnion (_, _, name, _, _, _, _) -> Identifier.ident_string name
      | TTypeClass (_, _, name, _, _, _) -> Identifier.ident_string name
      | TInstance _ -> "<instance>"
    in
    List.iter
      (fun p ->
        match p with
        | Common.LiquidPragma (kind_name, src) -> (
           match LiquidTypes.contract_kind_of_string kind_name with
           | Some kind -> audit_contract env ~decl:decl_name ~is_known_type kind src
           | None -> ())
        | _ -> ())
      pragmas
  in
  try
    List.iter audit_decl decls;
    None
  with Refused msg -> Some msg
