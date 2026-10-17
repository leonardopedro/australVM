(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(* Unit tests for the avm-pkg JSON derivation export (lib/DerivAst.ml):
   extraction of `Derivation` constants from a typed module, the exact
   plan.json shape avm-eval deserializes, name-sorted deterministic
   output, file merging across modules, and loud rejection of constants
   that do not fit the shape. *)

open OUnit2
open Identifier
open Common
open Id
open Stages.Tast
open Escape
open Error

let sid = make_ident
let anyty = Type.Unit

let str (s: string): texpr = TStringConstant (escape_string s)

let qid name =
  make_qident (make_mod_name "Derivation", sid name, sid name)

(* A `List` value built from Cons/Nil constructors, in source order. *)
let list_of (items: texpr list): texpr =
  List.fold_right
    (fun head tail ->
      TUnionConstructor (anyty, sid "Cons",
                         [(sid "head", head); (sid "tail", tail)]))
    items
    (TUnionConstructor (anyty, sid "Nil", []))

let pair (k: string) (v: string): texpr =
  TRecordConstructor (anyty, [(sid "first", str k); (sid "second", str v)])

let derivation_ty =
  Type.NamedType (qid "Derivation", [], Type.FreeUniverse)

let drv
    (name: string)
    ~(version: string)
    ~(deps: string list)
    ~(sources: string list)
    ~(env: (string * string) list)
    ~(builder: string)
  : typed_decl =
  TConstant (
      fresh_decl_id (), VisPrivate, sid name, derivation_ty,
      TRecordConstructor (anyty, [
                             (sid "name", str name);
                             (sid "version", str version);
                             (sid "sources", list_of (List.map str sources));
                             (sid "dependencies", list_of (List.map str deps));
                             (sid "env_vars", list_of (List.map (fun (k, v) -> pair k v) env));
                             (sid "builder_symbol", str builder);
                           ]),
      Docstring "")

let one_module (decls: typed_decl list): typed_module =
  TypedModule (make_mod_name "Recipes", decls)

let json_equal (expected: string) (actual: string): unit =
  assert_equal
    ~printer:Yojson.Safe.to_string
    (Yojson.Safe.from_string expected)
    (Yojson.Safe.from_string actual)

(* ── extraction + shape ────────────────────────────────────────────── *)

let test_extraction_shape _ =
  let d =
    drv "hello"
      ~version:"1.0.0"
      ~deps:["musl"]
      ~sources:["aa"; "bb"]
      ~env:[("CC", "tcc"); ("CFLAGS", "-O2")]
      ~builder:"stdenv.build"
  in
  let ds = DerivAst.module_derivations (one_module [d]) in
  assert_equal ~printer:string_of_int 1 (List.length ds);
  let json = DerivAst.to_json ds in
  json_equal
    {|{"derivations":[
        {"name":"hello","version":"1.0.0","builder_symbol":"stdenv.build",
         "dependencies":["musl"],"sources":["aa","bb"],
         "env_vars":[["CC","tcc"],["CFLAGS","-O2"]],
         "totality":"unproven"}
      ]}|}
    json

let test_empty_lists _ =
  let d =
    drv "base" ~version:"1" ~deps:[] ~sources:[] ~env:[] ~builder:"stdenv.build"
  in
  let ds = DerivAst.module_derivations (one_module [d]) in
  json_equal
    {|{"derivations":[
        {"name":"base","version":"1","builder_symbol":"stdenv.build",
         "dependencies":[],"sources":[],"env_vars":[],
         "totality":"unproven"}
      ]}|}
    (DerivAst.to_json ds)

(* ── determinism: output is name-sorted regardless of declaration order ─ *)

let test_sorted_by_name _ =
  let zebra = drv "zebra" ~version:"1" ~deps:[] ~sources:[] ~env:[] ~builder:"b"
  and alpha = drv "alpha" ~version:"1" ~deps:[] ~sources:[] ~env:[] ~builder:"b" in
  (* Declared zebra first; the export must still emit alpha first. *)
  let ds = DerivAst.module_derivations (one_module [zebra; alpha]) in
  json_equal
    {|{"derivations":[
        {"name":"alpha","version":"1","builder_symbol":"b",
         "dependencies":[],"sources":[],"env_vars":[],
         "totality":"unproven"},
        {"name":"zebra","version":"1","builder_symbol":"b",
         "dependencies":[],"sources":[],"env_vars":[],
         "totality":"unproven"}
      ]}|}
    (DerivAst.to_json ds)

(* ── non-derivation constants are ignored ──────────────────────────── *)

let test_plain_constants_ignored _ =
  let c =
    TConstant (fresh_decl_id (), VisPrivate, sid "answer",
               Type.Integer (Type.Signed, Type.Width64), TIntConstant "42", Docstring "")
  in
  assert_equal [] (DerivAst.module_derivations (one_module [c]))

(* ── malformed derivations fail loudly ─────────────────────────────── *)

let expect_rejection (m: typed_module) (what: string): unit =
  try
    ignore (DerivAst.module_derivations m);
    assert_failure (what ^ " must be rejected by the exporter")
  with
  | Austral_error _ ->
     ()

let test_missing_field_rejected _ =
  (* A Derivation constructor that forgets `sources` must not be dropped
     silently — the plan would silently lose inputs. *)
  let bad =
    TConstant (
        fresh_decl_id (), VisPrivate, sid "broken", derivation_ty,
        TRecordConstructor (anyty, [
                               (sid "name", str "broken");
                               (sid "version", str "1");
                               (* no sources, no dependencies, no env_vars,
                                  no builder_symbol *)
                             ]),
        Docstring "")
  in
  expect_rejection (one_module [bad]) "a Derivation missing required fields"

let test_non_string_field_rejected _ =
  let bad =
    TConstant (
        fresh_decl_id (), VisPrivate, sid "numver", derivation_ty,
        TRecordConstructor (anyty, [
                               (sid "name", str "numver");
                               (sid "version", TIntConstant "1");
                               (sid "sources", list_of []);
                               (sid "dependencies", list_of []);
                               (sid "env_vars", list_of []);
                               (sid "builder_symbol", str "stdenv.build");
                             ]),
        Docstring "")
  in
  expect_rejection (one_module [bad]) "a non-string Derivation field"

(* ── file merging across modules ───────────────────────────────────── *)

let test_write_json_merges _ =
  let path = Filename.temp_file "avm-plan" ".json" in
  let a1 = drv "app" ~version:"1.0.0" ~deps:["musl"] ~sources:[] ~env:[] ~builder:"b"
  and musl = drv "musl" ~version:"1.2" ~deps:[] ~sources:["aa"] ~env:[] ~builder:"b" in
  DerivAst.write_json path (DerivAst.module_derivations (one_module [a1]));
  (* A second module contributes musl and a *revised* app. *)
  let a2 = drv "app" ~version:"2.0.0" ~deps:["musl"] ~sources:[] ~env:[] ~builder:"b" in
  DerivAst.write_json path (DerivAst.module_derivations (one_module [musl; a2]));
  let merged = Yojson.Safe.from_file path in
  let items =
    Yojson.Safe.Util.(member "derivations" merged |> to_list)
  in
  assert_equal ~printer:string_of_int 2 (List.length items);
  let names = List.map (fun j -> Yojson.Safe.Util.(member "name" j |> to_string)) items in
  assert_equal ["app"; "musl"] names;
  let app = List.hd items in
  assert_equal "2.0.0" Yojson.Safe.Util.(member "version" app |> to_string)
    ~msg:"later declaration of the same name must win the merge";
  Sys.remove path

(* ── totality status: the plan records the --pkg-totality=true selection *)

let plan_field (path: string) (name: string) (field: string): string =
  let j = Yojson.Safe.from_file path in
  let items = Yojson.Safe.Util.(member "derivations" j |> to_list) in
  let d =
    List.find
      (fun d -> Yojson.Safe.Util.(member "name" d |> to_string) = name)
      items
  in
  Yojson.Safe.Util.(member field d |> to_string)

let test_totality_status_flagged _ =
  (* Compiling under --pkg-totality=true selects the declarations: the
     derived plan records them as proven. *)
  let d = drv "app" ~version:"1.0.0" ~deps:[] ~sources:[] ~env:[] ~builder:"b" in
  let ds = DerivAst.module_derivations ~totality:true (one_module [d]) in
  json_equal
    {|{"derivations":[
        {"name":"app","version":"1.0.0","builder_symbol":"b",
         "dependencies":[],"sources":[],"env_vars":[],
         "totality":"proven"}
      ]}|}
    (DerivAst.to_json ds)

let test_totality_status_merge _ =
  let path = Filename.temp_file "avm-plan-totality" ".json" in
  let a1 = drv "app" ~version:"1.0.0" ~deps:[] ~sources:[] ~env:[] ~builder:"b"
  and a2 = drv "app" ~version:"2.0.0" ~deps:[] ~sources:[] ~env:[] ~builder:"b" in
  DerivAst.write_json path (DerivAst.module_derivations ~totality:true (one_module [a1]));
  assert_equal "proven" (plan_field path "app" "totality");
  (* A later declaration of the same name without the flag wins the
     merge: the re-selected status is unproven (last write, deterministically). *)
  DerivAst.write_json path (DerivAst.module_derivations (one_module [a2]));
  assert_equal "unproven" (plan_field path "app" "totality")
    ~msg:"re-declaring a derivation without the flag must downgrade it to unproven";
  Sys.remove path

let test_v0_plan_decodes_unproven _ =
  (* A plan written before the totality field existed: absence of the
     record must decode as unproven, never as proven, and must not fail. *)
  let path = Filename.temp_file "avm-plan-v0" ".json" in
  let oc = open_out path in
  output_string oc
    {|{"derivations":[{"name":"old","version":"1","builder_symbol":"b",
        "dependencies":[],"sources":[],"env_vars":[]}]}|};
  close_out oc;
  let added = drv "new" ~version:"1" ~deps:[] ~sources:[] ~env:[] ~builder:"b" in
  DerivAst.write_json path (DerivAst.module_derivations (one_module [added]));
  assert_equal "unproven" (plan_field path "old" "totality");
  assert_equal "unproven" (plan_field path "new" "totality");
  Sys.remove path

let test_unknown_totality_rejected _ =
  (* An unknown status string must fail loudly, like any other
     mis-shaped plan data — never be dropped or guessed. *)
  let path = Filename.temp_file "avm-plan-badtotality" ".json" in
  let oc = open_out path in
  output_string oc
    {|{"derivations":[{"name":"old","version":"1","builder_symbol":"b",
        "dependencies":[],"sources":[],"env_vars":[],"totality":"maybe"}]}|};
  close_out oc;
  (try
     DerivAst.write_json path [];
     assert_failure "an unknown totality status must be rejected"
   with
   | Austral_error _ ->
      ());
  Sys.remove path

let suite =
  "DerivAst" >::: [
      "extraction + plan.json shape" >:: test_extraction_shape;
      "empty lists" >:: test_empty_lists;
      "sorted by name" >:: test_sorted_by_name;
      "plain constants ignored" >:: test_plain_constants_ignored;
      "missing field rejected" >:: test_missing_field_rejected;
      "non-string field rejected" >:: test_non_string_field_rejected;
      "write_json merges across modules" >:: test_write_json_merges;
      "flagged compilation records proven" >:: test_totality_status_flagged;
      "merge downgrade to unproven" >:: test_totality_status_merge;
      "v0 plan decodes unproven" >:: test_v0_plan_decodes_unproven;
      "unknown totality status rejected" >:: test_unknown_totality_rejected;
    ]

let _ = run_test_tt_main suite
