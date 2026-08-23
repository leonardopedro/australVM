(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   S36: the WhyML-derived compiler plugin. `Authorize_gate` is the Why3
   extraction pinned in `lib/why3_plugin/` (semantics-preserving translation
   of the WhyML program the unfer probability kernel emits via
   `uk_whyml_emit`); these tests corroborate at runtime that the extracted
   functions satisfy the postcondition proved in the `.mlw`, and that the
   compiler-plugin registry + the env-driven gate pass behave as specified.
*)
open OUnit2
open Compiler_plugin
open TestUtil

(* ── the extracted gate satisfies its postcondition ───────────────────── *)

let test_gate_is_the_subset_check _ =
  (* authorize grants required = true  <->  required ⊆ grants *)
  eq true (Authorize_gate.authorize [1; 2; 3] [1; 2]);
  eq true (Authorize_gate.authorize [1; 2; 3] []);
  eq true (Authorize_gate.authorize [] []);
  eq true (Authorize_gate.authorize [1; 2; 3] [3; 1; 2]);
  eq false (Authorize_gate.authorize [1; 2] [1; 3]);
  eq false (Authorize_gate.authorize [] [1]);
  eq false (Authorize_gate.authorize [2] [1; 2]);
  (* gate_verdict: 0 iff the subset holds *)
  eq 0 (Authorize_gate.gate_verdict [1; 2; 3] [2]);
  eq 1 (Authorize_gate.gate_verdict [1; 2] [2; 3]);
  eq 0 (Authorize_gate.gate_verdict [] [])

(* ── the compiler-plugin registry ─────────────────────────────────────── *)

let test_registry_register_run_reset _ =
  reset ();
  eq [] (list_registered ());
  register ~name:"always_ok" (fun ~module_name:_ ~foreign_externals:_ ~constants:_ -> VerdictOk);
  register ~name:"reject_uk_foo" (fun ~module_name ~foreign_externals ~constants:_ ->
    if List.mem "uk_foo" foreign_externals then
      VerdictReject (Printf.sprintf "module %s may not import uk_foo" module_name)
    else VerdictOk);
  eq ["always_ok"; "reject_uk_foo"] (list_registered ());
  eq VerdictOk (run ~module_name:"m" ~foreign_externals:["uk_version"] ~constants:[]);
  (match run ~module_name:"m" ~foreign_externals:["uk_foo"; "uk_version"] ~constants:[] with
   | VerdictReject msg -> assert_bool ("names the module: " ^ msg) (msg = "module m may not import uk_foo")
   | VerdictOk -> assert_failure "uk_foo must be rejected");
  reset ();
  eq [] (list_registered ())

(* ── the WhyML gate pass (env-driven) ─────────────────────────────────── *)

let test_why3_gate_enforces_the_grant_set _ =
  reset ();
  Why3_plugin.install ();
  (* The pure core is exercised with explicit grant sets — no process-env
     mutation (ounit2 flags env changes between tests; OCaml 4.14 has no
     Unix.unsetenv). `Why3_plugin.check` is the env-reading wrapper. *)
  let check ~grants ~ext = Why3_plugin.check_with_grants ~grants ~module_name:"m" ~foreign_externals:ext ~constants:[] in
  (* no grants: no-op *)
  (match check ~grants:[] ~ext:["uk_evolve"] with
   | VerdictOk -> ()
   | VerdictReject _ -> assert_failure "empty grants must be a no-op");
  (* no externals: no-op *)
  (match check ~grants:["uk_version"] ~ext:[] with
   | VerdictOk -> ()
   | VerdictReject _ -> assert_failure "no externals must pass");
  (* grants uk_version only: uk_evolve import is rejected *)
  (match check ~grants:["uk_version"] ~ext:["uk_evolve"; "uk_version"] with
   | VerdictReject msg ->
       assert_bool
         ("names the missing symbol: " ^ msg)
         (try
            let _ = Str.search_forward (Str.regexp_string "uk_evolve") msg 0 in
            true
          with Not_found -> false)
   | VerdictOk -> assert_failure "a non-granted uk_evolve import must be rejected");
  (* uk_version import alone is granted *)
  (match check ~grants:["uk_version"] ~ext:["uk_version"] with
   | VerdictOk -> ()
   | VerdictReject msg -> assert_failure ("granted import rejected: " ^ msg));
  (* a superset grant set admits everything required *)
  (match check ~grants:["uk_evolve"; "uk_version"; "uk_session_fork"] ~ext:["uk_evolve"; "uk_version"] with
   | VerdictOk -> ()
   | VerdictReject msg -> assert_failure ("superset grant rejected: " ^ msg));
  reset ()

(* ── the compiler as a plugin of the application/VM (Vm_plugin) ──────── *)

let test_compiler_plugin_registry _ =
  (* clean slate: drop both the pass registry and the compiler registry *)
  Compiler_plugin.reset ();
  Vm_plugin.reset ();
  eq [] (Vm_plugin.list_compilers ());
  (* a synthetic compiler service registers and is routed through *)
  let seen = ref 0 in
  let fake : Vm_plugin.compiler_service = {
      name = "fake";
      compile = (fun _mods -> incr seen; Compiler.empty_compiler);
    } in
  Vm_plugin.register_compiler fake;
  eq [ "fake" ] (Vm_plugin.list_compilers ());
  let _ = Vm_plugin.run_compiler [] in
  eq 1 !seen;
  (* the last registration wins (stack semantics); list_compilers returns
     registration order *)
  Vm_plugin.register_compiler { fake with name = "fake2" };
  eq [ "fake"; "fake2" ] (Vm_plugin.list_compilers ());
  Vm_plugin.reset ();
  Compiler_plugin.reset ()

let test_compiler_plugin_boot_registers_builtin _ =
  Compiler_plugin.reset ();
  Vm_plugin.reset ();
  (* boot is idempotent *)
  Vm_plugin.boot ();
  Vm_plugin.boot ();
  eq [ "austral-builtin" ] (Vm_plugin.list_compilers ());
  (* routing through the registry compiles a real module with the built-in
     pipeline (which installs the Why3 gate as a pass plugin) *)
  let src : Compiler.module_source = Compiler.BodyModuleSource {
      body_filename = "Probe.aum";
      body_code = "module body Probe is\n" ^
                  "    function main(): Int64 is\n" ^
                  "        return 42;\n" ^
                  "    end;\n" ^
                  "end module body.\n";
    } in
  let _ = Vm_plugin.run_compiler [ src ] in
  (* the built-in compiler installed the Why3 gate as its pass plugin *)
  assert_bool "why3_gate pass installed by the compiler plugin"
    (List.mem "why3_gate" (Compiler_plugin.list_registered ()));
  Vm_plugin.reset ();
  Compiler_plugin.reset ()

let suite =
  "PluginTest" >::: [
      "gate_is_the_subset_check" >:: test_gate_is_the_subset_check;
      "registry_register_run_reset" >:: test_registry_register_run_reset;
      "why3_gate_enforces_the_grant_set" >:: test_why3_gate_enforces_the_grant_set;
      "compiler_plugin_registry" >:: test_compiler_plugin_registry;
      "compiler_plugin_boot_registers_builtin" >:: test_compiler_plugin_boot_registers_builtin;
    ]

let () =
  run_test_tt_main suite
