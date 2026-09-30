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

(* ── the NPU DMA gate pass (GPU.md, WhyML-verified SRAM invariant) ──── *)

let test_npu_dma_gate_is_the_sram_check _ =
  (* dma_verdict offset bytes = 0  <->  offset + bytes <= MAX_NPU_SRAM *)
  eq 0 (Npu_dma_gate.dma_verdict 0 0);
  eq 0 (Npu_dma_gate.dma_verdict 0 262144);
  eq 0 (Npu_dma_gate.dma_verdict 100000 162144);
  eq 1 (Npu_dma_gate.dma_verdict 0 262145);
  eq 1 (Npu_dma_gate.dma_verdict 262144 1);
  eq 1 (Npu_dma_gate.dma_verdict 200000 100000);
  (* dma_ok mirrors the same bound on the (size, offset) buffer record *)
  eq true (Npu_dma_gate.dma_ok (262144, 0) 262144);
  eq false (Npu_dma_gate.dma_ok (262144, 0) 262145)

let test_npu_dma_gate_enforces_the_sram_bound _ =
  reset ();
  Npu_dma_plugin.install ();
  let dma = Identifier.make_ident in
  let check ~constants = Npu_dma_plugin.check ~module_name:"m" ~foreign_externals:[] ~constants in
  (* no dma_* constants: no-op *)
  (match check ~constants:[] with
   | VerdictOk -> ()
   | VerdictReject _ -> assert_failure "no DMA constants must pass");
  (* an unrelated constant is not a DMA transfer *)
  (match check ~constants:[(dma "ordinary", TIntConstant "1024")] with
   | VerdictOk -> ()
   | VerdictReject msg -> assert_failure ("unrelated constants must pass: " ^ msg));
  (* in-bounds transfer: pass *)
  (match check ~constants:[(dma "dma_embed_bytes", TIntConstant "65536")] with
   | VerdictOk -> ()
   | VerdictReject msg -> assert_failure ("in-bounds transfer rejected: " ^ msg));
  (* explicit offset, still in bounds: pass *)
  (match
     check
       ~constants:
         [ (dma "dma_embed_offset", TIntConstant "100000");
           (dma "dma_embed_bytes", TIntConstant "100000") ]
   with
   | VerdictOk -> ()
   | VerdictReject msg -> assert_failure ("in-bounds offset transfer rejected: " ^ msg));
  (* over-limit transfer: rejected, naming the constant and the limit *)
  (match check ~constants:[(dma "dma_embed_bytes", TIntConstant "262145")] with
   | VerdictReject msg ->
       assert_bool
         ("names the transfer and the limit: " ^ msg)
         (try
            let _ = Str.search_forward (Str.regexp_string "dma_embed") msg 0 in
            let _ = Str.search_forward (Str.regexp_string "262144") msg 0 in
            true
          with Not_found -> false)
   | VerdictOk -> assert_failure "an over-limit DMA transfer must be rejected");
  (* over-limit via offset + bytes: rejected *)
  (match
     check
       ~constants:
         [ (dma "dma_embed_offset", TIntConstant "200000");
           (dma "dma_embed_bytes", TIntConstant "100000") ]
   with
   | VerdictReject _ -> ()
   | VerdictOk -> assert_failure "offset+bytes overflow must be rejected");
  (* malformed negative size: rejected *)
  (match check ~constants:[(dma "dma_embed_bytes", TIntConstant "-1")] with
   | VerdictReject _ -> ()
   | VerdictOk -> assert_failure "a negative DMA size must be rejected");
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
  (* the built-in compiler installed the Why3 gates as its pass plugins *)
  assert_bool "why3_gate pass installed by the compiler plugin"
    (List.mem "why3_gate" (Compiler_plugin.list_registered ()));
  assert_bool "npu_dma_gate pass installed by the compiler plugin"
    (List.mem "npu_dma_gate" (Compiler_plugin.list_registered ()));
  Vm_plugin.reset ();
  Compiler_plugin.reset ()

let suite =
  "PluginTest" >::: [
      "gate_is_the_subset_check" >:: test_gate_is_the_subset_check;
      "registry_register_run_reset" >:: test_registry_register_run_reset;
      "why3_gate_enforces_the_grant_set" >:: test_why3_gate_enforces_the_grant_set;
      "npu_dma_gate_is_the_sram_check" >:: test_npu_dma_gate_is_the_sram_check;
      "npu_dma_gate_enforces_the_sram_bound" >:: test_npu_dma_gate_enforces_the_sram_bound;
      "compiler_plugin_registry" >:: test_compiler_plugin_registry;
      "compiler_plugin_boot_registers_builtin" >:: test_compiler_plugin_boot_registers_builtin;
    ]

let () =
  run_test_tt_main suite
