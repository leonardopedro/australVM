(* S36: AustralVM as a unified application/VM packaged as a Mirage unikernel.

   This is the `config.ml` manifest, mirroring the mirage-skeleton layout
   (`../mirage-skeleton/tutorial/local-library/config.ml`): the unikernel is a
   single job that boots the AustralVM application, whose compiler is itself a
   plugin of the application (Vm_plugin). The Why3-derived extensions
   (authorize_gate.ml + why3_plugin.ml) are plugins of that compiler — so the
   whole chain (VM -> compiler -> Why3 gate) is one self-contained unikernel
   binary, the "unified application/VM".

   Build with the Mirage toolchain (opam install mirage):
     mirage configure -t unix && make depend && make
     ./dist/unikernel  (or the target-specific binary)

   The unikernel's `start` is the same boot as the CLI (`Cli.main'` calls
   `Vm_plugin.boot ()`): load the plugins, then route compiles through
   `Vm_plugin.run_compiler`. See unikernel.ml.

   The logic is exercised without Mirage by `unikernel/dune` (the plain
   `compiler_vm_test` executable, which links the same modules) — see
   test/PluginTest.ml for the registry-level checks.
*)
open Mirage

let main = main "Unikernel" job ~local_libs:[ "austral_lib" ]
let () = register "australvm-compiler" [ main ]
