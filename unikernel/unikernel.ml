(* S36: the AustralVM unified application/VM as a unikernel job.

   `start` is what Mirage invokes when the unikernel boots. It mirrors the
   CLI boot (`Cli.main'`): `Vm_plugin.boot` registers the built-in compiler
   as a plugin of the application (which installs the Why3-derived gate as a
   plugin of the compiler), and the compile request is routed through the
   plugin registry — `Vm_plugin.run_compiler` — instead of a hard-coded
   pipeline call. The compiler therefore runs inside the same application
   that hosts the JIT'd modules it produces.

   A tiny probe module is compiled end-to-end through the registry, proving
   the plugin path (and, transitively, the Why3 gate's no-op path when no
   grant env is set) works inside the unikernel. *)

open Compiler

(** The probe module: a trivial Austral body. Compiled through the registry
    to exercise the full pipeline (parse -> typecheck -> plugin pass
    registry -> CPS -> codegen). *)
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

(** Boot the application and compile the probe through the compiler plugin.
    Returns the number of compiler plugins registered after boot (>= 1) and
    the names of the Why3-derived pass plugins installed by the compiler
    (the `why3_gate` pass when `empty_compiler` ran it). *)
let boot_and_compile () : int * string list =
  Vm_plugin.boot ();
  let _compiler = Vm_plugin.run_compiler [ probe_source ] in
  (List.length (Vm_plugin.list_compilers ()), Compiler_plugin.list_registered ())

(** Mirage entrypoint. `start` is called by the unikernel runtime. *)
let start () =
  let (n_compilers, passes) = boot_and_compile () in
  Logs.info (fun f ->
      f "australvm unikernel: booted with %d compiler plugin(s); why3 passes: %s"
        n_compilers
        (String.concat "," passes));
  Lwt.return_unit
