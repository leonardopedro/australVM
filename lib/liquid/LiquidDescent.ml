(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   L11e: a separately admitted fragment for *direct* structural descent over
   one immutable inductive argument (docs/LIQUID_BEND_PLAN.md §L11e,
   PLAN_liquid_types.md §L11e).

   The existing L3 totality gate (T1) bans every call cycle. That is the right
   default, and this module does not weaken it: only a function that opts in
   with `pragma Liquid_Descent(void)` and passes the checks below is exempted,
   and it is exempted from T1 *only* for its own self-edge. Mutual recursion,
   effects, loops and foreign calls are still refused by L3.

   WHAT IS PROVED HERE. Nothing. The decrease is a *syntactic* relation: the
   recursive argument must be a binder introduced by a `case` whose scrutinee
   is the function's own parameter (or another binder already established as a
   subterm). The rank this refers to is the extracted `LiquidMeasure.fuel`,
   whose decreasingness obligation is proved in `liquid_measure.mlw`; this
   module only records it. "Termination alone supplies no parallel work balance
   or time bound" — the evidence line reports the checker's observed depth and
   the evaluator's step budget as *measurements*, never as a proof.

   WHAT IS NOT ADMITTED (first version): self-calls on the original argument,
   on a rebuilt value, or outside such a `case`; mutual recursion; higher-order
   or foreign calls; assignment, loops and mutable borrows. Keeping fragment
   small is what makes the exemption safe. An inability to prove a decrease
   leaves the definition outside the fragment, exactly as the card says.
*)
open Stages.Tast

let ident_name (n : Identifier.identifier) : string = Identifier.ident_string n

let is_marked (pragmas : Common.pragma list) : bool =
  List.exists
    (fun p ->
      match p with
      | Common.LiquidPragma (kind, _) ->
         (match LiquidTypes.contract_kind_of_string kind with
          | Some LiquidTypes.KDescent -> true
          | _ -> false)
      | _ -> false)
    pragmas

(* ── the syntactic subterm relation ─────────────────────────────────────── *)

(** A variable named in [vars], written as a parameter or a local. The typed
    tree uses both spellings for case binders, so a name-based test is the only
    one that survives erasure and binder renaming. *)
let named_var ~(temps : (string * string) list) (vars : string list)
    (e : texpr) : string option =
  match e with
  | TParamVar (n, _) | TLocalVar (n, _) ->
     let s = ident_name n in
     if List.mem s vars then Some s else None
  | TTemporary (n, _) -> List.assoc_opt (ident_name n) temps
  | _ -> None

let resolve ~(temps : (string * string) list) (vars : string list) (e : texpr) :
    string option =
  named_var ~temps vars e

