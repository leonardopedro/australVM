(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The unified application/VM plugin registry (S36). AustralVM is a single
   application that IS a VM: the compiler pipeline registers itself as a
   plugin of the application (the "compiler" service), and the Why3-derived
   extensions (the authorization gate, `why3_plugin.ml`) register as plugins
   of that compiler. The CLI boot sequence is therefore just "load the
   plugins": `Vm_plugin.boot` installs the built-in compiler (which installs
   the Why3 gate), and every compile request is routed through the registry
   (`run_compiler`) instead of a hard-coded pipeline call.

   This is the self-hosting cycle: the compiler is a plugin of the same
   application that hosts the JIT'd modules it produces — the compiler runs
   inside the VM that runs the code the compiler compiles. See
   `unfer/docs/WHYML_CYCLE.md` and the `unikernel/` packaging scaffold.
*)

(** A registered compiler service: the full pipeline as a plugin. *)
type compiler_service = {
  name : string;
  compile : Compiler.module_source list -> Compiler.compiler;
}

(** Register a compiler plugin. The last registration wins; the built-in
    compiler is registered by [boot]. *)
val register_compiler : compiler_service -> unit

(** Route a compile request through the registry. Boots if no compiler is
    registered yet (so direct `CliEngine` callers keep working), then runs
    the current compiler plugin. *)
val run_compiler : Compiler.module_source list -> Compiler.compiler

(** Names of the registered compiler plugins, in registration order. *)
val list_compilers : unit -> string list

(** Install the built-in compiler plugin (idempotent). The built-in pipeline
    installs the Why3-derived passes (`Why3_plugin.install`) as part of its
    boot, so the gate is a plugin of the compiler plugin. *)
val boot : unit -> unit

(** Drop every registered compiler plugin (test/QA reset). *)
val reset : unit -> unit
