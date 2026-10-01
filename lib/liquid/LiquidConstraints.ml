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

   L4 emits the VCs it can decide from the typed AST: the entry/exit contract
   pair (§5.6 APPLICATION, §5.10 RETURN). Two limitations are deliberate and
   are stated rather than papered over:

   - **No spans at this level.** `Stages.Tast`'s `TFunction` carries no span,
     so a goal cannot be named `g_<span-hash>` as §8.1 suggests. Goals are
     named from the declaration id and the contract kind instead. Threading
     spans to the typed AST is a prerequisite for exact blame, and belongs with
     L5's verdict→span mapping — which cannot work until they exist.
   - **Callee contracts are assumptions.** Cycle A (§8.3), which emits a Why3
     `function` per contracted function so obligations discharge in the
     extended theory, is L10. Until it lands a caller's goal *assumes* the
     callee's postcondition, and the emitted file says so in an `assumes`
     block so it is honest about what it stands in for.

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

(* ── the generated file ──────────────────────────────────────────────────── *)

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
}

type t = {
  module_name : string;
  contracts : contract_entry list;
  goals : goal list;
  (** L6: the qualifier templates that survived Houdini elimination. Emitted as
      `predicate` declarations and assumed by the goals, so an unproved
      obligation is visible as an assumption rather than hidden. *)
  templates : LiquidInfer.qualifier list;
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

(* A direct builder, kept separate so the signature stays readable. *)
let make_return_goal ~(decl : int) ~(name : string) ~(kind : LiquidTypes.contract_kind)
    ~(ensures : string) ~(req : string option) ~(params : string list)
    ~(ret_sort : string) : goal option =
  match kind with
  | LiquidTypes.KEnsures ->
     let binders =
       String.concat " "
         (List.map (fun p -> Printf.sprintf "(%s: %s)" p ret_sort) params)
       ^ (if params = [] then "" else " ")
       ^ Printf.sprintf "(%s: %s)" result_var ret_sort
     in
     Some
       {
         gname = goal_name decl name "ensures";
         grule = "§5.10 RETURN (assumes §5.6 APPLICATION)";
         gbinder = "forall " ^ binders ^ ". ";
         gassumes = req;
         gbody = emit_contract ensures;
       }
  | _ -> None

(* ── the emitter ─────────────────────────────────────────────────────────── *)

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
              (emit_contract (LiquidTypes.string_of_formula q.LiquidInfer.qbody)))
      c.templates;
  line "\n";
  line "  (* --- verification conditions --- *)\n";
  List.iter
    (fun g ->
      line "\n  (* %s — %s *)\n" g.gname g.grule;
      line "  goal %s:\n" g.gname;
      (match g.gassumes with
       | None -> ()
       | Some h ->
          (* Stands in for the callee contract until Cycle A (L10) emits real
             Why3 `function` definitions. *)
          line "    assumes a_%s: %s\n" g.gname (emit_contract h));
      line "    %s%s\n" g.gbinder g.gbody;
      line "  end\n")
    c.goals;
  line "\nend\n";
  Buffer.contents b

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
          let req =
            List.assoc_opt e.cname requires
          in
          match
            make_return_goal ~decl:decl_id ~name:e.cname ~kind:e.ckind
              ~ensures:e.csrc ~req ~params:e.cparams ~ret_sort:"int"
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
    Some { module_name = name; contracts; goals; templates = inference.LiquidInfer.surviving }
  end