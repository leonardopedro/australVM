(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open Identifier
open Id
open Env
open BuiltIn
open BuiltInModules
open ParserInterface
open CombiningPass
open ExtractionPass
open TypingPass
open BodyExtractionPass
open CodeGen
open CRenderer
open CRepr
open Cst
open CstUtil
open Stages.Tast
open Error
open Stages
open Linked
open Mtast
open Monomorphize
open ReturnCheck
open LinearityCheck
open Reporter
open Entrypoint
open ExportInstantiation
open HtmlError

(* Phase 7: CPS JIT Integration *)
let use_cps_jit = ref false
let jit_server_mode = ref false
let emit_cps_path = ref (None: string option)
let jit_functions : (string, int64) Hashtbl.t = Hashtbl.create 16

let append_import_to_interface (ci: concrete_module_interface) (import: concrete_import_list): concrete_module_interface =
  let (ConcreteModuleInterface (mn, docstring, imports, decls)) = ci in
  if equal_module_name mn pervasive_module_name then
    ci
  else
    ConcreteModuleInterface (mn, docstring, import :: imports, decls)

let append_import_to_body (cb: concrete_module_body) (import: concrete_import_list): concrete_module_body =
  let (ConcreteModuleBody (mn, kind, docstring, imports, decls)) = cb in
  if equal_module_name mn pervasive_module_name then
    cb
  else
    ConcreteModuleBody (mn, kind, docstring, import :: imports, decls)

type compiler = Compiler of env * string

(******************************************************************************)
(* SAFESTOS: Typed Eval Interface for Runtime Compilation                    *)
(******************************************************************************)

(* Parse a single expression or declaration into C source code *)
let _parse_and_compile_c (_source: string) : string option =
  None

(* Compile a module with @cell attribute *)
let _compile_cell_module (_source: string) : (string * string) option =
  None

(** Extract the env from the compiler. *)
let cenv (Compiler (m, _)): env = m

let compiler_code (Compiler (_, c)): string = c

type module_source =
  | TwoFileModuleSource of {
      int_filename: string;
      int_code: string;
      body_filename: string;
      body_code: string
    }
  | BodyModuleSource of {
      body_filename: string;
      body_code: string
    }

let parse_and_combine (env: env) (source: module_source): (env * module_name * Combined.combined_module * file_id option * file_id) =
  match source with
  | TwoFileModuleSource { int_filename; int_code; body_filename; body_code } ->
     let (env, int_file_id) = add_file env { path = int_filename; contents = int_code } in
     let (env, body_file_id) = add_file env { path = body_filename; contents = body_code } in
     let ci: concrete_module_interface = parse_module_int int_code int_filename in
     let name: module_name = mod_int_name ci in
     adorn_error_with_module_name name
       (fun _ ->
         let cb: concrete_module_body = parse_module_body body_code body_filename in
         let ci: concrete_module_interface = append_import_to_interface ci pervasive_imports
         and cb: concrete_module_body = append_import_to_body cb pervasive_imports in
         let combined: Combined.combined_module = combine env ci cb in
         (env, name, combined, Some int_file_id, body_file_id))
  | BodyModuleSource { body_filename; body_code } ->
     let (env, body_file_id) = add_file env { path = body_filename; contents = body_code } in
     let cb: concrete_module_body = parse_module_body body_code body_filename in
     let name: module_name = mod_body_name cb in
     adorn_error_with_module_name name
       (fun _ ->
         let cb: concrete_module_body = append_import_to_body cb pervasive_imports in
         let combined: Combined.combined_module = body_as_combined env cb in
         (env, name, combined, None, body_file_id))

let rec compile_mod (c: compiler) (source: module_source): compiler =
  with_frame "Compile module"
    (fun _ ->
      let env: env = cenv c in
      let (env, name, combined, int_file_id, body_file_id) = parse_and_combine env source in
      adorn_error_with_module_name name
        (fun _ ->
          let _ = check_ends_in_return combined in
          let combined: SmallCombined.combined_module = DesugaringPass.desugar combined in
          let (env, linked): (env * linked_module) = extract env combined int_file_id body_file_id in
          let typed: typed_module = augment_module env linked in
          (* S36 WhyML plugin seam: every registered compiler pass runs on the
             typed module after typing and before codegen. A VerdictReject
             aborts compilation — the machine-verified authorization gate
             (Why3-extracted) refuses modules whose uk_* imports are not
             granted. *)
          (match Compiler_plugin.run_on_typed typed with
           | Compiler_plugin.VerdictReject msg ->
              (* `austral_raise` never returns — it raises the Austral_error. *)
              austral_raise TypeError [ErrorText.Text msg]
           | Compiler_plugin.VerdictOk -> ());
          let _ = check_module_linearity typed in
          let env: env = extract_bodies env typed in
          let (env, mono): (env * mono_module) = monomorphize env typed in
          
          (* Phase 7: CPS JIT Integration Path *)
          if !use_cps_jit then begin
            try
              let (funcs, module_name) = Compiler_cps.compile_module_cps mono in
              (if Sys.getenv_opt "CPS_DEBUG" <> None then
                 Compiler_cps.debug_print_cps_functions funcs);
              if List.length funcs > 0 then begin
                let binary = CpsGen.serialize_functions ~module_name funcs in
                (match !emit_cps_path with
                 | Some path ->
                    Compiler_cps.write_cps_binary funcs module_name path;
                    Printf.eprintf "CPS JIT: Emitted CPS binary to %s\n%!" path
                 | None -> ());
                Printf.eprintf "CPS JIT: Generated %d functions (%d bytes)\n%!"
                  (List.length funcs) (String.length binary);

                (* Ensure bridge is initialized *)
                if not (CamlCompiler_rust_bridge.initialize ()) then
                  Printf.eprintf "CPS JIT: Warning — Rust bridge failed to initialize\n%!";

                (* Compile binary — always compiles all functions into JIT module *)
                let (_fn_ptr, jit_err) = CamlCompiler_rust_bridge.compile_binary binary in
                (match jit_err with
                 | Some msg -> Printf.eprintf "CPS JIT: Compilation error: %s\n%!" msg
                 | None -> ());

                (* Save all compiled function pointers for re-lookup *)
                let names = CamlCompiler_rust_bridge.list_function_names () in
                List.iter (fun name ->
                  let ptr = CamlCompiler_rust_bridge.lookup_function name in
                  if ptr <> Int64.zero then
                    Hashtbl.replace jit_functions name ptr
                ) names;
                Printf.eprintf "CPS JIT: %d functions compiled\n%!" (Hashtbl.length jit_functions);

                (* Always also emit C backend output for linking *)
                let unit: c_unit = gen_module env mono in
                let unit_code: string = render_unit unit in
                let code: string = (compiler_code c) ^ "\n" ^ unit_code in
                Compiler (env, code)
              end else begin
                Printf.eprintf "CPS JIT: No compilable functions — using C backend\n%!";
                let unit: c_unit = gen_module env mono in
                let unit_code: string = render_unit unit in
                let code: string = (compiler_code c) ^ "\n" ^ unit_code in
                Compiler (env, code)
              end
            with exn ->
              Printf.eprintf "CPS JIT: Unhandled exception (%s) — falling back to C\n%!"
                (Printexc.to_string exn);
              let unit: c_unit = gen_module env mono in
              let unit_code: string = render_unit unit in
              let code: string = (compiler_code c) ^ "\n" ^ unit_code in
              Compiler (env, code)

          end
          else begin
            (* Original C codegen path *)
            let unit: c_unit = gen_module env mono in
            let unit_code: string = render_unit unit in
            let code: string = (compiler_code c) ^ "\n" ^ unit_code in
            Compiler (env, code)
          end))

let rec compile_multiple c modules =
  match modules with
  | m::rest -> compile_multiple (compile_mod c m) rest
  | [] -> c

(* Hot-swap: recompile all module sources via the CPS-JIT path and replace
   the running JIT module. The C codegen output is discarded — only the
   JIT function table is updated. Returns true on success. *)
let rec cps_jit_swap_modules (mods: module_source list): bool =
  if not !use_cps_jit then begin
    Printf.eprintf "CPS JIT: swap requires --use-cps-jit\n%!";
    false
  end else
  try
    (* Build env starting from empty_env with Pervasive + Memory modules *)
    let env = ref empty_env in
    let code = ref prelude_c in
    let compile_mod_to_env (source: module_source) =
      let (new_env, _name, combined, int_file_id, body_file_id) =
        parse_and_combine !env source in
      env := new_env;
      let _ = check_ends_in_return combined in
      let combined = DesugaringPass.desugar combined in
      let (new_env, linked) = extract !env combined int_file_id body_file_id in
      env := new_env;
      let typed = augment_module !env linked in
      (* S36 WhyML plugin seam (same gate as compile_mod). *)
      (match Compiler_plugin.run_on_typed typed with
       | Compiler_plugin.VerdictReject msg ->
          austral_raise TypeError [ErrorText.Text msg]
       | Compiler_plugin.VerdictOk -> ());
      let _ = check_module_linearity typed in
      let new_env = extract_bodies !env typed in
      env := new_env;
      let (new_env, mono) = monomorphize !env typed in
      env := new_env;
      let unit: c_unit = gen_module !env mono in
      let unit_code = render_unit unit in
      code := !code ^ "\n" ^ unit_code
    in
    let make_source is bs = TwoFileModuleSource { int_filename = ""; int_code = is; body_filename = ""; body_code = bs } in
    compile_mod_to_env (make_source pervasive_interface_source pervasive_body_source);
    compile_mod_to_env (make_source memory_interface_source memory_body_source);
    let all_funcs = ref [] in
    let swap_module_name = ref "" in
    List.iter (fun source ->
      let (new_env, _name, combined, int_file_id, body_file_id) =
        parse_and_combine !env source in
      env := new_env;
      let _ = check_ends_in_return combined in
      let combined = DesugaringPass.desugar combined in
      let (new_env, linked) = extract !env combined int_file_id body_file_id in
      env := new_env;
      let typed = augment_module !env linked in
      let _ = check_module_linearity typed in
      let new_env = extract_bodies !env typed in
      env := new_env;
      let (new_env, mono) = monomorphize !env typed in
      env := new_env;
      let (funcs, module_name) = Compiler_cps.compile_module_cps mono in
      swap_module_name := module_name;
      all_funcs := !all_funcs @ funcs
    ) mods;
    if List.length !all_funcs > 0 then begin
      let binary = CpsGen.serialize_functions ~module_name:!swap_module_name !all_funcs in
      Printf.eprintf "CPS JIT: Swap compiled %d functions (%d bytes)\n%!"
        (List.length !all_funcs) (String.length binary);

      let (_fn_ptr, jit_err) = CamlCompiler_rust_bridge.swap_binary binary in
      (match jit_err with
       | Some msg -> Printf.eprintf "CPS JIT: Swap error: %s\n%!" msg; false
       | None ->
           (* Update jit_functions hashtable *)
           Hashtbl.clear jit_functions;
           let names = CamlCompiler_rust_bridge.list_function_names () in
           List.iter (fun name ->
             let ptr = CamlCompiler_rust_bridge.lookup_function name in
             if ptr <> Int64.zero then
               Hashtbl.replace jit_functions name ptr
           ) names;
           Printf.eprintf "CPS JIT: Swap complete — %d functions ready\n%!"
             (Hashtbl.length jit_functions);
           true)
    end else begin
      Printf.eprintf "CPS JIT: No CPS functions to swap\n%!";
      false
    end
  with exn ->
    Printf.eprintf "CPS JIT: Swap failed: %s\n%!"
      (match exn with
       | Austral_error error -> render_error_to_plain error
       | _ -> Printexc.to_string exn);
    false

let compile_entrypoint c mn i =
  let qi = make_qident (mn, i, i) in
  let (Compiler (m, code)) = c in
  let entry_code: string = entrypoint_code m qi in
  Compiler (m, code ^ "\n" ^ entry_code)

let fake_mod_source (is: string) (bs: string): module_source =
  TwoFileModuleSource { int_filename = ""; int_code = is; body_filename = ""; body_code = bs }

let dump_and_die _: 'a =
  print_endline "Compiler call tree printed to calltree.html";
  Reporter.dump ();
  exit (-1)

let post_compile (compiler: compiler): compiler =
  let env: env = cenv compiler in
  let (env, decls): env * mdecl list = monomorphize_wrappers env in
  let unit: c_unit = gen_module env (MonoModule (make_mod_name "Austral.Wrappers", decls)) in
  let unit_code: string = render_unit unit in
  let wrappers: c_unit = CUnit ("Wrappers", all_wrappers env) in
  let wrapper_code: string = render_unit wrappers in
  let code: string = (compiler_code compiler) ^ "\n" ^ unit_code ^ "\n" ^ wrapper_code in
  Compiler (env, code)

let empty_compiler: compiler =
  with_frame "Compile built-in modules"
    (fun _ ->
      (* S36: install the WhyML-derived compiler passes (idempotent). *)
      Why3_plugin.install ();
      (* We have to compile the Austral.Pervasive module, followed by
         Austral.Memory, since the latter uses declarations from the former. *)
      let env: env = empty_env in
      (* Start with the C prelude. *)
      let c = Compiler (env, prelude_c) in
      let c =
        (* Handle errors during the compilation of the Austral,Pervasive
           module. Otherwise, a typo in the source code of this module will cause a
           fatal error due to an exception stack overflow (unsure why this
           happens). *)
        try
          compile_mod c (fake_mod_source pervasive_interface_source pervasive_body_source)
        with Austral_error error ->
          Printf.eprintf "%s" (render_error_to_plain error);
          html_error_dump error;
          dump_and_die ()
      in
      let c =
        try
          compile_mod c (fake_mod_source memory_interface_source memory_body_source)
        with Austral_error error ->
          Printf.eprintf "%s" (render_error_to_plain error);
          html_error_dump error;
          dump_and_die ()
      in
      c)
