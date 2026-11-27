(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open OUnit2
open Identifier
open CliUtil
open CliParser

let parse_cmd (args: string list): cmd =
  parse (parse_args args)

let test_help_cmd _ =
  let cmd: cmd = parse_cmd ["austral"; "--help"] in
  assert_equal cmd HelpCommand

let test_version_cmd _ =
  let cmd: cmd = parse_cmd ["austral"; "--version"] in
  assert_equal cmd VersionCommand

let test_compile_help_cmd _ =
  let cmd: cmd = parse_cmd ["austral"; "compile"; "--help"] in
  assert_equal cmd CompileHelp

let test_compile_default _ =
  let cmd: cmd = parse_cmd ["austral"; "compile"; "foo.aum"; "bar.aui,bar.aum"; "--entrypoint=Foo:main"; "--output=out"]
  and expected: cmd = WholeProgramCompile {
                          modules = [
                            ModuleBodySource { body_path = "foo.aum" };
                            ModuleSource { inter_path = "bar.aui"; body_path = "bar.aum" };
                          ];
                          target = Executable {
                                       bin_path = "out";
                                       entrypoint = Entrypoint (make_mod_name "Foo", make_ident "main")
                                     };
                            error_reporting_mode = ErrorReportPlain;
                            use_cps_jit = false;
                            jit_server = false;
                            allow_all = false;
                            auth_manifest = None;
                            emit_cps_path = None;
                            emit_cps_only = false;
                            emit_cps_bundle = false;
                            emit_adapters_path = None;
                            pkg_totality = false;
                            liquid_required = false;
                            emit_derivations = None;
                            emit_linearity_ledger = None
                          }
  in
  assert_equal ~printer:(fun _ -> "cmd") cmd expected

(* U17: the bundled compile-only artifact is a parser flag like any other; it
   must not be mistaken for an option value or swallow the source list. *)
let test_compile_emit_cps_bundle _ =
  let cmd: cmd = parse_cmd ["austral"; "compile"; "--use-cps-jit"; "--target-type=tc";
                             "--emit-cps-only"; "--emit-cps-bundle";
                             "--emit-cps=out.cps"; "a.aum"] in
  match cmd with
  | WholeProgramCompile { emit_cps_only; emit_cps_bundle; emit_cps_path; modules; _ } ->
      assert_bool "--emit-cps-only" emit_cps_only;
      assert_bool "--emit-cps-bundle" emit_cps_bundle;
      assert_equal (Some "out.cps") emit_cps_path;
      assert_equal 1 (List.length modules)
  | _ -> assert_failure "expected a whole-program compile"

let suite =
  "CliParser" >::: [
      "--help" >:: test_help_cmd;
      "--version" >:: test_version_cmd;
      "compile --help" >:: test_compile_help_cmd;
      "compile default" >:: test_compile_default;
      "compile --emit-cps-bundle" >:: test_compile_emit_cps_bundle;
    ]

let _ = run_test_tt_main suite
