(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(* Unit tests for the avm-pkg package totality pass (lib/Totality.ml).

   The pass runs over the *typed* AST, so the tests hand-build typed
   modules: enough to pin every rule of the v1 checker (while rejection,
   intra-module cycle rejection, foreign/embed/fptr rejection) plus one
   module that must be accepted because every one of its constructs is
   provably bounded. *)

open OUnit2
open Identifier
open Common
open Id
open Stages.Tast
open Error
open TestUtil

let sid = make_ident
let modname = make_mod_name "PkgTest"

let qid name =
  make_qident (modname, sid name, sid name)

let unit_ty = Type.Unit
let bindings = TypeBindings.empty_bindings
let empty_typarams = TypeParameters.empty_typarams

let fn (id: decl_id) (name: string) (body: tstmt): typed_decl =
  TFunction (id, VisPrivate, sid name, empty_typarams, [], unit_ty, body, Docstring "", [])

let call (id: decl_id) (name: string): texpr =
  TFuncall (id, qid name, [], unit_ty, bindings)

let module_of (decls: typed_decl list): typed_module =
  TypedModule (modname, decls)

let expect_rejection (m: typed_module) (what: string): unit =
  try
    Totality.check_module m;
    assert_failure (what ^ " must be rejected by the totality pass")
  with
  | Austral_error _ ->
     ()

(* ── rule 1: while ─────────────────────────────────────────────────── *)

let test_while_rejected _ =
  let id = fresh_decl_id () in
  let body = TWhile (simplespan "w", TBoolConstant true, TSkip (simplespan "s")) in
  expect_rejection (module_of [fn id "build" body]) "a `while` loop"

(* ── rule 2: recursion ─────────────────────────────────────────────── *)

let test_self_recursion_rejected _ =
  let id = fresh_decl_id () in
  let body = TReturn (simplespan "r", call id "count") in
  expect_rejection (module_of [fn id "count" body]) "direct self-recursion"

let test_mutual_recursion_rejected _ =
  let a = fresh_decl_id ()
  and b = fresh_decl_id () in
  let body_a = TReturn (simplespan "a", call b "even") in
  let body_b = TReturn (simplespan "b", call a "odd") in
  expect_rejection
    (module_of [fn a "odd" body_a; fn b "even" body_b])
    "mutual recursion between two functions"

let test_acyclic_call_chain_accepted _ =
  (* main -> helper, helper -> leaf: a DAG, no cycle. *)
  let main_id = fresh_decl_id ()
  and helper_id = fresh_decl_id ()
  and leaf_id = fresh_decl_id () in
  let leaf_body = TReturn (simplespan "l", TIntConstant "1") in
  let helper_body = TBlock (simplespan "b",
                            TLet (simplespan "l", Immutable, sid "x", unit_ty,
                                  TSkip (simplespan "s")),
                            TReturn (simplespan "r", call leaf_id "leaf")) in
  let main_body = TReturn (simplespan "m", call helper_id "helper") in
  (* Must not raise. *)
  Totality.check_module
    (module_of [fn leaf_id "leaf" leaf_body;
                fn helper_id "helper" helper_body;
                fn main_id "main" main_body])

(* ── rule 3: unmediated effects and unprovable targets ─────────────── *)

let test_foreign_function_rejected _ =
  let id = fresh_decl_id () in
  let decl =
    TForeignFunction (id, VisPrivate, sid "puts", [], unit_ty, "puts", Docstring "", [])
  in
  expect_rejection (module_of [decl]) "a foreign function declaration"

let test_foreign_import_pragma_rejected _ =
  let id = fresh_decl_id () in
  let decl =
    TFunction (id, VisPrivate, sid "putchar", empty_typarams, [], unit_ty,
               TReturn (simplespan "r", TIntConstant "0"), Docstring "",
               [ForeignImportPragma "putchar"])
  in
  expect_rejection (module_of [decl]) "a Foreign_Import pragma"

let test_embed_rejected _ =
  let id = fresh_decl_id () in
  let body = TReturn (simplespan "r", TEmbed (unit_ty, "return 0;", [])) in
  expect_rejection (module_of [fn id "build" body]) "an @embed expression"

let test_fptr_call_rejected _ =
  let id = fresh_decl_id () in
  let body = TReturn (simplespan "r", TFptrCall (sid "f", [], unit_ty)) in
  expect_rejection (module_of [fn id "run" body]) "a function-pointer call"

(* ── the accepting side: everything bounded must pass ──────────────── *)

let test_bounded_module_accepted _ =
  let leaf = fresh_decl_id ()
  and main = fresh_decl_id () in
  let leaf_body = TReturn (simplespan "l", TIntConstant "1") in
  (* A `for` loop (bounded by construction) around a call and an `if`. *)
  let loop_body =
    TIf (simplespan "i", TBoolConstant true,
         TDiscarding (simplespan "d", call leaf "leaf"),
         TSkip (simplespan "s"))
  in
  let main_body =
    TBlock (simplespan "b",
            TFor (simplespan "f", sid "i", TIntConstant "0", TIntConstant "10", loop_body),
            TReturn (simplespan "r", TBoolConstant true))
  in
  (* Must not raise. *)
  Totality.check_module
    (module_of [fn leaf "leaf" leaf_body; fn main "main" main_body])

(* ── scope: the pass only fires on the constructs above ────────────── *)

let test_non_package_constructs_untouched _ =
  (* Constants, records, and case statements carry no unbounded construct;
     a module of pure data + a case dispatch must pass. *)
  let id = fresh_decl_id () in
  let body =
    TCase (simplespan "c", TIntConstant "1",
           [TypedWhen (sid "Some", [], TSkip (simplespan "s"))],
           CasePlain)
  in
  Totality.check_module
    (module_of [TConstant (fresh_decl_id (), VisPrivate, sid "answer",
                           Type.Integer (Type.Signed, Type.Width64), TIntConstant "42", Docstring "");
                fn id "pick" body])

let suite =
  "Totality" >::: [
      "while loop rejected" >:: test_while_rejected;
      "self recursion rejected" >:: test_self_recursion_rejected;
      "mutual recursion rejected" >:: test_mutual_recursion_rejected;
      "acyclic call chain accepted" >:: test_acyclic_call_chain_accepted;
      "foreign function rejected" >:: test_foreign_function_rejected;
      "foreign import pragma rejected" >:: test_foreign_import_pragma_rejected;
      "embed rejected" >:: test_embed_rejected;
      "function pointer call rejected" >:: test_fptr_call_rejected;
      "bounded module accepted" >:: test_bounded_module_accepted;
      "pure data + case accepted" >:: test_non_package_constructs_untouched;
    ]

let _ = run_test_tt_main suite
