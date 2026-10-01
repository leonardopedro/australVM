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
  (* PLAN_liquid_types.md L1: the liquid contract pass is a tenant of this
     seam. L0 only fixed the representation — the `Liquid_*` pragmas now parse
     and reach the typed AST — but there is nothing to enforce until L4/L5/L6
     generate and discharge constraints. So the pass is installed as a no-op
     that accepts every module; L1's acceptance test registers its own
     rejecting pass to prove the seam works, and `boot` is where the real
     checker will replace this. *)
  Compiler_plugin.register_typed ~name:"liquid" (fun _ -> Compiler_plugin.VerdictOk);
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
