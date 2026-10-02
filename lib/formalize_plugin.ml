(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The CNL formalization gate (Part 2 P5 of the unfer rewrite plan, §16): the
   ProofFlow NL→logos adaptation, as a compiler plugin.

   A module declares a controlled-natural-language lemma by binding it to a
   string constant whose name begins with `cnl_`:

       constant cnl_sees:   String = "Mary sees Bob"
       constant cnl_adds:   String = "John adds two three"

   This pass compiles each such sentence through the unfer `logos` pipeline —
   gate → CCG parse → CoreIR → linearity → interaction net → reduce → readback →
   SHA-256 — and rejects the module unless every sentence reduces to a **unique**
   normal form.

   That is the analogue of `Deltanet_plugin`, with one difference worth stating.
   The DeltaNets gate compares the compiler's own evaluation of a constant
   against the kernel's `uk_austral_unf` reduction, so it has a second opinion
   to disagree with. This gate has none: a sentence either denotes a term with a
   content address, or it is not a lemma. So instead of cross-checking a value
   it checks *identity* — the UNF hash is the node's identity (rewrite plan §14),
   which means two `cnl_` constants reducing to the same hash are the same node,
   and a module declaring both is making a mistake. That is a check the
   DeltaNets gate has no reason to make, and it catches a real class of bug:
   copy-pasting a lemma and renaming the constant.

   Three properties are carried over from `Deltanet_plugin` deliberately:

   - **Opt-in** via `UNFER_LOGOS=1`, so the compiler keeps working outside the
     unfer checkout.
   - **No-op when the kernel is unavailable.** `Deltanet_plugin` cannot tell an
     unavailable bridge from a failing one, so it consults the bridge's
     last-error channel. A subprocess *can* be told apart — the binary either
     exists or it does not — so the no-op is decided by the filesystem and every
     other outcome rejects. Passing silently in the second case would hide the
     exact kernel↔compiler inconsistency this gate exists to catch.
   - **A protocol mismatch rejects.** Output that is not a `LogosReport` is not
     evidence of anything, and must not be read as agreement.

   The binary comes from `$UNFER_LOGOS_BIN`, else `logos` on `$PATH`.
*)

open Compiler_plugin
open Stages.Tast

let enabled_var = "UNFER_LOGOS"

(* String constants whose name carries this prefix are CNL declarations. A bare
   marker, not a naming convention: the pass must never reject a module for an
   ordinary string constant, so the surface is opt-in per constant. *)
let cnl_prefix = "cnl_"

let enabled () =
  match Sys.getenv_opt enabled_var with Some "1" -> true | _ -> false

(* ── locating the kernel ───────────────────────────────────────────── *)

(* `$UNFER_LOGOS_BIN` wins over `$PATH`, so a checkout can be pointed at
   explicitly without touching the environment the compiler inherits.

   `None` means "no kernel available", and that is the single condition under
   which this gate is a no-op — so it is answered from the filesystem, before any
   declaration is examined.

   Both branches *check that the file exists*, and that check is not optional
   politeness: it is the entire no-op-versus-reject split. An `UNFER_LOGOS_BIN`
   pointing at a stale path would otherwise look like "a kernel that ran and
   failed", and every module would be rejected for a reason that has nothing to
   do with the module. `test_absent_binary_is_noop_not_rejection` covers exactly
   that, because it is the bug this check prevents. *)
(* The same decision, with both inputs supplied rather than read from the
   environment — so the "explicit path that is not there" case is testable
   without mutating the process environment, which OUnit2 (rightly) fails a test
   for. `kernel_binary` is this, fed from the environment. *)
let kernel_binary_with ~(explicit : string option) ~(path : string option) :
    string option =
  let is_regular f =
    match Unix.stat f with
    | { Unix.st_kind = Unix.S_REG; _ } -> true
    | _ -> false
    | exception _ -> false
  in
  let usable p = if String.length p > 0 && is_regular p then Some p else None in
  match explicit with
  | Some b -> (
      (* An explicit path that is not there is *not* a reason to search the
         PATH: the operator asked for this binary, and silently substituting
         another one would verify against an engine they did not choose. *)
      match usable b with Some _ as ok -> ok | None -> None)
  | None -> (
      match path with
      | None -> None
      | Some path ->
          let found =
            List.filter_map
              (fun dir ->
                let dir = if String.length dir = 0 then "." else dir in
                usable (Filename.concat dir "logos"))
              (String.split_on_char ':' path)
          in
          match found with first :: _ -> Some first | [] -> None)

let kernel_binary () : string option =
  kernel_binary_with
    ~explicit:(Sys.getenv_opt "UNFER_LOGOS_BIN")
    ~path:(Sys.getenv_opt "PATH")

(* ── asking the kernel ────────────────────────────────────────────── *)

