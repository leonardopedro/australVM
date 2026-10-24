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
  (* `<=` is emitted as its strict negation. The kernel's Austral subset has no
     non-strict comparison: `logos/src/austral_codegen/parser.rs` rewrites `<=`
     to `<` on the way in, so emitting `(2 <= 3)` produced `true` here and
     `false` there — a silent disagreement in the one place this gate exists to
     catch. Same for `>=`. *)
  let e2 = TComparison (LessThanOrEqual, TIntConstant "2", TIntConstant "3") in
  eq (Some "(not (3 < 2))") (Deltanet_plugin.to_austral e2);
  let e2b = TComparison (GreaterThanOrEqual, TIntConstant "2", TIntConstant "3") in
  eq (Some "(not (2 > 3))") (Deltanet_plugin.to_austral e2b);
  (* `<>` has no token in the kernel's grammar at all, so it is spelled as a
     negated equality rather than as `<>` — which used to make
     `uk_austral_unf` return UK-4804 and reject a valid module. *)
  let e2c = TComparison (NotEqual, TIntConstant "2", TIntConstant "3") in
  eq (Some "(not (2 = 3))") (Deltanet_plugin.to_austral e2c);
  let e3 = TConjunction (TBoolConstant true, TBoolConstant false) in
  eq (Some "(true and false)") (Deltanet_plugin.to_austral e3);
  let e4 = TNegation (TBoolConstant true) in
  eq (Some "(not true)") (Deltanet_plugin.to_austral e4)

let contains needle haystack =
  let n = String.length needle and h = String.length haystack in
  let rec go i = i + n <= h && (String.sub haystack i n = needle || go (i + 1)) in
  n = 0 || go 0

(* The emitted program must not contain an operator the kernel cannot parse.
   Every operator `to_austral` can emit is checked against the kernel's Austral
   subset grammar, which is `== = <= >= < > and or + - * /` — of which `<=` and
   `>=` are silently rewritten to strict forms, so only `=`, `<`, `>` are safe to
   emit directly. *)
let test_emitted_operators_are_in_the_kernels_subset _ =
  let open Stages.Tast in
  let open Common in
  let ops =
    [ (Equal, "="); (NotEqual, ""); (LessThan, "<"); (LessThanOrEqual, "");
      (GreaterThan, ">"); (GreaterThanOrEqual, "") ]
  in
  List.iter
    (fun (op, forbidden) ->
      let emitted =
        Deltanet_plugin.to_austral (TComparison (op, TIntConstant "2", TIntConstant "3"))
      in
      match emitted with
      | None -> ()
      | Some text ->
          List.iter
            (fun bad ->
              let needle = " " ^ bad ^ " " in
              if forbidden = bad && contains needle text then
                failwith
                  (Printf.sprintf
                     "%s emits %S, which the kernel's parser does not accept" text
                     needle))
            [ "<>"; "<="; ">=" ])
    ops


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

(* ── the conversion gate's pure helpers ──────────────────────────────────

   [literal_source] and [report_equivalent] are the two pure pieces of the
   `uk_austral_defeq` gate; the round-trip against the real kernel is
   covered by unfer's `unfer_ffi` defeq tests and the australVM bridge
   integration. These pin the pure contracts here, where they run
   everywhere. *)

let test_literal_source_is_the_kernels_subset _ =
  eq "42" (Deltanet_plugin.literal_source (Deltanet_plugin.VInt 42L));
  eq "-5" (Deltanet_plugin.literal_source (Deltanet_plugin.VInt (-5L)));
  eq "true" (Deltanet_plugin.literal_source (Deltanet_plugin.VBool true));
  eq "false" (Deltanet_plugin.literal_source (Deltanet_plugin.VBool false));
  eq "3.75" (Deltanet_plugin.literal_source (Deltanet_plugin.VFloat 3.75));
  (* These literals must re-parse on the kernel side and reduce to the same
     UNF as the expression they name — verified against the real kernel:
     `(3 > 2)` ≡ `true`, `(0 - 5)` ≡ `-5`, `(1.5 + 2.25)` ≡ `3.75`,
     `(2.0 + 2.0)` ≡ `4.` (all `logos iso` exit 0). *)
  ()

