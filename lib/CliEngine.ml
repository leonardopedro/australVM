(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open CliParser
open Version
open Compiler
open Util
open Error
open HtmlError
open SourceContext

(* Source map stuff *)

(* Map of filenames to file contents. *)
module SourceMap =
  Map.Make(
      struct
        type t = string
        let compare a b = compare a b
      end
    )

type source_map = string SourceMap.t

(* Parsing file contents *)

let make_module_source (m: mod_source): module_source =
  match m with
  | ModuleSource { inter_path; body_path; } ->
     TwoFileModuleSource {
         int_filename = inter_path;
         int_code = read_file_to_string inter_path;
         body_filename = body_path;
         body_code = read_file_to_string body_path
       }
  | ModuleBodySource { body_path; } ->
     BodyModuleSource {
         body_filename = body_path;
         body_code = read_file_to_string body_path
       }

let parse_source_files (mods: mod_source list): (module_source list * source_map) =
  let contents = List.map make_module_source mods in
  (* Build source map for error handling *)
  let source_maps =
    List.map (fun source ->
        match source with
        | TwoFileModuleSource { int_filename; int_code; body_filename; body_code; } ->
           let smap = SourceMap.empty in
           let smap = SourceMap.add int_filename int_code smap in
           let smap = SourceMap.add body_filename body_code smap in
           smap
        | BodyModuleSource { body_filename; body_code; } ->
           let smap = SourceMap.empty in
           let smap = SourceMap.add body_filename body_code smap in
           smap)
      contents
  in
  let source_map =
    List.fold_left
      (fun sm sm' -> SourceMap.union (fun _ v _ -> Some v) sm sm')
      (SourceMap.empty)
      source_maps
  in
  (contents, source_map)

(* Execution *)

(* Stored module sources for hot-swap recompilation *)
let last_module_sources : (module_source list) ref = ref []

(* C output is a cell that is linked later: a symbol the JIT cannot resolve
   now is a cross-cell import the loader resolves, so the compile binds it
   instead of failing. JIT execution targets (`tc`, `exe`) keep strict
   resolution. Pure, so the policy is testable without compiling. *)
let link_time_imports (target: target): bool =
  match target with
  | CStandalone _ -> true
  | _ -> false

let rec exec (cmd: cmd): unit =
  match cmd with
  | HelpCommand ->
     print_usage ()
  | VersionCommand ->
     print_version ()
  | CompileHelp ->
     print_compile_usage ()
  | WholeProgramCompile { modules; target; error_reporting_mode; use_cps_jit; jit_server; allow_all; auth_manifest; emit_cps_path; emit_adapters_path; pkg_totality; emit_derivations; emit_linearity_ledger } ->
     validate_jit_server_target jit_server target;
     (match target with
      | WgslBundle _ | MlirBundle _ ->
         (* The GPU targets are not JIT/hot-swap targets and have no package or
            proof side exports in v1. Refuse the combination before compilation
            instead of ignoring the flag. *)
         let target_name =
           match target with WgslBundle _ -> "wgsl" | _ -> "mlir" in
         if use_cps_jit || jit_server then
           err (Printf.sprintf
                  "--target-type=%s is incompatible with --use-cps-jit/--jit-server: \
                   the GPU target does not lower through CLIF." target_name);
         if emit_cps_path <> None then
           err (Printf.sprintf "--target-type=%s is incompatible with --emit-cps." target_name);
         if emit_adapters_path <> None then
           err (Printf.sprintf "--target-type=%s is incompatible with --emit-adapters." target_name);
         if pkg_totality || emit_derivations <> None then
           err (Printf.sprintf
                  "--target-type=%s is incompatible with package/totality exports." target_name);
         if emit_linearity_ledger <> None then
           err (Printf.sprintf
                  "--target-type=%s is incompatible with --emit-linearity-ledger." target_name);
         if allow_all || auth_manifest <> None then
           err (Printf.sprintf
                  "--target-type=%s does not use the JIT authorization engine; \
                   --allow-all/--auth-manifest are JIT options." target_name)
      | _ -> ());
     Compiler.use_cps_jit := use_cps_jit;
     Compiler.jit_server_mode := jit_server;
     Compiler.emit_cps_path := emit_cps_path;
     Compiler.emit_adapters_path := emit_adapters_path;
     Compiler.pkg_totality := pkg_totality;
     Compiler.emit_derivations_path := emit_derivations;
     Compiler.emit_linearity_ledger_path := emit_linearity_ledger;
     (* C output is a cell to be linked later: a symbol the JIT cannot
        resolve now is a cross-cell import the loader resolves, so bind it
        instead of failing the compile. Executable/JIT-check targets keep
        the strict resolution. *)
     CamlCompiler_rust_bridge.set_link_time_imports (link_time_imports target);
     if allow_all then CamlCompiler_rust_bridge.set_allow_all ();
     (match auth_manifest with
      | Some path ->
         let toml = read_file_to_string path in
         if not (CamlCompiler_rust_bridge.load_auth_manifest toml) then
           err ("Failed to load --auth-manifest " ^ path ^ " (invalid TOML or no [grants] section).")
      | None -> ());
     exec_compile modules target error_reporting_mode

and print_usage _: unit =
  print_endline ("austral " ^ version_string);
  print_endline "";
  print_endline "Usage:";
  print_endline "    austral [options] <command>";
  print_endline "";
  print_endline "Options:";
  print_endline "    --help     Print this text.";
  print_endline "    --version  Print the compiler's version.";
  print_endline "";
  print_endline "Commands:";
  print_endline "    compile    Compile modules."

and print_version _: unit =
  print_endline version_string

and compile_help_lines (): string list =
  (* Prompt text is part of the product: a flag that exists but the help does
     not mention — or a value the help advertises that the parser rejects —
     is a capability the user cannot discover. Keep these lines in lockstep
     with CliParser's accepted flags and target values; CliEngineTest pins
     the shipped surface. *)
  [
    "austral compile";
    "";
    "Usage:";
    "    austral compile [options] <module...>";
    "";
    "Options:";
    "    --help          Print this text.";
    "    --target-type   One of `exe`, `tc`, `c`, `wgsl`, `mlir`. Default is `exe`.";
    "    --gpu-chip      With `--target-type=mlir`, the PTX target chip (e.g.";
    "                    `sm_70`, the default, or `sm_61`). PTX is not";
    "                    forward-compatible, so the chip is recorded.";
    "    --gpu-numeric-profile  With `--target-type=wgsl`, the kernel's numeric";
    "                    contract: `u32-modular-v1` (default) or `wgsl-f32-v1`";
    "                    (Float32 add/subtract/multiply only).";
    "    --output        Path to the output file.";
    "    --entrypoint    The name of the entrypoint function, in the";
    "                    format `<module name>:<function name>`.";
    "    --no-entrypoint  Don't compile an entrypoint. Incompatible with";
    "                    the `exe` target.";
    "    --use-cps-jit   Use CPS JIT compilation pipeline.";
    "    --jit-server    JIT REPL server: stay resident and serve `call|swap|exit`";
    "                    commands on stdin. Implies --use-cps-jit; requires";
    "                    `--target-type=tc` (the server only runs on the tc target).";
    "    --allow-all     Disable authorization (AllowAll engine). Without a manifest";
    "                    or --allow-all the engine denies everything.";
    "    --auth-manifest=<path>  Load a module.toml grant set into the JIT";
    "                    authorization engine.";
    "    --emit-cps      Save CPS binary IR to the given path.";
    "    --emit-adapters Write the adapter descriptors the CPS lowering requires.";
    "    --pkg-totality[=true|false]  Package modules only: run the totality";
    "                    pass, which rejects while loops, recursion, foreign/embed";
    "                    I/O, and other constructs whose termination cannot be";
    "                    proven. Off by default; plans record each derivation as";
    "                    proven only when this flag ran.";
    "    --emit-derivations=<path>  Export this module's Derivation constants";
    "                    as plan JSON (merging with an existing file).";
    "    --emit-linearity-ledger=<path>  Export the linearity checker's";
    "                    decisions (acquisitions, consumptions, borrows) as the";
    "                    JSON ledger scripts/ledger2mm0 certifies (merging with";
    "                    an existing file). See docs/memsafe.md.";
    "";
    "Positional arguments:";
    "    module    Of the form 'file.aui,file.aum' for modules with";
    "              both an interface and body file, or 'file.aum' for";
    "              modules with only a body.";
  ]

and print_compile_usage _: unit =
  List.iter print_endline (compile_help_lines ())

and validate_jit_server_target (jit_server: bool) (target: target): unit =
  (* `--jit-server` only takes effect on the `tc` target. On any other target
     the requested REPL mode would be SILENTLY IGNORED — the compile produces
     an artifact and the stdin `call|swap|exit` protocol never starts, which
     reads to the driver as a hang. Refuse loudly and name the fix. *)
  match target with
  | TypeCheck -> ()
  | _ when jit_server ->
     err ("--jit-server requires `--target-type=tc`: the JIT REPL stays resident \
           and serves `call|swap|exit` on stdin, which only the tc target does. \
           Add --target-type=tc (or drop --jit-server to compile normally).")
  | _ -> ()


and exec_compile (modules: mod_source list) (target: target) (error_reporting_mode: error_reporting_mode): unit =
  (* Parse source files *)
  let (mods, source_map): (module_source list * source_map) = parse_source_files modules in
  last_module_sources := mods;
  (* Error handling setup *)
  try
    exec_target mods target
  with Austral_error error ->
    (* Print errors *)
    begin
      match error_reporting_mode with
      | ErrorReportPlain ->
         let error: austral_error = try_adding_source_ctx error source_map in
         Printf.eprintf "%s" (render_error_to_plain error);
         html_error_dump error;
         dump_and_die ()
      | ErrorReportJson ->
         let error: austral_error = try_adding_source_ctx error source_map in
         Printf.eprintf "%s" (Yojson.Basic.pretty_to_string (render_error_to_json error));
         html_error_dump error;
         dump_and_die ()
    end

and dump_and_die _: unit =
  print_endline "Compiler call tree printed to calltree.html";
  Reporter.dump ();
  exit (-1)

and try_adding_source_ctx (error: austral_error) (source_map: source_map): austral_error =
  let (AustralError { span; source_ctx; _ }) = error in
  match source_ctx with
  | Some _ ->
     (* Already have a context. *)
     error
  | None ->
     (match span with
      | Some span ->
         let (Span { filename; _ }) = span in
         (match (SourceMap.find_opt filename source_map) with
          | Some code ->
             add_source_ctx error (get_source_ctx code span)
          | None ->
             error)
      | None ->
         error)

(* Render an `execute_function` result for the REPL. The bridge returns the
   JIT_PANIC sentinel (i64::MIN) when the JIT-compiled function panicked and
   records the reason on the last-error channel; a panic must surface as
   ERROR with that reason, never as a misleading RESULT. Pure so the test
   suite can pin the sentinel contract. A true i64::MIN result (channel
   empty) still prints as RESULT. *)
and format_exec_result (res: int64) (last_error: string option): string =
  if res = Int64.min_int then
    match last_error with
    | Some err -> "ERROR " ^ err
    | None -> Printf.sprintf "RESULT %Ld" res
  else
    Printf.sprintf "RESULT %Ld" res

and exec_target (mods: module_source list) (target: target): unit =
  match target with
  | TypeCheck ->
     (* Compile everything, emit no code. Routed through the VM plugin
        registry (S36): the compiler is a plugin of the application/VM, so
        this is `Vm_plugin.run_compiler`, not a hard-coded pipeline call. *)
     let _ = Vm_plugin.run_compiler mods in
     (* JIT server mode: keep process alive for repeated calls *)
     if !jit_server_mode then begin
       Printf.eprintf "CPS JIT: Entering server mode (stdin commands)\n%!";
       Printf.printf "READY cmd=call|swap|exit\n%!";
       flush stdout;
       try
         while true do
           let line = try Some (read_line ()) with End_of_file -> None in
           match line with
           | None -> exit 0
           | Some "" -> ()
           | Some line ->
               (* One bad command must not kill the server: `handle_server_line`
                  reports the failure on the protocol channel and returns, so
                  the loop keeps serving — the driving script waits on stdout
                  for a response, and a server that dies mid-protocol reads as
                  a hang. *)
               handle_server_line line
         done
       with exn ->
         Printf.eprintf "CPS JIT: Server error: %s\n%!" (Printexc.to_string exn)
     end


  | Executable { bin_path; entrypoint; } ->
     exec_compile_to_bin mods bin_path entrypoint
  | CStandalone { output_path; entrypoint; } ->
     exec_compile_to_c mods output_path entrypoint
  | WgslBundle { output_dir; entrypoint; numeric_profile; source_profile; gpu_operation; } ->
     exec_compile_to_wgsl mods output_dir entrypoint numeric_profile
       source_profile gpu_operation
  | MlirBundle { output_dir; entrypoint; extent; chip; } ->
     exec_compile_to_mlir mods output_dir entrypoint extent chip

(* Handle one JIT-server command line. Never raises: a command whose
   handling throws (e.g. a swap of a nonexistent module) is reported as
   `ERROR <reason>` on the protocol channel — stdout, not just stderr — and
   the server keeps serving. Parsing splits on single spaces, so multi-word
   args (e.g. a file path) survive as one token. *)
and handle_server_line (line: string): unit =
  try
    let parts = String.split_on_char ' ' line in
    match parts with
    | ["call"; name] ->
        (match Hashtbl.find_opt jit_functions name with
         | Some ptr ->
             let res = CamlCompiler_rust_bridge.execute_function ptr in
             (* JIT_PANIC sentinel → ERROR with the recorded reason;
                everything else prints as RESULT. *)
             Printf.printf "%s\n%!"
               (format_exec_result res (CamlCompiler_rust_bridge.last_jit_error ()));
             flush stdout
         | None ->
             Printf.printf "ERROR unknown function '%s'\n%!" name;
             flush stdout)
    | "swap" :: path_spec :: _ ->
        let mod_src = parse_mod_source path_spec in
        let (new_mods, _) = parse_source_files [mod_src] in
        (* Replace the entry for this module in the stored list *)
        let all_mods = !last_module_sources in
        (* Recompile all modules with the swapped one replaced *)
        let combined = List.map (fun m ->
          let m_name = match m with
            | TwoFileModuleSource { int_filename; _ } -> int_filename
            | BodyModuleSource { body_filename; _ } -> body_filename
          in
          let new_name = match List.hd new_mods with
            | TwoFileModuleSource { int_filename; _ } -> int_filename
            | BodyModuleSource { body_filename; _ } -> body_filename
          in
          if m_name = new_name then List.hd new_mods else m
        ) all_mods in
        if cps_jit_swap_modules combined then
          Printf.printf "SWAP_OK\n%!"
        else
          Printf.printf "SWAP_FAIL\n%!";
        flush stdout
    | ["exit"] -> exit 0
    | _ ->
        Printf.printf "ERROR unknown command\n%!";
        flush stdout
  with exn ->
    Printf.printf "ERROR %s\n%!" (Printexc.to_string exn);
    flush stdout

and exec_compile_to_bin (mods: module_source list) (bin_path: string) (entrypoint: entrypoint): unit =
  (* Compile everything to a C file. Routed through the VM plugin registry
     (S36): the compiler is a plugin of the application/VM. *)
  let compiler = Vm_plugin.run_compiler mods in
  (* Compile the wrapper functions *)
  let compiler = post_compile compiler in
  (* Compile the entrypoint. *)
  let compiler =
    let (Entrypoint (module_name, name)) = entrypoint in
    compile_entrypoint compiler module_name name
  in
  (* Write the output to a temporary file. *)
  let cfile: string = Filename.temp_file "austral_" ".c" in
  (* Removed on the way out, success or not: `Filename.temp_file` lands in
     $TMPDIR and the generated C is megabytes, so every `--target-type=exe`
     compile was leaving one behind. `Fun.protect` so a `cc` failure does not
     skip the cleanup. *)
  Fun.protect
    ~finally:(fun () -> try Sys.remove cfile with Sys_error _ -> ())
    (fun () ->
      write_string_to_file cfile (compiler_code compiler);
      (* Invoke `cc`. *)
      let _ = compile_c_code cfile bin_path in
      ())

and exec_compile_to_c (mods: module_source list) (output_path: string) (entrypoint: entrypoint option): unit =
  (* Compile everything to a C file. Routed through the VM plugin registry
     (S36): the compiler is a plugin of the application/VM. *)
  let compiler = Vm_plugin.run_compiler mods in
  (* Compile the wrapper functions *)
  let compiler = post_compile compiler in
  (* Compile the entrypoint, if needed. *)
  let compiler =
    match entrypoint with
    | Some (Entrypoint (module_name, name)) ->
       (* It's an executable. *)
       compile_entrypoint compiler module_name name
    | None ->
       (* It's a library. *)
       compiler
  in
  (* Write the output to the given file. *)
  write_string_to_file output_path (compiler_code compiler)

(* -------------------------------------------------------------------------- *)
(* GPU / WGSL target (PLAN_gpu_kernels.md G05, G07)                            *)
(* -------------------------------------------------------------------------- *)

and gpu_checker_path (): string option =
  match Sys.getenv_opt "AUSTRAL_GPU_CHECKER" with
  | Some path when String.trim path <> "" -> Some path
  | _ ->
     let rec search = function
       | [] -> None
       | dir :: rest ->
          let candidate = Filename.concat dir "avm-gpu" in
          if Sys.file_exists candidate then Some candidate else search rest
     in
     search (String.split_on_char ':' (try Sys.getenv "PATH" with Not_found -> ""))

(* Run the checker with an argument vector and a bounded wait. No shell
   interpolation: paths are passed as argv entries. Returns (exit, output). *)
and run_gpu_checker (checker: string) (args: string list): (int * string) =
  let out_path = Filename.temp_file "avm-gpu-out-" ".json"
  and err_path = Filename.temp_file "avm-gpu-err-" ".log" in
  let cleanup () =
    (try Sys.remove out_path with Sys_error _ -> ());
    (try Sys.remove err_path with Sys_error _ -> ())
  in
  Fun.protect ~finally:cleanup (fun () ->
    let fd_out = Unix.openfile out_path [Unix.O_WRONLY; Unix.O_CREAT; Unix.O_TRUNC] 0o600
    and fd_err = Unix.openfile err_path [Unix.O_WRONLY; Unix.O_CREAT; Unix.O_TRUNC] 0o600 in
    let argv = Array.of_list (checker :: args) in
    let pid = Unix.create_process checker argv Unix.stdin fd_out fd_err in
    Unix.close fd_out;
    Unix.close fd_err;
    let deadline = Unix.gettimeofday () +. 30.0 in
    let rec wait () =
      let (done_pid, status) = Unix.waitpid [Unix.WNOHANG] pid in
      if done_pid <> 0 then status
      else if Unix.gettimeofday () > deadline then begin
        (try Unix.kill pid Sys.sigkill with Unix.Unix_error _ -> ());
        let _ = Unix.waitpid [] pid in
        err "avm-gpu checker timed out after 30s"
      end else begin
        Unix.sleepf 0.02;
        wait ()
      end
    in
    let status = wait () in
    let read path =
      let ic = open_in_bin path in
      let n = in_channel_length ic in
      let s = really_input_string ic n in
      close_in ic;
      s
    in
    let output = read out_path ^ read err_path in
    let code =
      match status with
      | Unix.WEXITED c -> c
      | Unix.WSIGNALED _ | Unix.WSTOPPED _ -> 1
    in
    (code, output))

and exec_compile_to_wgsl (mods: module_source list) (output_dir: string)
    (entrypoint: entrypoint)
    (numeric_profile: Gpu_ir.numeric_profile)
    (source_profile: Gpu_subset.source_profile)
    (gpu_operation: string option): unit =
  let (Entrypoint (entry_module, entry_name)) = entrypoint in
  if Sys.file_exists output_dir then
    err (Printf.sprintf
           "Refusing to publish GPU bundle: destination %s already exists. \
            Remove it or choose a new --output path." output_dir);
  (* Run the shared checked frontend for each supplied module, in order. The
     GPU target captures the selected module; every module is still checked
     exactly once through the ordinary stages. *)
  let compiler = Compiler.empty_compiler in
  let prepared_selected = ref None in
  let _ =
    List.fold_left
      (fun (env: Env.env) (source: module_source) ->
        let (env, prepared) = Compiler.prepare_module env source in
        if Identifier.mod_name_string prepared.Compiler.prepared_name
           = Identifier.mod_name_string entry_module then
          prepared_selected := Some prepared;
        env)
      (Compiler.cenv compiler) mods
  in
  let prepared =
    match !prepared_selected with
    | Some p -> p
    | None ->
       err (Printf.sprintf
              "Entry module %s is not among the supplied sources."
              (Identifier.mod_name_string entry_module))
  in
  let kernel =
    try
      Gpu_subset.build ~profile:numeric_profile ~source_profile
        ~entry:(Identifier.ident_string entry_name)
        prepared.Compiler.prepared_env prepared.Compiler.prepared_mono
    with Gpu_subset.Refused msg ->
      err ("GPU subset refused the module: " ^ msg)
  in
  let sources =
    List.map
      (fun (source: module_source) ->
        match source with
        | TwoFileModuleSource { int_filename; int_code; body_filename; body_code } ->
           [ Gpu_bundle.digest_source ~path:int_filename int_code;
             Gpu_bundle.digest_source ~path:body_filename body_code ]
        | BodyModuleSource { body_filename; body_code } ->
           [ Gpu_bundle.digest_source ~path:body_filename body_code ])
      mods
    |> List.concat
  in
  let evidence =
    { (Gpu_bundle.default_evidence ()) with
      Gpu_bundle.shader_validation = "checked";
      Gpu_bundle.source_profile = Gpu_subset.source_profile_name source_profile }
  in
  (* A pointwise entry publishes `kernel.json`/`kernel.wgsl`; `map-sum-u32`
     publishes the staged `reduce.json`/`reduce.wgsl` and is validated by the
     checker's reduce path. Ordinary bundle bytes and validation are
     untouched. *)
  let (wgsl, artifact_name, artifact_json, wgsl_name, validate_command) =
    match gpu_operation with
    | None ->
       let wgsl = Gpu_wgsl.render kernel in
       let bundle = Gpu_bundle.make ~evidence ~wgsl ~sources kernel in
       (wgsl, "kernel.json", Gpu_bundle.render_kernel_json bundle,
        "kernel.wgsl", "validate")
    | Some "map-sum-u32" ->
       (Gpu_reduce_wgsl.render kernel, "reduce.json",
        Gpu_reduce_wgsl.metadata ~evidence ~sources kernel,
        "reduce.wgsl", "validate-reduce")
    | Some "map-sum-u32-x4" ->
       (Gpu_reduce_wgsl.render ~variant:Gpu_reduce_wgsl.Lane4 kernel,
        "reduce.json",
        Gpu_reduce_wgsl.metadata ~variant:Gpu_reduce_wgsl.Lane4
          ~evidence ~sources kernel,
        "reduce.wgsl", "validate-reduce")
    | Some other ->
       err (Printf.sprintf "Internal error: unhandled GPU operation %s" other)
  in
  if String.trim wgsl = "" then
    err "Internal error: the WGSL renderer produced an empty shader.";
  let staging =
    Printf.sprintf "%s.staging-%d" output_dir (Unix.getpid ())
  in
  if Sys.file_exists staging then
    err (Printf.sprintf "Refusing to reuse an existing staging directory %s." staging);
  let published = ref false in
  Fun.protect
    ~finally:(fun () ->
      if not !published then begin
        (try Sys.remove (Filename.concat staging artifact_name) with Sys_error _ -> ());
        (try Sys.remove (Filename.concat staging wgsl_name) with Sys_error _ -> ());
        (try Unix.rmdir staging with Unix.Unix_error _ -> ())
      end)
    (fun () ->
      Unix.mkdir staging 0o755;
      write_string_to_file (Filename.concat staging artifact_name) artifact_json;
      write_string_to_file (Filename.concat staging wgsl_name) wgsl;
      let checker =
        match gpu_checker_path () with
        | Some path -> path
        | None ->
           err
             "The GPU checker is required to publish a bundle, but no \
              AUSTRAL_GPU_CHECKER path and no avm-gpu on PATH were found. \
              Build/install safestos/gpu (cargo build --release \
              --manifest-path safestos/gpu/Cargo.toml --bin avm-gpu) and set \
              AUSTRAL_GPU_CHECKER, or put avm-gpu on PATH."
      in
      let (code, output) = run_gpu_checker checker [validate_command; staging] in
      if code <> 0 then
        err (Printf.sprintf
               "The GPU checker refused the staged %s (exit %d):\n%s"
               (if gpu_operation = None then "bundle" else "reduction bundle")
               code output);
      (* The checker's structured response must agree with what we staged. *)
      (try
         let json = Yojson.Safe.from_string output in
         let open Yojson.Safe.Util in
         let expected_schema, expected_program =
           match gpu_operation with
           | None ->
              (Gpu_bundle.schema, None)
           | Some "map-sum-u32-x4" ->
              (Gpu_reduce_wgsl.schema,
               Some (Gpu_reduce_wgsl.program_id
                       ~variant:Gpu_reduce_wgsl.Lane4 kernel))
           | Some _ ->
              (Gpu_reduce_wgsl.schema, Some (Gpu_reduce_wgsl.program_id kernel))
         in
         let schema = json |> member "schema" |> to_string in
         if schema <> expected_schema then
           err (Printf.sprintf "Checker schema %s does not match %s"
                  schema expected_schema);
         let ir = json |> member "ir_sha256" |> to_string
         and wgsl_sha = json |> member "wgsl_sha256" |> to_string in
         if ir <> Gpu_ir.semantic_sha256 kernel then
           err "Checker IR digest does not match the staged KernelIR.";
         if wgsl_sha <> Gpu_ir.sha256_hex wgsl then
           err "Checker WGSL digest does not match the staged shader.";
         (match expected_program with
          | None -> ()
          | Some program ->
             let staged = json |> member "program_sha256" |> to_string in
             if staged <> program then
               err "Checker program digest does not match the reduction program.")
       with Yojson.Json_error msg ->
         err ("The GPU checker returned malformed JSON: " ^ msg));
      Unix.rename staging output_dir;
      published := true;
      Printf.eprintf "GPU %s published to %s\n%!"
        (if gpu_operation = None then "bundle" else "reduction bundle")
        output_dir)

(* -------------------------------------------------------------------------- *)
(* MLIR bundle target (PLAN_gpu_kernels.md G20b)                                *)
(* -------------------------------------------------------------------------- *)

and mlir_opt_path (): string option =
  match Sys.getenv_opt "AUSTRAL_MLIR_OPT" with
  | Some path when String.trim path <> "" && Sys.file_exists (String.trim path) ->
     Some (String.trim path)
  | _ ->
     let rec search = function
       | [] -> None
       | dir :: rest ->
          let candidate = Filename.concat dir "mlir-opt" in
          if Sys.file_exists candidate then Some candidate else search rest
     in
     search (String.split_on_char ':' (try Sys.getenv "PATH" with Not_found -> ""))

(* Run one tool with an argument vector, no shell, bounded wait. Returns the
   exit status and stdout; stderr goes to a temp file and is appended so a
   pipeline failure reports the compiler's own message. *)
and run_tool (tool: string) (args: string list): (int * string) =
  let out_path = Filename.temp_file "avm-tool-out-" ".txt"
  and err_path = Filename.temp_file "avm-tool-err-" ".txt" in
  let cleanup () =
    (try Sys.remove out_path with Sys_error _ -> ());
    (try Sys.remove err_path with Sys_error _ -> ())
  in
  Fun.protect ~finally:cleanup (fun () ->
    let fd_out = Unix.openfile out_path [Unix.O_WRONLY; Unix.O_CREAT; Unix.O_TRUNC] 0o600
    and fd_err = Unix.openfile err_path [Unix.O_WRONLY; Unix.O_CREAT; Unix.O_TRUNC] 0o600 in
    let argv = Array.of_list (tool :: args) in
    let pid = Unix.create_process tool argv Unix.stdin fd_out fd_err in
    Unix.close fd_out;
    Unix.close fd_err;
    let deadline = Unix.gettimeofday () +. 120.0 in
    let rec wait () =
      let (done_pid, status) = Unix.waitpid [Unix.WNOHANG] pid in
      if done_pid <> 0 then status
      else if Unix.gettimeofday () > deadline then begin
        (try Unix.kill pid Sys.sigkill with Unix.Unix_error _ -> ());
        let _ = Unix.waitpid [] pid in
        err (Printf.sprintf "%s timed out after 120s" tool)
      end else begin
        Unix.sleepf 0.02;
        wait ()
      end
    in
    let status = wait () in
    let read path =
      let ic = open_in_bin path in
      let n = in_channel_length ic in
      let s = really_input_string ic n in
      close_in ic;
      s
    in
    let combined = read out_path ^ read err_path in
    let code = match status with
      | Unix.WEXITED c -> c
      | Unix.WSIGNALED _ | Unix.WSTOPPED _ -> 1
    in
    (code, combined))

and exec_compile_to_mlir (mods: module_source list) (output_dir: string)
    (entrypoint: entrypoint) (extent: int) (chip: string): unit =
  let (Entrypoint (entry_module, entry_name)) = entrypoint in
  if Sys.file_exists output_dir then
    err (Printf.sprintf
           "Refusing to publish an MLIR bundle: destination %s already exists. \
            Remove it or choose a new --output path." output_dir);
  (* The same checked frontend as the WGSL target: one KernelIR, two consumers. *)
  let compiler = Compiler.empty_compiler in
  let prepared_selected = ref None in
  let _ =
    List.fold_left
      (fun (env: Env.env) (source: module_source) ->
        let (env, prepared) = Compiler.prepare_module env source in
        if Identifier.mod_name_string prepared.Compiler.prepared_name
           = Identifier.mod_name_string entry_module then
          prepared_selected := Some prepared;
        env)
      (Compiler.cenv compiler) mods
  in
  let prepared =
    match !prepared_selected with
    | Some p -> p
    | None ->
       err (Printf.sprintf
              "Entry module %s is not among the supplied sources."
              (Identifier.mod_name_string entry_module))
  in
  let kernel =
    try
      Gpu_subset.build ~entry:(Identifier.ident_string entry_name)
        prepared.Compiler.prepared_env prepared.Compiler.prepared_mono
    with Gpu_subset.Refused msg ->
      err ("GPU subset refused the module: " ^ msg)
  in
  (* Refuse before publication, with the emitter's own words. `render` also
     refuses a non-positive extent and a non-integer profile; the parser has
     already refused both, so this is the second line of defence. *)
  let mlir =
    try Gpu_mlir.render ~extent kernel
    with Invalid_argument msg -> err ("MLIR emission refused: " ^ msg)
  in
  (* `validate` raises on the first structural disagreement and otherwise
     returns its counts; the shape is a guard here, not a claim. *)
  let _counts =
    try Gpu_mlir.validate mlir
    with Invalid_argument msg ->
      err ("Internal error: the emitted module failed its own structural \
            check: " ^ msg)
  in
  let tool =
    match mlir_opt_path () with
    | Some path -> path
    | None ->
       err
         "The MLIR bundle target needs `mlir-opt`, but no AUSTRAL_MLIR_OPT path \
          and no mlir-opt on PATH were found. Install an LLVM/MLIR release \
          (this repo measures 19.1.7 from nixpkgs `llvmPackages_19.mlir`) and \
          set AUSTRAL_MLIR_OPT, or use --target-type=wgsl for a toolchain-free \
          artifact."
  in
  let (version_code, version) = run_tool tool ["--version"] in
  (* `mlir-opt --version` prints the LLVM banner and then the version; the
     recorded identity keeps both, because "LLVM (http://llvm.org/)" alone
     would not say which release verified the module. *)
  let toolchain =
    if version_code = 0 then
      let lines =
        String.split_on_char '\n' (String.trim version)
        |> List.map String.trim
        |> List.filter (fun l -> l <> "")
      in
      (match lines with
       | first :: second :: _ -> first ^ " " ^ second
       | [only] -> only
       | [] -> "mlir-opt (version unavailable)")
    else "mlir-opt (version unavailable)"
  in
  let staging = Printf.sprintf "%s.staging-%d" output_dir (Unix.getpid ()) in
  if Sys.file_exists staging then
    err (Printf.sprintf "Refusing to reuse an existing staging directory %s." staging);
  let published = ref false in
  Fun.protect
    ~finally:(fun () ->
      if not !published then begin
        List.iter (fun name ->
          try Sys.remove (Filename.concat staging name) with Sys_error _ -> ())
          [ Gpu_mlir_bundle.file_json; Gpu_mlir_bundle.file_kernelir;
            Gpu_mlir_bundle.file_mlir; Gpu_mlir_bundle.file_ptx ];
        (try Unix.rmdir staging with Unix.Unix_error _ -> ())
      end)
    (fun () ->
      Unix.mkdir staging 0o755;
      let mlir_path = Filename.concat staging Gpu_mlir_bundle.file_mlir in
      write_string_to_file mlir_path mlir;
      (* 1. verify: `mlir-opt <module>` parses, verifies and prints it. *)
      let (verify_code, verify_out) = run_tool tool [mlir_path] in
      if verify_code <> 0 then
        err (Printf.sprintf
               "mlir-opt refused the emitted module (exit %d):\n%s\n\
                Nothing was published." verify_code (String.trim verify_out));
      (* 2. lower: the pinned pipeline, one argv element for its options. *)
      let (lower_code, lowered) =
        run_tool tool [mlir_path; Gpu_mlir_bundle.pipeline_for chip]
      in
      if lower_code <> 0 then
        err (Printf.sprintf
               "mlir-opt failed to lower the module (exit %d):\n%s\n\
                Nothing was published." lower_code (String.trim lowered));
      (* 3. extract: the PTX bytes the pipeline embedded. *)
      let ptx =
        match Gpu_mlir_bundle.extract_assembly lowered with
        | Some ptx -> ptx
        | None ->
           err "The lowering produced no gpu.object assembly; the pinned pipeline \
                must emit PTX text."
      in
      let facts = Gpu_mlir_bundle.ptx_facts ptx in
      (match Gpu_mlir_bundle.abi_mismatches ~arity:kernel.Gpu_ir.input_arity
               ~chip facts with
       | [] -> ()
       | problems ->
          err (Printf.sprintf
                 "The lowered ABI does not match the recorded kernel:\n  %s\n\
                  Nothing was published." (String.concat "\n  " problems)));
      (* 4. bind: the derivative digests are computed from the bytes written. *)
      write_string_to_file
        (Filename.concat staging Gpu_mlir_bundle.file_kernelir)
        (Gpu_ir.canonical_bytes kernel);
      write_string_to_file
        (Filename.concat staging Gpu_mlir_bundle.file_ptx) ptx;
      let sources =
        List.map
          (fun (source: module_source) ->
            match source with
            | TwoFileModuleSource { int_filename; int_code; body_filename; body_code } ->
               [ Gpu_bundle.digest_source ~path:int_filename int_code;
                 Gpu_bundle.digest_source ~path:body_filename body_code ]
            | BodyModuleSource { body_filename; body_code } ->
               [ Gpu_bundle.digest_source ~path:body_filename body_code ])
          mods
        |> List.concat
      in
      let sources = sources @ Gpu_bundle.builtin_sources () in
      let record =
        Gpu_mlir_bundle.make ~entry:(Identifier.ident_string entry_name)
          ~extent ~sources ~kernel ~mlir ~ptx
          ~lowering:{ Gpu_mlir_bundle.target = chip;
                      pipeline = Gpu_mlir_bundle.pipeline_for chip;
                      toolchain;
                      ptxas = "not used: cubin-format=isa stops at PTX text" }
      in
      write_string_to_file
        (Filename.concat staging Gpu_mlir_bundle.file_json)
        (Gpu_mlir_bundle.render record);
      (* 5. check the set as a reader would, before it becomes visible. *)
      (match Gpu_mlir_bundle.verify_dir staging with
       | Ok _ -> ()
       | Error problems ->
          err (Printf.sprintf
                 "The staged MLIR bundle failed its own consistency check:\n  %s\n\
                  Nothing was published." (String.concat "\n  " problems)));
      Unix.rename staging output_dir;
      published := true;
      Printf.eprintf
        "MLIR bundle published to %s (KernelIR %s…, %s, %d x u64 params)\n%!"
        output_dir (String.sub record.Gpu_mlir_bundle.ir_sha256 0 12)
        record.Gpu_mlir_bundle.lowering.target
        record.Gpu_mlir_bundle.abi.Gpu_mlir_bundle.ptx_params)
