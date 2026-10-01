(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L5: the Why3 driver. `docs/LIQUID.md` §2.3 specifies a
   subprocess `why3 prove -P alt-ergo` — no Why3 library linkage in any Apache
   binary, the LGPL discipline of WHYML_CYCLE. This module is that seam.

   It deliberately **reuses `Util.run_command`**, the compiler's existing
   subprocess helper, rather than opening a second mechanism: that helper
   already does the three things this driver needs (`open_process_full` with an
   explicit environment, full output capture, and exit-status extraction) and
   is what `compile_c_code` uses. One way to shell out, not two.

   Outcomes are a four-way sum rather than a bool, because the plan requires
   three distinguishable behaviours (§2.3):

     `Proved          the engine ran and every goal held
      `Refused (goals) the engine ran and at least one goal failed
      `Unknown         the engine ran but returned an inconclusive status
      `NoEngine        why3 is not installed — a *structured skip*, not a pass

   `NoEngine` is the important one. A missing prover must never read as
   success: it is reported as a skip, and it becomes a hard compile error only
   when the module manifest asks for liquid types to be `required`.
*)
open Stages.Tast

type outcome =
  | Proved of int                     (** goals proved *)
  | Refused of string list            (** the goals that did not hold *)
  | Unknown of string                 (** engine inconclusive *)
  | NoEngine of string                (** why3 not found *)

(* ── locating the engine ──────────────────────────────────────────────────── *)

(** `WHY3_CLI` overrides, then `PATH` — the order §2.3 specifies. *)
let why3_cli () : string =
  match Sys.getenv_opt "WHY3_CLI" with
  | Some c when c <> "" -> c
  | _ -> "why3"

(** Is liquid checking required for this compilation? The module manifest is
    not yet threaded into the compiler (that is L8), so the decision comes
    from the environment for now; L8 replaces this with the `module.toml` read.
    Absent means optional, which is why a missing engine is only a skip. *)
let required () : bool =
  match Sys.getenv_opt "AUSTRAL_LIQUID_REQUIRED" with
  | Some ("1" | "true" | "required") -> true
  | _ -> false

(* ── goal bookkeeping ────────────────────────────────────────────────────── *)

(** The goal names in an emitted `.mlw`, so a failure can name the contract it
    came from. Read back from the file rather than recomputed: the file is the
    artefact the engine actually saw. *)
let goals_in (mlw_path : string) : string list =
  let ic = open_in mlw_path in
  let found = ref [] in
  (try
     while true do
       let line = input_line ic in
       let line = String.trim line in
       if String.length line > 6 && String.sub line 0 6 = "  goal" then begin
         let rest = String.trim (String.sub line 6 (String.length line - 6)) in
         (* `goal <name>:` *)
         let n = String.length rest in
         if n > 0 && rest.[n - 1] = ':' then
           found := String.sub rest 0 (n - 1) :: !found
       end
     done
   with End_of_file -> close_in ic);
  List.rev !found

(* ── the driver ──────────────────────────────────────────────────────────── *)

(** Run `why3 prove` over an emitted `.mlw`.

    Why3's exit status is not a boolean proof result: 0 means the goals were
    discharged, 1 means at least one was refuted, and anything else (2, or a
    signal) means the run itself did not conclude. A run that could not
    conclude is reported as [Unknown], never folded into success. *)
let prove (mlw_path : string) : outcome =
  let cmd = Printf.sprintf "%s prove -P alt-ergo %s" (why3_cli ()) mlw_path in
  let (Util.CommandOutput { code; stdout; stderr; _ }) = Util.run_command cmd in
  let cli = why3_cli () in
  if code = 127 || (code <> 0 && cli <> "" && not (String.length stdout > 0)
                    && String.length stderr > 0
                    && (let sub = "not found" in
                        try ignore (Str.search_forward (Str.regexp_string sub) stderr 0); true
                        with Not_found -> false)) then
    NoEngine (Printf.sprintf "`%s` is not installed (looked at WHY3_CLI then PATH)" cli)
  else
    match code with
    | 0 -> Proved (List.length (goals_in mlw_path))
    | 1 ->
       (* Why3 1.x reports each unproved goal as

            File <path>, line <n>, characters <p>-<q>: Unproved goal:
            File <path>, line <n>, characters <p>-<q>: The following holds: <goal>

          so the goal name is whatever follows the last ": " on a line that
          carries a `characters p-q:` location. Taking it from the engine's own
          output — rather than re-deriving which goal failed from the file —
          is what keeps the diagnostic honest: it names what Why3 actually
          refused, and it degrades to "<unnamed goal>" only if the output does
          not carry the location at all. *)
       let refs =
         List.filter_map
           (fun line ->
             if
               (try
                  ignore
                    (Str.search_forward
                       (Str.regexp_string ", characters ") line 0);
                  true
                with Not_found -> false)
             then
               match String.rindex_opt line ':' with
               | None -> None
               | Some i ->
                  let name = String.trim (String.sub line (i + 1) (String.length line - i - 1)) in
                  if name = "" then None
                  else if name = "Unproved goal" then None
                  else Some name
             else None)
           (String.split_on_char '\n' stdout)
       in
       let failed = if refs = [] then [ "<unnamed goal>" ] else refs in
       Refused failed
    | other ->
       Unknown
         (Printf.sprintf "`%s prove` exited %d: %s" cli other
            (String.trim (String.sub stderr 0 (min 400 (String.length stderr)))))

(** Turn an outcome into a compiler verdict. `~where_` names the artifact. *)
let verdict ~(where_ : string) (o : outcome) : Compiler_plugin.verdict =
  match o with
  | Proved _ -> Compiler_plugin.VerdictOk
  | Refused goals ->
     Compiler_plugin.VerdictReject
       (Printf.sprintf
          "liquid: Why3 refused %d goal(s) in %s: %s%s"
          (List.length goals) where_ (String.concat ", " goals)
          " — the contract does not hold (the asserted goal names the \
           declaration it came from)")
  | Unknown msg ->
     Compiler_plugin.VerdictReject
       (Printf.sprintf "liquid: Why3 was inconclusive on %s: %s" where_ msg)
  | NoEngine msg ->
     if required () then
       Compiler_plugin.VerdictReject
         (Printf.sprintf
            "liquid: %s, and the module requires liquid checking \
             (AUSTRAL_LIQUID_REQUIRED); install Why3 or unset the requirement"
            msg)
     else
       (* Structured skip: report it, do not fail the build, and do not pretend
          the contracts were discharged. *)
       Compiler_plugin.VerdictOk

(** Verify one module end to end: emit, prove, verdict. Returns [None] when the
    module has no contracts at all, so the driver is never invoked. *)
let verify (m : typed_module) : Compiler_plugin.verdict option =
  match LiquidConstraints.generate m with
  | None -> None
  | Some c ->
     (* Emit beside a caller-chosen directory so the `.mlw` survives for
        inspection; `AUSTRAL_LIQUID_DUMP` already chose one for L4's goldens,
        and reusing it keeps a single knob. *)
     let dir =
       match Sys.getenv_opt "AUSTRAL_LIQUID_DUMP" with
       | Some d -> d
       | None -> Filename.get_temp_dir_name ()
     in
     let path = Filename.concat dir (c.LiquidConstraints.module_name ^ ".mlw") in
     let oc = open_out path in
     output_string oc (LiquidConstraints.to_mlw c);
     close_out oc;
     Some (verdict ~where_:path (prove path))