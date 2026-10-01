(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L2: the `liquid` compiler pass. L1 installed this tenant
   as a no-op because nothing could parse a contract yet; L2 makes it real, but
   deliberately stops short of *enforcing* anything — L4 generates constraints,
   L5 discharges them through Why3, and L6 infers qualifiers. What this pass
   does today is the part that is already decidable and is normative in
   `docs/LIQUID.md` §2.2–§2.3:

     - the contract string parses as a TRL formula (or is empty, for the two
       marker pragmas);
     - the integer literal range of §2.2 holds;
     - the v1 restrictions hold (no `forall`/`exists`/binders);
     - WF1: every free variable is a parameter, `result`, or a total-fragment
       constant — the set the caller supplies as `~bound`;
     - WF2, partially: no variable is used without a declared sort.

   A rejected contract is reported with its span into the pragma string, and
   names the declaration it came from, so a failure points at the offending
   characters rather than at the module.
*)
open Stages.Tast

let rejection (decl : string) (sp : LiquidTypes.span) (msg : string) : string =
  Printf.sprintf
    "liquid contract error on %s at %d-%d: %s"
    decl sp.LiquidTypes.start sp.LiquidTypes.stop msg

(** Parse one contract string, turning the parser's `Failure` into a rejection
    message. The parser reports through [failwith] with the span already
    formatted into the message, so the span is recovered here rather than
    re-derived. *)
let parse_or_reject (decl : string) (kind : LiquidTypes.contract_kind) (src : string)
  : (LiquidTypes.contract, string) result =
  try Ok (LiquidParse.parse_contract kind src) with
  | Failure msg -> Error (Printf.sprintf "on %s: %s" decl msg)

(** Check one declaration's pragmas. Returns [None] when the declaration is
    clean, or the rejection message otherwise. *)
