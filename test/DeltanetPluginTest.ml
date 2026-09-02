(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The Austral→DeltaNets translation as a compiler pass: `Deltanet_plugin`
   recomputes every top-level constant through the kernel's `uk_austral_unf`
   unique-normal-form reducer and rejects the module when the values
   disagree with the compiler's own evaluation. These tests exercise the
   serializer, the independent OCaml evaluator, and the pass verdicts. The
   kernel round-trip (via the rust bridge) is exercised when the bridge is
   available and `UNFER_DELTANET=1`; the pure parts run everywhere.
*)
open OUnit2
open Compiler_plugin
open TestUtil

(* ── the serializer: closed texprs → the Austral subset ──────────────── *)

let test_to_austral_literals _ =
  eq (Some "42") (Deltanet_plugin.to_austral (Stages.Tast.TIntConstant "42"));
  eq (Some "6371.0") (Deltanet_plugin.to_austral (Stages.Tast.TFloatConstant "6371.0"));
  eq (Some "true") (Deltanet_plugin.to_austral (Stages.Tast.TBoolConstant true));
  eq None (Deltanet_plugin.to_austral (Stages.Tast.TStringConstant (Escape.escape_string "x")))

let test_to_austral_arithmetic _ =
  let open Stages.Tast in
  let open Common in
  let e = TComparison (Equal, TIntConstant "2", TIntConstant "3") in
  eq (Some "(2 = 3)") (Deltanet_plugin.to_austral e);
  let e2 = TComparison (LessThanOrEqual, TIntConstant "2", TIntConstant "3") in
  eq (Some "(2 <= 3)") (Deltanet_plugin.to_austral e2);
  let e3 = TConjunction (TBoolConstant true, TBoolConstant false) in
  eq (Some "(true and false)") (Deltanet_plugin.to_austral e3);
  let e4 = TNegation (TBoolConstant true) in
  eq (Some "(not true)") (Deltanet_plugin.to_austral e4)

(* ── the independent OCaml evaluator ─────────────────────────────────── *)

let test_eval_closed_expressions _ =
  let open Stages.Tast in
  let open Common in
  eq (Some (Deltanet_plugin.VInt 42L)) (Deltanet_plugin.eval (TIntConstant "42"));
  eq (Some (Deltanet_plugin.VFloat 6371.0)) (Deltanet_plugin.eval (TFloatConstant "6371.0"));
  eq (Some (Deltanet_plugin.VBool true)) (Deltanet_plugin.eval (TBoolConstant true));
  let e = TComparison (GreaterThan, TIntConstant "5", TIntConstant "2") in
  eq (Some (Deltanet_plugin.VBool true)) (Deltanet_plugin.eval e);
  let e2 = TComparison (Equal, TIntConstant "2", TIntConstant "3") in
  eq (Some (Deltanet_plugin.VBool false)) (Deltanet_plugin.eval e2);
  let e3 = TConjunction (TBoolConstant true, TBoolConstant false) in
  eq (Some (Deltanet_plugin.VBool false)) (Deltanet_plugin.eval e3);
  let e4 = TNegation (TBoolConstant false) in
  eq (Some (Deltanet_plugin.VBool true)) (Deltanet_plugin.eval e4)

(* ── the pass: registry, enablement, per-constant verdicts ───────────── *)

let test_pass_registers_and_respects_env _ =
  Compiler_plugin.reset ();
  Deltanet_plugin.install ();
  Deltanet_plugin.install (); (* idempotent *)
  assert_bool "deltanet_unf pass installed"
    (List.mem "deltanet_unf" (list_registered ()));
  (* disabled by default (no UNFER_DELTANET=1): the gate is a no-op *)
  let c = Deltanet_plugin.check ~module_name:"m" ~foreign_externals:[] ~constants:[] in
  eq VerdictOk c;
  Compiler_plugin.reset ()

let test_check_constant_agreement _ =
  Compiler_plugin.reset ();
  Deltanet_plugin.install ();
  (* A closed arithmetic constant whose kernel reduction agrees with the
     compiler's evaluation passes when enabled. The kernel round-trip needs
     the rust bridge + a model; when unavailable the pass is a no-op
     (VerdictOk) rather than a false rejection. *)
  let init = Stages.Tast.TComparison (Common.Equal, Stages.Tast.TIntConstant "2", Stages.Tast.TIntConstant "3") in
  let name = Identifier.make_ident "x" in
  (match Deltanet_plugin.check_constant ~module_name:"m" (name, init) with
   | VerdictOk -> ()
   | VerdictReject msg -> assert_failure ("agreement must not reject: " ^ msg));
  Compiler_plugin.reset ()

(* When the kernel returns no report, the gate must distinguish "kernel
   unavailable" (documented no-op) from "kernel reached but errored" (a
   real failure recorded on the bridge error channel — a silent pass would
   hide the exact consistency problem this gate exists to catch). *)

let test_missing_report_without_error_is_noop _ =
  let name = Identifier.make_ident "x" in
  eq VerdictOk
    (Deltanet_plugin.verdict_on_missing_report ~module_name:"m" name None)

let test_missing_report_with_kernel_error_rejects _ =
  let name = Identifier.make_ident "c" in
  match
    Deltanet_plugin.verdict_on_missing_report ~module_name:"m" name
      (Some "uk_austral_unf failed with -5000")
  with
  | VerdictOk -> assert_failure "kernel error must not pass silently"
  | VerdictReject msg ->
      let contains sub =
        try
          ignore (Str.search_forward (Str.regexp_string sub) msg 0);
          true
        with Not_found -> false in
      assert_bool "rejection names the module" (contains "module m");
      assert_bool "rejection names the constant" (contains "constant `c`");
      assert_bool "rejection carries the kernel error"
        (contains "uk_austral_unf failed with -5000")

let suite =
  "DeltanetPluginTest" >::: [
      "to_austral_literals" >:: test_to_austral_literals;
      "to_austral_arithmetic" >:: test_to_austral_arithmetic;
      "eval_closed_expressions" >:: test_eval_closed_expressions;
      "pass_registers_and_respects_env" >:: test_pass_registers_and_respects_env;
      "check_constant_agreement" >:: test_check_constant_agreement;
      "missing_report_without_error_is_noop" >:: test_missing_report_without_error_is_noop;
      "missing_report_with_kernel_error_rejects" >:: test_missing_report_with_kernel_error_rejects;
    ]

let () =
  run_test_tt_main suite