let rec is_constructed (e : texpr) : bool =
  match e with
  | TRecordConstructor _ | TUnionConstructor _ -> true
  | TCast (e', _) | TDeref e' | TSlotAccessor (e', _, _)
  | TPointerSlotAccessor (e', _, _) -> is_constructed e'
  | TFuncall (_, _, args, _, _) | TMethodCall (_, _, _, args, _, _) ->
     List.exists is_constructed args
  | _ -> false

type finding =
  | Original
  | Rebuilt
  | Outside
  | Shape
  | Effect

let describe = function
  | Original ->
     "calls itself on the original argument; descent requires a proper \
      constructor subterm"
  | Rebuilt ->
     "calls itself on a rebuilt value; there is no decrease proof for it"
  | Outside ->
     "calls itself outside a `case` on its recursive argument"
  | Shape ->
     "calls itself with an argument shape this fragment cannot read"
  | Effect ->
     "uses an effect (assignment, loop or mutable borrow); the descent       fragment is pure"

type observation = {
  o_name : string;
  o_arg : string option;      (** the binder the recursive argument resolves to *)
  o_calls : int;              (** self-calls found *)
  o_depth : int;              (** deepest nested case on a subterm *)
}

(** Typing hoists a `case` scrutinee into a generated temporary. Collect those
    aliases in one pass: ids are unique, so a global map is enough and the
    statement walk need not thread an environment through `TBlock`. *)
let rec collect_aliases (acc : (string * string) list) (s : tstmt) :
    (string * string) list =
  match s with
  | TLetTmp (id, _, e) ->
     (match e with
      | TParamVar (n, _) | TLocalVar (n, _) | TTemporary (n, _) ->
         (ident_name id, ident_name n) :: acc
      | _ -> acc)
  | TLet (_, _, _, _, s') | TDestructure (_, _, _, _, s') ->
     collect_aliases acc s'
  | TIf (_, _, t, f) ->
     collect_aliases (collect_aliases acc t) f
  | TCase (_, _, whens, _) ->
     List.fold_left
       (fun acc (TypedWhen (_, _, body)) -> collect_aliases acc body)
       acc whens
  | TBorrow { body; _ } -> collect_aliases acc body
  | TBlock (_, a, b) -> collect_aliases (collect_aliases acc a) b
  | _ -> acc

let observe ~(name : string) ~(entry : Id.decl_id) ~(params : Type.value_parameter list)
    ~(body : tstmt) : observation * finding option =
  let params_names =
    List.filter_map
      (fun (p : Type.value_parameter) ->
        match p with Type.ValueParameter (n, _) -> Some (ident_name n))
      params
  in
  let self = match entry with Id.DeclId i -> i in
  let temps = collect_aliases [] body in
  let calls = ref 0 and depth = ref 0 and worst : finding option ref = ref None
  and arg : string option ref = ref None in
  let note f = if !worst = None then worst := Some f in
  let rec walk_expr ~temps ~subterms (e : texpr) : unit =
    match e with
    | TFuncall (id, _, args, _, _) ->
       let k = match id with Id.DeclId i -> i in
       if k = self then begin
         incr calls;
         (match args with
          | [ a ] ->
             (match named_var ~temps subterms a with
              | Some s -> arg := Some s
              | None ->
                 if named_var ~temps params_names a <> None then note Original
                 else if is_constructed a then note Rebuilt
                 else note Outside)
          | _ -> note Shape)
       end;
       List.iter (walk_expr ~temps ~subterms) args
    (* `+` and friends are elaborated to `TMethodCall` on `Austral.Pervasive`;
       rejecting the wrapper would reject every arithmetic descent. L3's T4
       still rules out foreign callees. *)
    | TMethodCall (_, _, _, args, _, _) ->
       List.iter (walk_expr ~temps ~subterms) args
    | TFptrCall (_, args, _) ->
       note Outside;
       List.iter (walk_expr ~temps ~subterms) args
    | TVarMethodCall { args; _ } ->
       note Outside;
       List.iter (walk_expr ~temps ~subterms) args
    | TCast (e', _) | TNegation e' | TDeref e' | TSlotAccessor (e', _, _)
    | TPointerSlotAccessor (e', _, _) | TArrayIndex (e', _, _)
    | TSpanIndex (e', _, _) -> walk_expr ~temps ~subterms e'
    | TComparison (_, a, b) | TConjunction (a, b) | TDisjunction (a, b) ->
       walk_expr ~temps ~subterms a;
       walk_expr ~temps ~subterms b
    | TIfExpression (c, a, b) ->
       walk_expr ~temps ~subterms c;
       walk_expr ~temps ~subterms a;
       walk_expr ~temps ~subterms b
    | TRecordConstructor (_, fs) | TUnionConstructor (_, _, fs) ->
       List.iter (fun (_, a) -> walk_expr ~temps ~subterms a) fs
    | TEmbed _ -> note Outside
    | TFunVar _ -> note Shape
    | _ -> ()
  and walk_stmt ~temps ~subterms ~level (s : tstmt) : unit =
    match s with
    | TSkip _ -> ()
    | TInitialAssign (_, e) -> walk_expr ~temps ~subterms e
    | TAssign _ | TAssignVar _ | TWhile _ | TFor _
    | TBorrow { mode = Common.Write; _ }
    | TBorrow { mode = Common.Reborrow; _ } ->
       note Effect
    | TLet (_, _, _, _, s') | TDestructure (_, _, _, _, s') ->
       walk_stmt ~temps ~subterms ~level s'
    | TLetTmp (_, _, e) -> walk_expr ~temps ~subterms e
    | TAssignTmp (_, e) | TDiscarding (_, e) | TReturn (_, e) ->
       walk_expr ~temps ~subterms e
    | TIf (_, c, t, f) ->
       walk_expr ~temps ~subterms c;
       walk_stmt ~temps ~subterms ~level t;
       walk_stmt ~temps ~subterms ~level f
    | TCase (_, scrutinee, whens, _) ->
       let descend =
         named_var ~temps (params_names @ subterms) scrutinee <> None
       in
       let level' = if descend then level + 1 else level in
       if level' > !depth then depth := level';
       List.iter
         (fun (TypedWhen (_, binders, body)) ->
           let subterms' =
             if descend then
               List.map
                 (fun (TypedBinding { name; _ }) -> ident_name name)
                 binders @ subterms
             else subterms
           in
           walk_expr ~temps ~subterms:subterms' scrutinee;
           walk_stmt ~temps ~subterms:subterms' ~level:level' body)
         whens
    | TBorrow { body; _ } -> walk_stmt ~temps ~subterms ~level body
    | TBlock (_, a, b) ->
       walk_stmt ~temps ~subterms ~level a;
       walk_stmt ~temps ~subterms ~level b
  in
  walk_stmt ~temps ~subterms:[] ~level:0 body;
  ({ o_name = name; o_arg = !arg; o_calls = !calls; o_depth = !depth }, !worst)

(** A module-level refusal, or [None] when every opted-in function is inside
    the fragment. Only functions carrying the marker are looked at: an ordinary
    recursive definition is L3's business and stays refused there. *)
let check (m : typed_module) : string option =
  let (TypedModule (_, decls)) = m in
  let rec go = function
    | [] -> None
    | TFunction (id, _, name, _, params, _, body, _, pragmas) :: rest
      when is_marked pragmas ->
       let name = Identifier.ident_string name in
       let _, finding = observe ~name ~entry:id ~params ~body in
       (match finding with
        | None -> go rest
        | Some f ->
           Some
             (Printf.sprintf
                "L11e: `%s` is not a direct structural descent: it %s. \
                 Opting in promises one recursive argument that is a proper \
                 constructor subterm in every self-call; remove the marker \
                 (L3 then refuses the cycle) or restructure the definition."
                name (describe f)))
    | _ :: rest -> go rest
  in
  go decls

(** Deterministic evidence, same line-oriented convention as the split pilot.
    All numbers are observations of the *checker*, not termination proofs. *)
let evidence (m : typed_module) : string =
  let (TypedModule (mod_name, decls)) = m in
  let b = Buffer.create 512 in
  let line fmt = Printf.ksprintf (fun s -> Buffer.add_string b (s ^ "\n")) fmt in
  line "# liquid descent evidence (L11e)";
  line "module: %s" (Identifier.mod_name_string mod_name);
  line "fragment: one immutable inductive argument, proper subterm in every self-call";
  line "rank: extracted fuel/weight from lib/liquid/subset/liquid_measure.mlw";
  line "fuel-sanity: fuel([])=%d fuel([1;2;3])=%d weight([2;3])=%d"
    (LiquidMeasure.fuel []) (LiquidMeasure.fuel [ 1; 2; 3 ])
    (LiquidMeasure.weight [ 2; 3 ]);
  line "evaluator-fuel: %d index steps (LiquidIndex.default_budget, a measurement)"
    LiquidIndex.default_budget.max_steps;
  let any = ref false in
  let folds = ref [] in
  List.iter
    (fun d ->
      match d with
      | TFunction (_, _, name, _, _, _, _, _, pragmas)
        when List.exists
               (fun p ->
                 match p with
                 | Common.LiquidPragma (k, _) ->
                    (match LiquidTypes.contract_kind_of_string k with
                     | Some LiquidTypes.KFold -> true
                     | _ -> false)
                 | _ -> false) pragmas ->
         folds := Identifier.ident_string name :: !folds
      | _ -> ())
    decls;
  (match !folds with
   | [] -> line "fold: none (no Liquid_Fold marker in this module)"
   | names ->
      line "fold: %s" (String.concat "," (List.rev names));
      line "correspondence: stated — a direct descent consumes one immutable \
            inductive argument, the same argument a marked fold consumes");
  List.iter
    (fun d ->
      match d with
      | TFunction (id, _, name, _, params, _, body, _, pragmas)
        when is_marked pragmas ->
         any := true;
         let o, finding = observe
           ~name:(Identifier.ident_string name) ~entry:id ~params ~body in
         line "descent: %s arg=%s self-calls=%d case-depth=%d verdict=%s"
           o.o_name
           (Option.value ~default:"<none>" o.o_arg)
           o.o_calls o.o_depth
           (match finding with None -> "held" | Some f -> describe f)
      | _ -> ())
    decls;
  if not !any then line "descent: none (no Liquid_Descent marker)";
  Buffer.contents b

(** Write the evidence when asked, off by default like the other dumps. *)
let dump_evidence (m : typed_module) : unit =
  match Sys.getenv_opt "AUSTRAL_LIQUID_DESCENT_EVIDENCE" with
  | None -> ()
  | Some path ->
     let oc = open_out path in
     output_string oc (evidence m);
     close_out oc
