(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
(** CLI parsing module. *)
open Identifier
open CliUtil

(** Represents a program entrypoint. *)
type entrypoint =
  | Entrypoint of module_name * identifier
[@@deriving eq]

(** Represents the path to a module. *)
type mod_source =
  | ModuleSource of { inter_path: string; body_path: string }
  (** Path to a module's interface and body files. *)
  | ModuleBodySource of { body_path: string }
  (** Path to a module's body file, for public body-only modules. *)
[@@deriving eq]

(** The compiler's target output. *)
type target =
  | TypeCheck
  (** Stop at the type checking stage. *)
  | Executable of { bin_path: string; entrypoint: entrypoint; }
  (** Compile to C, generate an executable. *)
  | CStandalone of { output_path: string; entrypoint: entrypoint option; }
  (** Compile to a standalone C file. *)
  | WgslBundle of {
      output_dir: string;
      entrypoint: entrypoint;
      numeric_profile: Gpu_ir.numeric_profile;
      source_profile: Gpu_subset.source_profile;
      gpu_operation: string option;
    }
  (** Compile the admitted GPU subset to a WGSL bundle directory under an
      explicit numeric profile (`--gpu-numeric-profile`). `--gpu-operation`
      selects a reduction artifact instead of the pointwise one. *)
  | MlirBundle of {
      output_dir: string;
      entrypoint: entrypoint;
      extent: int;
      chip: string;
    }
  (** Publish a versioned MLIR bundle directory: canonical KernelIR, emitted
      module, lowered PTX and the chained digests in `bundle.json`. The static
      extent is explicit (`--gpu-static-elements`, default 65) and the target
      chip is explicit (`--gpu-chip`, default `sm_70`), because the PTX targets
      one virtual ISA. *)
[@@deriving eq]

type error_reporting_mode =
  | ErrorReportPlain
  | ErrorReportJson

(** The type of compiler commands. The compiler's CLI arglist is parsed to an
    instance of this type. *)
type cmd =
  | HelpCommand
  (** Print usage. *)
  | VersionCommand
  (** Print the compiler's version. *)
  | CompileHelp
  (** Print usage of the compile command. *)
  | WholeProgramCompile of {
      modules: mod_source list;
      target: target;
      error_reporting_mode: error_reporting_mode;
      use_cps_jit: bool;
      jit_server: bool;
      allow_all: bool;
      auth_manifest: string option;
      emit_cps_path: string option;
      (** U07: emit the CPS artifact and stop. No bridge initialization, no JIT
          compile and no C output; `--emit-cps` is required. *)
      emit_cps_only: bool;
      (** U11: write the canonical adapter descriptors for the foreign calls
          whose signature needs a generated wrapper. *)
      emit_adapters_path: string option;
      (** Run the package totality pass (avm-pkg: rejects unbounded
          constructs in package modules). *)
      pkg_totality: bool;
      (** D18: require liquid verification; a missing or inconclusive prover is
          a compile error, not a structured skip. *)
      liquid_required: bool;
      (** Export the module's `Derivation` constants as plan JSON to this
          path (avm-pkg), merging with an existing file. *)
      emit_derivations: string option;
      (** Export the linearity ledger — the checker's linear-resource and
          borrow decisions — as JSON to this path (docs/memsafe.md §3). *)
      emit_linearity_ledger: string option;
    }
  (** Whole program compilation using the C backend. *)

(** Parse an argument list into a compiler command. *)
val parse : arglist -> cmd

(** Parse a module source string (e.g. "file.aui,file.aum" or "file.aum")
    into a [mod_source]. *)
val parse_mod_source : string -> mod_source