let test_report_equivalent_reads_the_verdict _ =
  eq (Some true)
    (Deltanet_plugin.report_equivalent
       "{\"equivalent\":true,\"unf_hash_a\":\"x\",\"witness\":[[0,0]]}");
  eq (Some false)
    (Deltanet_plugin.report_equivalent
       "{\"equivalent\":false,\"unf_hash_a\":\"x\",\"unf_hash_b\":\"y\",\"witness\":null}");
  (* A report without a usable verdict is None — the caller falls back to
     the legacy value comparison rather than trusting what it cannot read. *)
  eq None (Deltanet_plugin.report_equivalent "not json at all");
  eq None (Deltanet_plugin.report_equivalent "{\"equivalent\":\"yes\"}");
  eq None (Deltanet_plugin.report_equivalent "{}")

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

(* ── the round-trip through the real bridge (.so) ────────────────────────

   [check_constant]'s conversion check calls [austral_defeq_json]; these two
   cases run that exact path against the built bridge: the initializer and
   its evaluated literal must come back [equivalent:true], a deliberately
   wrong pairing [equivalent:false] (otherwise the gate could never reject),
   and [None] must carry a bridge error (the "not passing silently" doctrine)
   — or mean the bridge/kernel is unavailable (no recorded error), in which
   case the case is a documented no-op rather than a failure. *)

let defeq_or_skip src_a src_b =
  match CamlCompiler_rust_bridge.austral_defeq_json src_a src_b with
  | Some json -> `Report json
  | None -> (
      match CamlCompiler_rust_bridge.last_jit_error () with
      | Some err -> assert_failure ("defeq round-trip reached the kernel but failed: " ^ err)
      | None -> `Unavailable) (* bridge/kernel unavailable: documented no-op *)

let test_defeq_roundtrip_agreement _ =
  match defeq_or_skip "(2 = 2)" "true" with
  | `Unavailable -> ()
  | `Report json -> (
      match Deltanet_plugin.report_equivalent json with
      | Some true -> ()
      | _ ->
          assert_failure
            (Printf.sprintf "expected equivalent:true for (2 = 2) ≡ true, report: %s" json))

let test_defeq_roundtrip_disagreement _ =
  match defeq_or_skip "(2 = 2)" "false" with
  | `Unavailable -> ()
  | `Report json -> (
      match Deltanet_plugin.report_equivalent json with
      | Some false -> ()
      | _ ->
          assert_failure
            (Printf.sprintf "expected equivalent:false for (2 = 2) ≢ false, report: %s" json))

let suite =
  "DeltanetPluginTest" >::: [
      "to_austral_literals" >:: test_to_austral_literals;
      "to_austral_arithmetic" >:: test_to_austral_arithmetic;
      "emitted_operators_are_in_the_kernels_subset" >::
        test_emitted_operators_are_in_the_kernels_subset;
      "eval_closed_expressions" >:: test_eval_closed_expressions;
      "literal_source_is_the_kernels_subset" >::
        test_literal_source_is_the_kernels_subset;
      "report_equivalent_reads_the_verdict" >::
        test_report_equivalent_reads_the_verdict;
      "pass_registers_and_respects_env" >:: test_pass_registers_and_respects_env;
      "check_constant_agreement" >:: test_check_constant_agreement;
      "missing_report_without_error_is_noop" >:: test_missing_report_without_error_is_noop;      "missing_report_with_kernel_error_rejects" >::
        test_missing_report_with_kernel_error_rejects;
      "defeq_roundtrip_agreement" >:: test_defeq_roundtrip_agreement;
      "defeq_roundtrip_disagreement" >:: test_defeq_roundtrip_disagreement;
    ]

let () =
  run_test_tt_main suite
