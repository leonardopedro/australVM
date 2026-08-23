(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The WhyML-derived compiler pass (S36): the authorization gate extracted
   from `authorize_gate.mlw` (produced by the unfer probability kernel's
   `uk_whyml_emit` and machine-checked by Why3) enforced as a compiler pass.
   The pass rejects any compiled module whose imported `uk_*`/`uz_*` foreign
   externals are not covered by the module's grant set. By the WhyML
   postcondition (soundness), the gate never returns 0 for a missing grant —
   so the compiler can never emit a module that calls a kernel symbol it is
   not granted, closing the loop between the JIT's authorization and the
   compiler that produces the JIT input.

   Grant source: `AUSTRAL_UK_GRANTS` (comma-separated `uk_*` symbol names).
   This stands in for the full module-manifest plumbing (which lives in the
   JIT/modhost layer) so the pass is opt-in today: with the variable unset
   the pass is a no-op. Threading the real `module.toml` manifest into the
   compiler is the documented extension point (see
   `unfer/docs/WHYML_CYCLE.md`).
*)

open Compiler_plugin

let env_grants_var = "AUSTRAL_UK_GRANTS"

let grants_from_env () : string list =
  match Sys.getenv_opt env_grants_var with
  | None -> []
  | Some s ->
      String.split_on_char ',' s
      |> List.map String.trim
      |> List.filter (fun s -> s <> "")

(** Hash-cons symbol names to unique ints (the WhyML gate works on `int`
    symbol ids — `list int` in `authorize_gate.mlw`). The map is injective
    within the process, so `required ⊆ grants` as strings holds iff it holds
    on the encoded int lists: List.mem on ids mirrors List.mem on names, and
    the Why3-verified `gate_verdict` therefore decides the string-level
    question exactly. (The `.mlw` header notes the kernel-external direction
    stays out of the verified fragment; this encoding is the bridge.) *)
let sym_id : (string, int) Hashtbl.t = Hashtbl.create 64
let sym_ids : int ref = ref 0

let id_of_symbol s =
  match Hashtbl.find_opt sym_id s with
  | Some i -> i
  | None ->
      incr sym_ids;
      Hashtbl.add sym_id s !sym_ids;
      !sym_ids

(** The gate check core: `required ⊆ grants` decides. `foreign_externals`
    are the module's imported `uk_*`/`uz_*` symbols (from the typed AST);
    `grants` are the caller's grant set. When either side is empty the pass
    is a no-op (no kernel surface to check). Pure — no environment access —
    so tests can exercise every branch without mutating the process env
    (ounit2 flags env changes between tests; OCaml 4.14 has no
    `Unix.unsetenv`). *)
let check_with_grants ~grants ~module_name ~foreign_externals ~constants:_ : verdict =
  if foreign_externals = [] || grants = [] then
    VerdictOk
  else
    let grants_ids = List.map id_of_symbol grants in
    let required_ids = List.map id_of_symbol foreign_externals in
    match Authorize_gate.gate_verdict grants_ids required_ids with
    | 0 ->
        VerdictOk
    | _ ->
        let missing =
          List.filter (fun f -> not (List.mem f grants)) foreign_externals
        in
        VerdictReject
          (Printf.sprintf
             "module %s imports kernel symbol(s) [%s] that are not granted \
              (set %s=<comma-separated uk_* symbols>; the WhyML-verified gate \
              guarantees no missing grant is accepted)"
             module_name (String.concat ", " missing) env_grants_var)

(** The gate check as registered: grants come from the
    environment/manifest. Thin wrapper over [check_with_grants]. *)
let check ~module_name ~foreign_externals ~constants : verdict =
  check_with_grants ~grants:(grants_from_env ()) ~module_name ~foreign_externals
    ~constants

(** Register the pass (idempotent). Called from `Compiler.empty_compiler`. *)
let install () =
  if List.mem "why3_gate" (list_registered ()) then
    ()
  else
    register ~name:"why3_gate" check
