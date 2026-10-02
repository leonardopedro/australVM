(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   See Vm_plugin.mli for the design. The registry is a stack of compiler
   services; `run_compiler` invokes the most recently registered one. The
   built-in compiler registers itself at boot, so the application/VM is
   defined by its plugins, not by hard-coded pipeline calls.
*)

open Compiler

type compiler_service = {
  name : string;
  compile : module_source list -> Compiler.compiler;
}

let registry : compiler_service list ref = ref []

let register_compiler (svc : compiler_service) =
  registry := svc :: !registry

let list_compilers () =
  List.rev_map (fun s -> s.name) !registry

let boot () =
  (* Idempotent: only register the built-in compiler once. The built-in
     pipeline installs the Why3-derived passes (`Why3_plugin.install`) and
     the Austral->deltanet UNF gate (`Deltanet_plugin.install`) as part of
     `Compiler.empty_compiler`; we install them explicitly here too because
     `empty_compiler` is a pre-evaluated value, so its side effect only
     happens at module load — re-booting after a `Compiler_plugin.reset`
     must re-install the passes. The gates are therefore plugins of the
     compiler plugin, restored on every boot. *)
  Why3_plugin.install ();
  Deltanet_plugin.install ();
  Npu_dma_plugin.install ();
  (* Part 2 P5 of the unfer rewrite plan: the CNL formalization gate. Same
     shape as the deltanet gate — opt-in via `UNFER_LOGOS=1`, no-op when the
     `logos` binary is absent — but it checks *identity* rather than a value:
     a `cnl_` string constant must reduce to a unique normal form, and two of
     them must not share one. See lib/formalize_plugin.ml for why identity is
     the right thing to check when there is no second opinion to disagree with. *)
  Formalize_plugin.install ();
  (* PLAN_liquid_types.md L2: the liquid contract pass is a tenant of this
     seam. L0 fixed the representation and L1 threaded the pragmas to the
     typed AST; L2 makes the pass real — it parses every contract and checks
     the parts of docs/LIQUID.md §2.2-§2.3 that are already decidable (the
     literal range, the no-quantifiers restriction, WF1, and the declared-sort
     half of WF2). It still *enforces* nothing: L4 generates constraints, L5
     discharges them via Why3, L6 infers qualifiers. *)
  Compiler_plugin.register_typed ~name:"liquid" LiquidCheck.check;
  (* L10 Cycle B: the checker extended by code *extracted* from the total subset.
     `LiquidSubset.check` consults `LiquidMeasure`, which is Why3 output whose
     decreasingness obligation is proved, rather than a hand-written rank that
     would only be asserted. Registered beside `liquid` on the same seam; the two
     tenant names are distinct so either can be enabled alone. *)
  Compiler_plugin.register_typed ~name:"liquid_measure" LiquidSubset.check;
  let names = list_compilers () in
  if not (List.mem "austral-builtin" names) then
    register_compiler
      {
        name = "austral-builtin";
        compile = (fun mods -> compile_multiple empty_compiler mods);
      }

let run_compiler (mods : module_source list) : Compiler.compiler =
  (match !registry with
   | [] -> boot ()
   | _ -> ());
  match !registry with
  | svc :: _ -> svc.compile mods
  | [] -> failwith "Vm_plugin.run_compiler: no compiler registered after boot"

let reset () =
  registry := []
