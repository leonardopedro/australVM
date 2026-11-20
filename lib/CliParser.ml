(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open Identifier
open CliUtil
open Error

module Errors = struct
  let invalid_entrypoint entry =
    austral_raise CliError [
      Text "Invalid entrypoint format ";
      Code entry;
      Break;
      Text "The entrypoint must be supplied in the form ";
      Code "Module:name"
    ]

  let invalid_module_source source =
    austral_raise CliError [
      Text "Invalid module source format ";
      Code source;
      Break;
      Text "Sources must be supplied as either";
      Code "interface.aui,body.aum";
      Text " or ";
      Code "body.aum"
    ]

  let missing_entrypoint () =
    austral_raise CliError [
      Code "--entrypoint";
      Text " argument not provided."
    ]

  let missing_module () =
    austral_raise CliError [
      Text "The ";
      Code "compile";
      Text " command must specify at least one module."
    ]

  let missing_output () =
    austral_raise CliError [
      Code "--output";
      Text " argument not provided."
    ]

  let no_entrypoint_wrong_target () =
    austral_raise CliError [
      Code "--no-entrypoint";
      Text " requires ";
      Code "--target-type=c";
      Text ", because otherwise the compiler will try to build the generated C code, and will fail because there is no entrypoint function."
    ]

  let unknown_target target =
    austral_raise CliError [
      Text "Unknown target type ";
      Code target;
      Text ". Valid target types are ";
      Code "exe";
      Text ", ";
      Code "tc";
      Text ", ";
      Code "c";
      Text ", and ";
      Code "wgsl";
      Text "."
    ]

  let no_entrypoint_wrong_gpu_target () =
    austral_raise CliError [
      Code "--no-entrypoint";
      Text " is incompatible with `--target-type=wgsl`: the GPU target compiles ";
      Text "one selected entry function and cannot publish a bundle without it."
    ]

  let unknown_numeric_profile (profile: string) =
    austral_raise CliError [
      Text "Unknown GPU numeric profile ";
      Code profile;
      Text ". Valid profiles are ";
      Code "u32-modular-v1";
      Text " and ";
      Code "wgsl-f32-v1";
      Text "."
    ]

  let numeric_profile_without_gpu () =
    austral_raise CliError [
      Code "--gpu-numeric-profile";
      Text " requires ";
      Code "--target-type=wgsl";
      Text ": the profile selects the GPU kernel's numeric contract."
    ]

  let unknown_source_profile (profile: string) =
    austral_raise CliError [
      Text "Unknown GPU source profile ";
      Code profile;
      Text ". The only opt-in profile is ";
      Code "bounded-fold-v1";
      Text "."
    ]

  let source_profile_without_gpu () =
    austral_raise CliError [
      Code "--gpu-source-profile";
      Text " requires ";
      Code "--target-type=wgsl";
      Text ": the profile selects which GPU source admission the entry gets."
    ]

  let unknown_gpu_operation (operation: string) =
    austral_raise CliError [
      Text "Unknown GPU operation ";
      Code operation;
      Text ". The fixed reductions in v1 are ";
      Code "map-sum-u32";
      Text " and its four-elements-per-lane variant ";
      Code "map-sum-u32-x4";
      Text "."
    ]

  let gpu_operation_without_gpu () =
    austral_raise CliError [
      Code "--gpu-operation";
      Text " requires ";
      Code "--target-type=wgsl";
      Text ": the operation selects the staged reduction the GPU target emits."
    ]

  let invalid_bool_flag_value (name: string) (value: string) =
    austral_raise CliError [
      Text "Invalid value for ";
      Code ("--" ^ name);
      Text ": ";
      Code value;
      Text ". Valid values are ";
      Code "true";
      Text " and ";
      Code "false";
      Text "."
    ]

  let unknown_error_reporting_mode (mode: string) =
    austral_raise CliError [
      Text "Unknown error reporting mode: ";
      Code mode;
      Text ". Valid values are ";
      Code "plain";
      Text " and ";
      Code "json";
      Text "."
    ]
end

type entrypoint =
  | Entrypoint of module_name * identifier
[@@deriving eq]

type mod_source =
  | ModuleSource of { inter_path: string; body_path: string }
  | ModuleBodySource of { body_path: string }
[@@deriving eq]

type target =
  | TypeCheck
  | Executable of { bin_path: string; entrypoint: entrypoint; }
  | CStandalone of { output_path: string; entrypoint: entrypoint option; }
  (** Compile the checked GPU subset to a WGSL bundle directory.
      `--output` is the bundle directory (`kernel.json` + `kernel.wgsl`).
      The numeric profile is explicit (`--gpu-numeric-profile`), defaulting to
      the integer v1 profile; it is never inferred from the source. *)
  | WgslBundle of {
      output_dir: string;
      entrypoint: entrypoint;
      numeric_profile: Gpu_ir.numeric_profile;
      source_profile: Gpu_subset.source_profile;
      (* `None` is the pointwise map wrapper; `Some "map-sum-u32"` selects the
         staged fixed reduction (G22b), which has its own artifact. *)
      gpu_operation: string option;
    }
[@@deriving eq]

type error_reporting_mode =
  | ErrorReportPlain
  | ErrorReportJson
[@@deriving eq]

type cmd =
  | HelpCommand
  | VersionCommand
  | CompileHelp
  | WholeProgramCompile of {
      modules: mod_source list;
      target: target;
      error_reporting_mode: error_reporting_mode;
      use_cps_jit: bool;
      jit_server: bool;
      allow_all: bool;
      auth_manifest: string option;
      emit_cps_path: string option;
      emit_adapters_path: string option;
      pkg_totality: bool;
      emit_derivations: string option;
      (** Export the linearity ledger — the checker's linear-resource and
          borrow decisions — as JSON to this path (docs/memsafe.md §3). *)
      emit_linearity_ledger: string option;
    }

let check_leftovers (arglist: arglist): unit =
  if (arglist_size arglist) > 0 then
    err "There are leftover arguments."
  else
    ()

let parse_mod_source (s: string): mod_source =
  let ss = String.split_on_char ',' s in
  match ss with
  | [path] ->
     ModuleBodySource { body_path = path }
  | [inter_path; body_path] ->
     ModuleSource { inter_path = inter_path; body_path = body_path }
  | _ ->
     Errors.invalid_module_source s

let parse_entrypoint (s: string): entrypoint =
  let ss = String.split_on_char ':' s in
  match ss with
  | [mn; i] ->
     Entrypoint (make_mod_name mn, make_ident i)
  | _ ->
     Errors.invalid_entrypoint s

let parse_executable_target (arglist: arglist): (arglist * target) =
  (* Get the --entrypoint *)
  match pop_value_flag arglist "entrypoint" with
  | Some (arglist, entrypoint) ->
     (match pop_value_flag arglist "output" with
      | Some (arglist, bin_path) ->
         (arglist, Executable { bin_path = bin_path; entrypoint = parse_entrypoint entrypoint })
      | None ->
         Errors.missing_output ())
  | None ->
     (match pop_bool_flag arglist "--no-entrypoint" with
      | Some _ ->
         Errors.no_entrypoint_wrong_target ()
      | None ->
         Errors.missing_entrypoint ())

let get_output (arglist: arglist): (arglist * string) =
  match pop_value_flag arglist "output" with
  | Some (arglist, output_path) ->
     (arglist, output_path)
  | None ->
     Errors.missing_output ()

let parse_c_target (arglist: arglist): (arglist * target) =
  (* Get the --entrypoint *)
  match pop_value_flag arglist "entrypoint" with
  | Some (arglist, entrypoint) ->
     (* An entrypoint was passed in. *)
     let (arglist, output_path) = get_output arglist in
     (arglist, CStandalone { output_path = output_path; entrypoint = Some (parse_entrypoint entrypoint) })
  | None ->
     (* No --entrypoint. Did we get the --no-entrypoint flag? *)
     (match pop_bool_flag arglist "no-entrypoint" with
      | Some arglist ->
         let (arglist, output_path) = get_output arglist in
         (arglist, CStandalone { output_path = output_path; entrypoint = None })
      | None ->
         Errors.missing_entrypoint ())

let parse_numeric_profile (arglist: arglist): (arglist * Gpu_ir.numeric_profile) =
  match pop_value_flag arglist "gpu-numeric-profile" with
  | Some (arglist, value) ->
     (match Gpu_ir.profile_of_name value with
      | profile -> (arglist, profile)
      | exception Gpu_ir.Ir_error _ -> Errors.unknown_numeric_profile value)
  | None -> (arglist, Gpu_ir.U32ModularV1)

let parse_source_profile (arglist: arglist): (arglist * Gpu_subset.source_profile) =
  match pop_value_flag arglist "gpu-source-profile" with
  | Some (arglist, value) ->
     (match Gpu_subset.source_profile_of_string value with
      | profile -> (arglist, profile)
      | exception Gpu_subset.Refused _ -> Errors.unknown_source_profile value)
  | None -> (arglist, Gpu_subset.StrictV1)

let parse_gpu_operation (arglist: arglist): (arglist * string option) =
  match pop_value_flag arglist "gpu-operation" with
  | Some (arglist, value) ->
     (match value with
      | "map-sum-u32" | "map-sum-u32-x4" -> (arglist, Some value)
      | _ -> Errors.unknown_gpu_operation value)
  | None -> (arglist, None)

let parse_wgsl_target (arglist: arglist): (arglist * target) =
  let (arglist, numeric_profile) = parse_numeric_profile arglist in
  let (arglist, source_profile) = parse_source_profile arglist in
  let (arglist, gpu_operation) = parse_gpu_operation arglist in
  match pop_value_flag arglist "entrypoint" with
  | Some (arglist, entrypoint) ->
     let (arglist, output_dir) = get_output arglist in
     (arglist, WgslBundle { output_dir; entrypoint = parse_entrypoint entrypoint;
                            numeric_profile; source_profile; gpu_operation })
  | None ->
     (match pop_bool_flag arglist "no-entrypoint" with
      | Some _ -> Errors.no_entrypoint_wrong_gpu_target ()
      | None -> Errors.missing_entrypoint ())

let parse_target_type (arglist: arglist): (arglist * target) =
  match pop_value_flag arglist "target-type" with
  | Some (arglist, target_value) ->
     (* An explicit target type was passed. *)
     if target_value <> "wgsl" then
       (match pop_value_flag arglist "gpu-numeric-profile" with
        | Some _ -> Errors.numeric_profile_without_gpu ()
        | None -> ());
     if target_value <> "wgsl" then
       (match pop_value_flag arglist "gpu-source-profile" with
        | Some _ -> Errors.source_profile_without_gpu ()
        | None -> ());
     if target_value <> "wgsl" then
       (match pop_value_flag arglist "gpu-operation" with
        | Some _ -> Errors.gpu_operation_without_gpu ()
        | None -> ());
     (match target_value with
      | "exe" ->
         (* Build an executable binary. *)
         parse_executable_target arglist
      | "c" ->
         (* Build a standaloine C file. *)
         parse_c_target arglist
      | "wgsl" ->
         (* Build a WGSL bundle directory. *)
         parse_wgsl_target arglist
      | "tc" ->
         (* Typecheck. *)
         (arglist, TypeCheck)
      | _ ->
         Errors.unknown_target target_value)
  | None ->
     (* The profile only means something for the GPU target. Refuse it here
        instead of leaving it to the generic leftover-argument error. *)
     (match pop_value_flag arglist "gpu-numeric-profile" with
      | Some _ -> Errors.numeric_profile_without_gpu ()
      | None -> ());
     (match pop_value_flag arglist "gpu-source-profile" with
      | Some _ -> Errors.source_profile_without_gpu ()
      | None -> ());
     (match pop_value_flag arglist "gpu-operation" with
      | Some _ -> Errors.gpu_operation_without_gpu ()
      | None -> ());
     (* The default target is to build an executable binary. This means we need
        an entrypoint. *)
     parse_executable_target arglist

let parse_error_reporting_mode (arglist: arglist): (arglist * error_reporting_mode) =
  match pop_value_flag arglist "error-format" with
  | Some (arglist, value) ->
     (* An explicit error reporting mode was passed. *)
     (match value with
      | "plain" ->
         (* Report errors in plain text. *)
         (arglist, ErrorReportPlain)
      | "json" ->
         (* Report errors in JSON. *)
         (arglist, ErrorReportJson)
      | _ ->
         Errors.unknown_error_reporting_mode value)
  | None ->
     (* The default target is plain text errors. *)
     (arglist, ErrorReportPlain)

let parse_compile_command' (arglist: arglist): (arglist * cmd) =
  (* Parse module list *)
  let (arglist, modules): (arglist * string list) = pop_positional arglist in
  let modules: mod_source list = List.map parse_mod_source modules in
  let (arglist, error_reporting_mode) = parse_error_reporting_mode arglist in
  (* Parse --use-cps-jit flag *)
  let (use_cps_jit, arglist) =
    match pop_bool_flag arglist "use-cps-jit" with
    | Some arglist -> (true, arglist)
    | None -> (false, arglist)
  in
  (* Parse --jit-server flag (implies --use-cps-jit) *)
  let (jit_server, arglist) =
    match pop_bool_flag arglist "jit-server" with
    | Some arglist -> (true, arglist)
    | None -> (false, arglist)
  in
  (* Parse --allow-all flag (disables authorization) *)
  let (allow_all, arglist) =
    match pop_bool_flag arglist "allow-all" with
    | Some arglist -> (true, arglist)
    | None -> (false, arglist)
  in
  (* Parse --auth-manifest=<path> flag (loads a module.toml grant set into
     the JIT authorization engine) *)
  let (auth_manifest, arglist) =
    match pop_value_flag arglist "auth-manifest" with
    | Some (arglist, path) -> (Some path, arglist)
    | None -> (None, arglist)
  in
  (* Parse --emit-cps=<path> flag (save CPS binary IR to file) *)
  let (emit_cps_path, arglist) =
    match pop_value_flag arglist "emit-cps" with
    | Some (arglist, path) -> (Some path, arglist)
    | None -> (None, arglist)
  in
  (* Parse --emit-adapters=<path> flag (U11: write the canonical adapter
     descriptors for the foreign calls that need a generated wrapper) *)
  let (emit_adapters_path, arglist) =
    match pop_value_flag arglist "emit-adapters" with
    | Some (arglist, path) -> (Some path, arglist)
    | None -> (None, arglist)
  in
  (* Parse --pkg-totality flag (avm-pkg: run the package totality pass,
     which rejects unbounded constructs in package modules). Accepts both
     the bare boolean form (`--pkg-totality`, preset => true) and the value
     form from the release contract (`--pkg-totality=true`), with an
     explicit value overriding the bare form. *)
  let pkg_totality_of_value value =
    match value with
    | "true" -> true
    | "false" -> false
    | _ -> Errors.invalid_bool_flag_value "pkg-totality" value
  in
  let (pkg_totality, arglist) =
    match pop_flag arglist "pkg-totality" with
    | Some (arglist, BoolFlag _) ->
       (true, arglist)
    | Some (arglist, ValueFlag (_, value)) ->
       (pkg_totality_of_value value, arglist)
    | Some (_, PositionalArg _) ->
       internal_err "pop_flag matched a positional argument by flag name"
    | None ->
       (false, arglist)
  in
  (* Parse --emit-derivations=<path> flag (avm-pkg: export the module's
     Derivation constants as plan JSON) *)
  let (emit_derivations, arglist) =
    match pop_value_flag arglist "emit-derivations" with
    | Some (arglist, path) -> (Some path, arglist)
    | None -> (None, arglist)
  in
  (* Parse --emit-linearity-ledger=<path> (docs/memsafe.md §3: export the
     linearity checker's decisions as the ledger ledger2mm0 certifies) *)
  let (emit_linearity_ledger, arglist) =
    match pop_value_flag arglist "emit-linearity-ledger" with
    | Some (arglist, path) -> (Some path, arglist)
    | None -> (None, arglist)
  in
  (* There must be at least one module. *)
  if ((List.length modules) < 1) then
    Errors.missing_module ()
  else
    (* Parse the target type. *)
    let (arglist, target): (arglist * target) = parse_target_type arglist in
    (arglist, WholeProgramCompile { modules = modules; target = target; error_reporting_mode = error_reporting_mode; use_cps_jit = use_cps_jit || jit_server; jit_server = jit_server; allow_all = allow_all; auth_manifest = auth_manifest; emit_cps_path = emit_cps_path; emit_adapters_path = emit_adapters_path; pkg_totality = pkg_totality; emit_derivations = emit_derivations; emit_linearity_ledger = emit_linearity_ledger; })

let parse_compile_command (arglist: arglist): (arglist * cmd) =
  match pop_bool_flag arglist "help" with
  | Some arglist ->
     (arglist, CompileHelp)
  | None ->
     parse_compile_command' arglist

let parse (arglist: arglist): cmd =
  let args: arg list = arglist_to_list arglist in
  match args with
  | [BoolFlag "help"] ->
     HelpCommand
  | [BoolFlag "version"] ->
     VersionCommand
  | (PositionalArg "compile")::rest ->
     (* Try parsing the `compile` command. *)
     let (arglist, cmd) = parse_compile_command (arglist_from_list rest) in
     let _ = check_leftovers arglist in
     cmd
  | _ ->
     HelpCommand
