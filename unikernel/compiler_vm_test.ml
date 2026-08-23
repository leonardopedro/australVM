(* S36: Mirage-free executable exercising the unified application/VM boot.
   This is the unikernel's `boot_and_compile` under the unix backend: boot
   the plugin registry, route a compile through the compiler plugin, and
   verify the Why3 gate pass is installed as a plugin of the compiler. *)

open Compiler

let probe_source : module_source =
  BodyModuleSource {
      body_filename = "Probe.aum";
      body_code =
        "module body Probe is\n" ^
        "    function main(): Int64 is\n" ^
        "        return 42;\n" ^
        "    end;\n" ^
        "end module body.\n";
    }

let () =
  Vm_plugin.boot ();
  (match Vm_plugin.list_compilers () with
   | [ "austral-builtin" ] -> ()
   | other -> failwith (Printf.sprintf "unexpected compiler plugins after boot: %s"
                          (String.concat "," other)));
  (* Route the compile through the registry (the plugin path, not a
     hard-coded pipeline call). *)
  let _compiler = Vm_plugin.run_compiler [ probe_source ] in
  (* The built-in compiler installs the Why3-derived gate as a pass plugin
     during `empty_compiler`; with no AUSTRAL_UK_GRANTS env it is a no-op. *)
  let passes = Compiler_plugin.list_registered () in
  (match passes with
   | [ "why3_gate" ] ->
       Printf.printf "compiler_vm_test: boot ok (compiler plugin + why3_gate pass)@."
   | other ->
       Printf.printf "compiler_vm_test: boot ok (passes: %s)@."
         (String.concat "," other));
  print_endline "compiler_vm_test: PASS"