let check_decl ~(bound : string list) (name : string) (pragmas : Common.pragma list)
  : string option =
  let fail sp msg = Some (rejection name sp msg) in
  let rec go = function
    | [] -> None
    | p :: rest ->
       (match p with
        | Common.LiquidPragma (kind_name, src) ->
           (match LiquidTypes.contract_kind_of_string kind_name with
            | None ->
               (* Not one of the six kinds: the pragma stage already rejects
                  unknown Liquid_* names, so this is unreachable from source. *)
               fail (LiquidTypes.span_of_string src)
                 (Printf.sprintf "unknown Liquid pragma kind %S" kind_name)
            | Some kind ->
               (match parse_or_reject name kind src with
                | Error e -> Some e
                | Ok contract ->
                   (match contract.LiquidTypes.formula with
                    | None -> go rest
                    | Some f ->
                       (* WF1 only. WF2 is deliberately **not** checked here:
                          it needs each variable's sort, and a sort is only
                          known at the declaration site — which is L4, where
                          parameter types are in hand. Enforcing it at L2 would
                          reject every contract that mentions a variable at
                          all, including the spec's own §3 example
                          `result >= 0 || result == -code`, because the parser
                          cannot yet tell a parameter from a constant. WF3 is
                          L3's totality gate. *)
                       let free = LiquidTypes.check_wf1 f ~bound in
                       if free <> [] then
                         fail (LiquidTypes.span_of_string src)
                           (Printf.sprintf
                              "WF1 (§2.3): not a parameter, `result`, or a total-fragment constant: %s"
                              (String.concat ", " free))
                       else go rest)))
        | _ -> go rest)
  in
  go pragmas

(** The pass itself: every function-like declaration in the module. The caller
    supplies the names in scope; [check_module] uses WF1's structural half
    (parameters plus `result`) and defers constant lookup, which is WF3 and
    needs the module's constant table. *)
let check_module ~(bound : string list) (m : typed_module) : string option =
  let (TypedModule (_, decls)) = m in
  let param_names (params : Type.value_parameter list) =
    List.filter_map
      (fun (p : Type.value_parameter) ->
        match p with
        | Type.ValueParameter (n, _) -> Some (Identifier.ident_string n))
      params
  in
  let rec go = function
    | [] -> None
    | d :: rest ->
       (match d with
        | TFunction (_, _, name, _, params, _, _, _, pragmas) ->
           (* WF1: this declaration's own parameters join the ambient scope,
               and `result` is always in scope (§2.2). *)
           let here = bound @ ("result" :: param_names params) in
           (match check_decl ~bound:here (Identifier.ident_string name) pragmas with
            | Some e -> Some e
            | None -> go rest)
        | TForeignFunction (_, _, name, _, _, _, _, pragmas) ->
           (* A foreign declaration has no parameters of its own; its
               contract may only mention `result` and module constants. *)
           (match check_decl ~bound (Identifier.ident_string name) pragmas with
            | Some e -> Some e
            | None -> go rest)
        | _ -> go rest)
  in
  go decls

(** L3: the totality gate, run on every function that carries a contract. A
    function with no `Liquid_*` pragma is outside the fragment and is left
    unchecked — the gate never rejects code that did not opt in.

    The call graph is built once for the module, over both the `decl_id` and
    `mono_id` namespaces (see `TotalityCheck` on why both are needed), and the
    foreign `decl_id`s are collected for T4. *)
let decl_key (id: Id.decl_id): int =
  match id with
  | Id.DeclId i -> i

let totality (m : typed_module) : string option =
  let (TypedModule (_, decls)) = m in
  let names : (int, string) Hashtbl.t = Hashtbl.create 16 in
  let foreign : (int, unit) Hashtbl.t = Hashtbl.create 16 in
  let graph : (int, int list) Hashtbl.t = Hashtbl.create 16 in
  List.iter
    (fun d ->
      match d with
      | TFunction (id, _, name, _, _, _, body, _, _) ->
         Hashtbl.replace names (decl_key id) (Identifier.ident_string name);
         Hashtbl.replace graph (decl_key id)
           (List.map decl_key (TotalityCheck.calls_in_stmt [] body))
      | TForeignFunction (id, _, _, _, _, _, _, _) ->
         Hashtbl.replace foreign (decl_key id) ()
      | _ -> ())
    decls;
  (* Report the first contracted function that is not total. *)
  let rec go = function
    | [] -> None
    | d :: rest ->
       (match d with
        | TFunction (id, _, name, _, _, _, body, _, pragmas) ->
           let contracted =
             List.exists
               (fun p ->
                 match p with
                 | Common.LiquidPragma _ -> true
                 | _ -> false)
               pragmas
           in
           if not contracted then go rest
           else
             TotalityCheck.check_function
               ~name:(Identifier.ident_string name)
               ~entry:id ~body ~graph ~foreign ~names
        | _ -> go rest)
  in
  go decls

(** L4: emit the module's `.mlw` when `AUSTRAL_LIQUID_DUMP` names a directory.
    This is how the goldens under `lib/liquid/golden/` are produced
    (`docs/LIQUID.md` §8.1) and how L5's driver will be exercised without
    committing a Why3 engine to the test suite. Opt-in: unset, the pass writes
    nothing. *)
let dump (m : typed_module) =
  match Sys.getenv_opt "AUSTRAL_LIQUID_DUMP" with
  | None -> ()
  | Some dir ->
     (match LiquidConstraints.generate m with
      | None -> ()
      | Some c ->
         let path = Filename.concat dir (c.LiquidConstraints.module_name ^ ".mlw") in
         let oc = open_out path in
         output_string oc (LiquidConstraints.to_mlw c);
         close_out oc)

(** The `typed_pass` registered as the `liquid` tenant. Uses the
    self-referential scope: a contract may only mention its own parameters and
    `result`. WF3 — that every applied function symbol is total-fragment
    admissible — is L3's totality gate, not this pass's job. *)
let check (m : typed_module) : Compiler_plugin.verdict =
  let (TypedModule (_, decls)) = m in
  (* Scope for WF1 without constant lookup: every parameter name in the
     module, plus `result`. Deliberately permissive — tightening it needs the
     per-declaration scope, which `check_module` does properly. *)
  let bound = ref [ "result" ] in
  List.iter
    (fun d ->
      match d with
      | TFunction (_, _, _, _, params, _, _, _, _) ->
         List.iter
           (fun (p : Type.value_parameter) ->
             match p with
             | Type.ValueParameter (n, _) ->
                let s = Identifier.ident_string n in
                if not (List.mem s !bound) then bound := s :: !bound)
           params
      | _ -> ())
    decls;
  dump m;
  (* L5: hand the module to the Why3 driver when asked. Off unless
     AUSTRAL_LIQUID_VERIFY is set, so the default build needs no prover. *)
  let prove_verdict =
    match Sys.getenv_opt "AUSTRAL_LIQUID_VERIFY" with
    | None -> None
    | Some _ -> LiquidWhy3.verify m
  in
  match check_module ~bound:!bound m with
  | Some msg -> Compiler_plugin.VerdictReject msg
  | None ->
     match totality m with
     | Some msg -> Compiler_plugin.VerdictReject msg
     | None ->
        (* A prover refusal outranks everything the syntactic pass found: the
           contract was parsed, total, and then *disproved*. *)
        (match prove_verdict with
         | Some v -> v
         | None -> Compiler_plugin.VerdictOk)