type kernel_report = { result : string; unf_hash : string; verified : bool }

(* Run `logos unf <sentence> --json` and parse the reply.

   `Ok None` is the no-op case — the binary disappeared between the probe and
   the call. `Ok (Some …)` means the kernel answered with a well-formed
   `LogosReport`. Everything else is `Error`: "the kernel ran and I could not
   understand it" is not agreement, and neither is "it printed nothing".

   **stderr is captured too**, and it matters: `logos unf` writes its reason
   (`words not in the lexicon: …`) to stderr, so without this the gate could say
   only that the sentence did not compile — which sends the author looking for a
   parse problem instead of at the one word outside the lexicon.

   The two streams are drained sequentially rather than concurrently, which is
   correct as long as stderr stays small. It does: the kernel emits one line.
   A future kernel that wrote a large diagnostic would need a select loop here,
   and the deadlock would be a hang in the compiler rather than a wrong answer —
   which is the better of the two failure modes, but still worth stating.

   The sentence goes through `Filename.quote` because `open_process_full` hands
   the command to `sh`. The quote is the reason this is safe, and the test suite
   checks that a sentence containing a quote and a semicolon survives. *)
let ask_kernel ~(binary : string) (sentence : string) :
    (kernel_report option, string) result =
  let cmd =
    Printf.sprintf "%s unf %s --json" binary (Filename.quote sentence)
  in
  let read_all ic =
    let buf = Buffer.create 256 in
    (try
       while true do
         Buffer.add_string buf (input_line ic);
         Buffer.add_char buf '\n'
       done
     with End_of_file -> ());
    Buffer.contents buf
  in
  let ic, oc, ec =
    Unix.open_process_full cmd (Unix.environment ())
  in
  let out = read_all ic in
  let err = read_all ec in
  let status = Unix.close_process_full (ic, oc, ec) in
  let err = String.trim err in
  let reason =
    if String.length err > 0 then Printf.sprintf ": %s" err else ""
  in
  match status with
  | Unix.WEXITED 0 -> (
      let text = String.trim out in
      try
        let json = Yojson.Safe.from_string text in
        let open Yojson.Safe.Util in
        let result = member "result" json |> to_string in
        let unf_hash = member "unf_hash" json |> to_string in
        let verified = member "verified" json |> to_bool in
        if String.length unf_hash <> 64 then
          Error
            (Printf.sprintf
               "kernel returned a %d-character unf_hash, expected a 64-character \
                SHA-256"
               (String.length unf_hash))
        else Ok (Some { result; unf_hash; verified })
      with _ ->
        let head = String.sub text 0 (min 80 (String.length text)) in
        Error
          (Printf.sprintf
             "kernel produced output that is not a LogosReport (first 80 \
              characters: %s)"
             head))
  | Unix.WEXITED n ->
      (* `logos unf` distinguishes two failures by exit code: 1 is a rejected
         sentence, 2 is one that compiled without a unique normal form. Both are
         rejections; the message says which, because the fixes differ, and
         carries the kernel's own reason. *)
      Error
        (Printf.sprintf "logos unf exited %d (%s)%s"
           n
           (if n = 1 then "the sentence did not compile"
            else if n = 2 then
              "it compiled but two reductions disagreed"
            else "unexpected exit code")
           reason)
  | Unix.WSIGNALED s ->
      Error (Printf.sprintf "logos was killed by signal %d%s" s reason)
  | Unix.WSTOPPED s ->
      Error (Printf.sprintf "logos was stopped by signal %d%s" s reason)

(* ── what a constant declares ──────────────────────────────────────── *)

(* The CNL sentence a constant declares, if it declares one.

   `Escape.escaped_to_string`, **not** `unescape_string`. The latter is the
   printer direction: it re-inserts C-format escape sequences for quotes and
   newlines, so a sentence containing a quote would reach the kernel with stray
   backslashes and be rejected for a reason that has nothing to do with CNL —
   the exact confusion `Compiler_cps.ml:138` warns about for the embedded
   buffer. `escaped_to_string` is what yields the real processed bytes.

   `None` for every constant that is not a `cnl_` string, which is what makes
   this a no-op on ordinary modules. *)
let cnl_of (id : Identifier.identifier) (init : texpr) : string option =
  let name = String.lowercase_ascii (Identifier.ident_string id) in
  if not (String.starts_with ~prefix:cnl_prefix name) then None
  else
    match init with
    | TStringConstant s -> Some (Escape.escaped_to_string s)
    | _ -> None

(* A `cnl_`-prefixed constant that is not a string is a mistake in the module,
   not something to ignore: the prefix is a declaration that a lemma follows, and
   a declaration that does not hold should not compile quietly. *)
let verdict_on_non_string ~(module_name : string) (id : Identifier.identifier) : verdict =
  VerdictReject
    (Printf.sprintf
       "module %s constant `%s` is named like a CNL declaration but is not a \
        string; a `cnl_` constant must hold exactly one L0 CNL sentence"
       module_name (Identifier.ident_string id))

let is_cnl_named (id : Identifier.identifier) : bool =
  String.starts_with ~prefix:cnl_prefix
    (String.lowercase_ascii (Identifier.ident_string id))

(* ── the two checks ────────────────────────────────────────────────── *)

let check_declaration ~(module_name : string) ~(binary : string)
    ((id : Identifier.identifier), (init : texpr)) : verdict =
  let name = Identifier.ident_string id in
  if not (is_cnl_named id) then VerdictOk
  else
    match init with
    | TStringConstant _ -> (
        match cnl_of id init with
        | None -> VerdictOk
        | Some sentence -> (
            match ask_kernel ~binary sentence with
            | Ok None -> VerdictOk (* the binary vanished mid-run: no-op *)
            | Ok (Some r) ->
                if r.verified then VerdictOk
                else
                  VerdictReject
                    (Printf.sprintf
                       "module %s constant `%s`: CNL %S reduced to %s but two \
                        reductions disagreed, so it has no unique normal form \
                        and therefore no content address (UNF %s)"
                       module_name name sentence r.result r.unf_hash)
            | Error msg ->
                VerdictReject
                  (Printf.sprintf
                     "module %s constant `%s`: the CNL gate could not verify %S \
                      via the kernel (%s); not passing silently"
                     module_name name sentence msg)))
    | _ -> verdict_on_non_string ~module_name id

(* §14: two constants with one UNF hash are one node. Reporting *both* names is
   the point — the author can then decide which to delete, or which one was
   meant to differ. *)
let check_distinct_hashes ~(module_name : string) ~(binary : string)
    (sentences : (Identifier.identifier * string) list) : verdict =
  let table : (string, string) Hashtbl.t = Hashtbl.create 16 in
  let rec go = function
    | [] -> VerdictOk
    | (id, sentence) :: rest -> (
        match ask_kernel ~binary sentence with
        | Ok (Some r) -> (
            match Hashtbl.find_opt table r.unf_hash with
            | Some previous ->
                VerdictReject
                  (Printf.sprintf
                     "module %s: constants `%s` and `%s` both reduce to UNF \
                      %s, so they denote the same node; a node's identity is its \
                      normal form, and declaring it twice is a mistake"
                     module_name previous (Identifier.ident_string id) r.unf_hash)
            | None ->
                Hashtbl.replace table r.unf_hash (Identifier.ident_string id);
                go rest)
        | Ok None -> go rest
        | Error _ -> go rest (* already reported by check_declaration *)
    )
  in
  go sentences

(* ── the pass ──────────────────────────────────────────────────────── *)

(** The pass as registered, and the same logic with its two environmental inputs
    supplied explicitly.

    Both checks run, declaration-checked first: a bad sentence is the more
    fundamental fault, and reporting it is more useful than reporting that two
    constants collided when one of them does not even parse.

    The first rejection wins, matching `Deltanet_plugin.check`.

    Splitting the environment out is what makes the gate testable without
    mutating the process environment — which OUnit2 (rightly) fails a test for. *)
let check_with ~(enabled : bool) ~(binary : string option) ~(module_name : string)
    ~(constants : (Identifier.identifier * texpr) list) : verdict =
  if not enabled then VerdictOk
  else
    match binary with
    | None ->
        (* Documented no-op: no `logos` available, so the compiler keeps working
           outside the unfer checkout. Never a rejection — see the header. *)
        VerdictOk
    | Some binary ->
        let first_rejection =
          List.fold_left
            (fun acc (id, init) ->
              match acc with
              | Some _ -> acc
              | None -> (
                  match check_declaration ~module_name ~binary (id, init) with
                  | VerdictOk -> None
                  | VerdictReject msg -> Some msg))
            None constants
        in
        (match first_rejection with
        | Some msg -> VerdictReject msg
        | None ->
            let sentences =
              List.filter_map
                (fun (id, init) -> cnl_of id init |> Option.map (fun s -> (id, s)))
                constants
            in
            check_distinct_hashes ~module_name ~binary sentences)

let check ~module_name ~foreign_externals:_ ~constants : verdict =
  check_with ~enabled:(enabled ()) ~binary:(kernel_binary ()) ~module_name
    ~constants

(* ── registration ──────────────────────────────────────────────────── *)

(** Register the pass (idempotent). Called from `Vm_plugin.boot`.

    Idempotent by name, like `Deltanet_plugin.install`: `boot` runs again after a
    `Compiler_plugin.reset`, and a second registration of the same name would make
    the gate run twice over every module. *)
let install () =
  if List.mem "logos_cnl" (list_registered ()) then ()
  else register ~name:"logos_cnl" check