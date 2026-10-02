(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The CNL formalization gate: `Formalize_plugin` compiles every `cnl_` string
   constant through the unfer `logos` pipeline and rejects the module unless each
   reduces to a unique normal form and no two share one.

   The suite is in two halves. The first needs no external binary — the
   declaration surface, enablement, registration, and the two rejection paths
   that do not depend on what the kernel says — and runs everywhere. The second
   exercises the real round trip and runs only when a `logos` binary is reachable,
   found by the same rules the plugin uses, so pointing the suite at a checkout is
   a matter of putting one directory on `$PATH`.

   Nothing here mutates the process environment: OUnit2 snapshots it per test and
   fails any test that changes it. Every case therefore goes through
   `Formalize_plugin.check_with`, whose two environmental inputs — whether the gate
   is enabled, and which kernel to call — are supplied explicitly. That is also
   the better factoring: the registered `check` is a one-liner over it.
*)
open OUnit2
open Compiler_plugin
open TestUtil

(* ── fixtures ──────────────────────────────────────────────────────── *)

(* Austral stores string literals escaped, so a fixture goes in through
   `Escape.escape_string` — the same shape a module's source produces. *)
let str_const s = Stages.Tast.TStringConstant (Escape.escape_string s)

let ident s = Identifier.make_ident s

(* Substring containment, defined locally as `DeltanetPluginTest` does — the
   rejection messages are the assertion surface here, and reading them is most of
   what these tests check. *)
let contains_substring haystack needle =
  try
    ignore (Str.search_forward (Str.regexp_string needle) haystack 0);
    true
  with Not_found -> false

(* The same rules the plugin uses to find the kernel. *)
let kernel () = Formalize_plugin.kernel_binary ()

let no_kernel () =
  print_endline "  (skipped: no logos binary on PATH or in UNFER_LOGOS_BIN)"

(* Run the gate over `constants` with the gate enabled and the real kernel. *)
let run constants =
  Formalize_plugin.check_with ~enabled:true ~binary:(kernel ())
    ~module_name:"m" ~constants

(* ── the declaration surface ──────────────────────────────────────── *)

(* Only `cnl_`-prefixed string constants are declarations. Everything else must
   be invisible to the gate, or an ordinary module would stop compiling. *)
let test_cnl_of_selects_only_prefixed_strings _ =
  (match
     Formalize_plugin.cnl_of (ident "cnl_sees") (str_const "Mary sees Bob")
   with
   | Some s -> eq "Mary sees Bob" s
   | None -> assert_failure "a cnl_ string constant is a declaration");
  eq None (Formalize_plugin.cnl_of (ident "greeting") (str_const "hello"));
  eq None
    (Formalize_plugin.cnl_of (ident "cnl_count")
       (Stages.Tast.TIntConstant "3"));
  (match
     Formalize_plugin.cnl_of (ident "CNL_Sees") (str_const "Mary sees Bob")
   with
   | Some _ -> ()
   | None -> assert_failure "the prefix check should ignore case")

(* `cnl_of` must return the real bytes. `Escape.unescape_string` is the *printer*
   direction — it re-inserts escape sequences for quotes and newlines — so using
   it would send a sentence containing a quote to the kernel with stray
   backslashes, and it would be rejected for a reason that has nothing to do with
   CNL. This is the confusion `Compiler_cps.ml:138` warns about for the embedded
   buffer. *)
let test_cnl_of_yields_the_real_bytes _ =
  let quoted = "John says \"hi\"" in
  (match Formalize_plugin.cnl_of (ident "cnl_q") (str_const quoted) with
   | Some s ->
       assert_bool "no backslash was introduced"
         (not (contains_substring s "\\"))
   | None -> assert_failure "expected a declaration")

let test_is_cnl_named _ =
  assert_bool "cnl_x is cnl-named"
    (Formalize_plugin.is_cnl_named (ident "cnl_x"));
  assert_bool "CNL_X is cnl-named"
    (Formalize_plugin.is_cnl_named (ident "CNL_X"));
  assert_bool "x is not" (not (Formalize_plugin.is_cnl_named (ident "x")))

(* ── registration ─────────────────────────────────────────────────── *)

let test_pass_registers_and_is_idempotent _ =
  Compiler_plugin.reset ();
  Formalize_plugin.install ();
  Formalize_plugin.install ();
  assert_bool "logos_cnl registered"
    (List.mem "logos_cnl" (list_registered ()));
  (* A second registration would run the gate twice over every module. *)
  eq 1
    (List.length
       (List.filter (fun n -> n = "logos_cnl") (list_registered ())));
  Compiler_plugin.reset ()

(* ── enablement ───────────────────────────────────────────────────── *)

(* Disabled by default: that is what makes the gate adoptable, since a checkout
   without `UNFER_LOGOS=1` must still compile. Exercised with a constant that
   *would* be rejected if enabled. *)
let test_disabled_is_a_noop _ =
  let v =
    Formalize_plugin.check_with ~enabled:false
      ~binary:(Some "/bin/sh")
      ~module_name:"m"
      ~constants:[ (ident "cnl_bad", Stages.Tast.TIntConstant "3") ]
  in
  eq VerdictOk v

(* A `cnl_` constant that is not a string is a rejection saying what is wrong,
   not a generic "invalid CNL" and not a silent pass. *)
let test_non_string_declaration_rejects _ =
  match
    Formalize_plugin.verdict_on_non_string ~module_name:"m" (ident "cnl_n")
  with
  | VerdictOk -> assert_failure "a non-string cnl_ constant must reject"
  | VerdictReject msg ->
      assert_bool "names the module" (contains_substring msg "module m");
      assert_bool "names the constant" (contains_substring msg "cnl_n");
      assert_bool "says what is wrong"
        (contains_substring msg "must hold exactly one L0")

(* ── the no-op / reject distinction ────────────────────────────────── *)

(* The kernel's availability is decided by *finding* the binary, so a path that
   does not exist means "unavailable" and the gate stands down.

   This is the case that made `kernel_binary` check its input: an explicit
   `UNFER_LOGOS_BIN` that is not there used to read as "a kernel that ran and
   failed", and every module was rejected for a reason that had nothing to do
   with the module. *)
let test_absent_binary_is_noop_not_rejection _ =
  let binary =
    Formalize_plugin.kernel_binary_with
      ~explicit:(Some "/nonexistent/logos")
      ~path:(Some "/nonexistent")
  in
  eq None binary;
  let v =
    Formalize_plugin.check_with ~enabled:true ~binary
      ~module_name:"m"
      ~constants:[ (ident "cnl_bad", str_const "Euler proves congruences") ]
  in
  eq VerdictOk v

(* An explicit path is not replaced by a PATH lookup when it is missing: the
   operator asked for that engine, and substituting another one would verify
   against something they did not choose. *)
let test_absent_explicit_path_does_not_fall_back_to_path _ =
  eq None
    (Formalize_plugin.kernel_binary_with
       ~explicit:(Some "/nonexistent/logos")
       ~path:(Some "/bin:/usr/bin"))

(* A binary that exists but is not a working `logos` must *reject*. This is the
   distinction `Deltanet_plugin` cannot make — it cannot tell an unavailable
   bridge from a failing one, so it consults a last-error channel — and getting
   it wrong here would let a broken kernel read as an absent one, which is
   exactly the silent-pass failure mode that gate's header warns about. *)
let test_broken_binary_rejects _ =
  let script = Filename.temp_file "fake_logos" ".sh" in
  let oc = open_out script in
  output_string oc "#!/bin/sh\necho 'not json'\nexit 0\n";
  close_out oc;
  Unix.chmod script 0o755;
  Fun.protect
    ~finally:(fun () -> try Sys.remove script with _ -> ())
    (fun () ->
      let v =
        Formalize_plugin.check_with ~enabled:true ~binary:(Some script)
          ~module_name:"m"
          ~constants:[ (ident "cnl_x", str_const "Mary sees Bob") ]
      in
      match v with
      | VerdictOk ->
          assert_failure "a kernel that answers with garbage must not pass"
      | VerdictReject msg ->
          assert_bool "says it was not a LogosReport"
            (contains_substring msg "LogosReport");
          assert_bool "does not pass silently"
            (contains_substring msg "not passing silently"))

(* A kernel that answers with the right schema but a non-unique normal form must
   also reject: `logos unf` exits 2 for that case, and a gate that only looked at
   the exit code would wave it through. *)
let test_broken_binary_reporting_success_is_checked _ =
  let script = Filename.temp_file "fake_logos" ".sh" in
  let oc = open_out script in
  output_string oc
    (Printf.sprintf
       "#!/bin/sh\ncat <<'EOF'\n{\"result\":\"X(1)\",\"unf_hash\":\"%s\",\"verified\":false,\"sentence\":\"x\"}\nEOF\n"
       (String.make 64 'a'));
  close_out oc;
  Unix.chmod script 0o755;
  Fun.protect
    ~finally:(fun () -> try Sys.remove script with _ -> ())
    (fun () ->
      let v =
        Formalize_plugin.check_with ~enabled:true ~binary:(Some script)
          ~module_name:"m"
          ~constants:[ (ident "cnl_x", str_const "Mary sees Bob") ]
      in
      match v with
      | VerdictOk ->
          assert_failure "a non-unique normal form must not pass"
      | VerdictReject msg ->
          assert_bool "says there was no unique normal form"
            (contains_substring msg "unique normal form"))

(* ── kernel-dependent behaviour ────────────────────────────────────── *)

let test_accepts_a_verified_sentence _ =
  match kernel () with
  | None -> no_kernel ()
  | Some _ ->
      (match run [ (ident "cnl_sees", str_const "Mary sees Bob") ] with
      | VerdictOk -> ()
      | VerdictReject msg -> assert_failure ("must not reject: " ^ msg))

(* An out-of-lexicon sentence must be a rejection naming the constant and
   carrying the kernel's own reason — "invalid CNL" would send the author looking
   in the wrong place. *)
let test_rejects_an_unlexiconable_sentence _ =
  match kernel () with
  | None -> no_kernel ()
  | Some _ ->
      (match run [ (ident "cnl_bad", str_const "Euler proves congruences") ] with
      | VerdictOk -> assert_failure "an out-of-lexicon sentence must reject"
      | VerdictReject msg ->
          assert_bool "names the module" (contains_substring msg "module m");
          assert_bool "names the constant" (contains_substring msg "cnl_bad");
          assert_bool "carries the kernel's reason"
            (contains_substring msg "lexicon"))

(* Rewrite plan §14: two constants reducing to the same UNF are the same node, so
   the module is making a mistake. Both names are reported so the author can
   decide which to change. *)
let test_rejects_duplicate_identities _ =
  match kernel () with
  | None -> no_kernel ()
  | Some _ ->
      let constants =
        [
          (ident "cnl_a", str_const "Mary sees Bob");
          (ident "cnl_b", str_const "Mary sees Bob");
        ]
      in
      (match run constants with
      | VerdictOk -> assert_failure "two identical declarations must reject"
      | VerdictReject msg ->
          assert_bool "names both constants"
            (contains_substring msg "cnl_a" && contains_substring msg "cnl_b");
          assert_bool "explains why" (contains_substring msg "same node"))

(* The flip side: the collision check must not reject merely because two
   constants exist. *)
let test_distinct_sentences_do_not_collide _ =
  match kernel () with
  | None -> no_kernel ()
  | Some _ ->
      let constants =
        [
          (ident "cnl_a", str_const "Mary sees Bob");
          (ident "cnl_b", str_const "John runs");
        ]
      in
      (match run constants with
      | VerdictOk -> ()
      | VerdictReject msg ->
          assert_failure ("distinct sentences must not collide: " ^ msg))

let test_ordinary_constants_are_ignored _ =
  match kernel () with
  | None -> no_kernel ()
  | Some _ ->
      let constants =
        [
          (ident "greeting", str_const "hello");
          (ident "count", Stages.Tast.TIntConstant "3");
          (ident "cnl_ok", str_const "Mary sees Bob");
        ]
      in
      (match run constants with
      | VerdictOk -> ()
      | VerdictReject msg ->
          assert_failure ("only cnl_ constants matter: " ^ msg))

(* `ask_kernel` hands the command to `sh`, so the sentence must be quoted. This
   checks the quoting end to end: a sentence with `;` reaches the kernel whole
   instead of executing anything. *)
let test_shell_metacharacters_are_quoted _ =
  match kernel () with
  | None -> no_kernel ()
  | Some binary ->
      (match
         Formalize_plugin.ask_kernel ~binary "Mary sees Bob; echo pwned"
       with
      | Ok None -> ()
      | Ok (Some _) ->
          assert_failure
            "a sentence containing `;` must not compile, so it cannot have parsed"
      | Error msg ->
          (* The kernel saw the whole string: it either named the words as out of
             lexicon, or refused it, or failed to parse — never a successful
             reduction of something else. *)
          assert_bool "the whole sentence reached the kernel"
            (contains_substring msg "pwned"
            || contains_substring msg "lexicon"
            || contains_substring msg "exited"))

let suite =
  "FormalizePluginTest"
  >::: [
         "cnl_of_selects_only_prefixed_strings" >:: test_cnl_of_selects_only_prefixed_strings;
         "cnl_of_yields_the_real_bytes" >:: test_cnl_of_yields_the_real_bytes;
         "is_cnl_named" >:: test_is_cnl_named;
         "pass_registers_and_is_idempotent" >:: test_pass_registers_and_is_idempotent;
         "disabled_is_a_noop" >:: test_disabled_is_a_noop;
         "non_string_declaration_rejects" >:: test_non_string_declaration_rejects;
         "absent_binary_is_noop_not_rejection" >:: test_absent_binary_is_noop_not_rejection;
         "absent_explicit_path_does_not_fall_back_to_path" >:: test_absent_explicit_path_does_not_fall_back_to_path;
         "broken_binary_rejects" >:: test_broken_binary_rejects;
         "broken_binary_reporting_success_is_checked" >:: test_broken_binary_reporting_success_is_checked;
         "accepts_a_verified_sentence" >:: test_accepts_a_verified_sentence;
         "rejects_an_unlexiconable_sentence" >:: test_rejects_an_unlexiconable_sentence;
         "rejects_duplicate_identities" >:: test_rejects_duplicate_identities;
         "distinct_sentences_do_not_collide" >:: test_distinct_sentences_do_not_collide;
         "ordinary_constants_are_ignored" >:: test_ordinary_constants_are_ignored;
         "shell_metacharacters_are_quoted" >:: test_shell_metacharacters_are_quoted;
       ]

let () = run_test_tt_main suite