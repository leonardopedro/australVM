open OUnit2
open CliEngine
open Error

(* The REPL "call" command renders an execute_function result through
   format_exec_result. The Rust bridge returns the JIT_PANIC sentinel
   (i64::MIN) and records the reason on the last-error channel when the
   JIT-compiled function panicked; the REPL must surface that as an ERROR,
   never as a misleading RESULT value. These tests pin that contract — the
   Rust-side panic_guard_tests verify the sentinel itself, this side verifies
   the rendering. *)

let test_sentinel_with_reason_is_error _ =
  assert_equal
    ~printer:(fun s -> s)
    "ERROR JIT-compiled function panicked"
    (format_exec_result Int64.min_int (Some "JIT-compiled function panicked"))

let test_real_value_is_result _ =
  assert_equal
    ~printer:(fun s -> s)
    "RESULT 42"
    (format_exec_result 42L None)

(* A stale last-error must never corrupt a real result. *)
let test_stale_error_ignored_for_real_value _ =
  assert_equal
    ~printer:(fun s -> s)
    "RESULT 7"
    (format_exec_result 7L (Some "stale error from an earlier op"))

(* A true i64::MIN result (no recorded error) still prints as RESULT. *)
let test_true_min_int_is_result _ =
  assert_equal
    ~printer:(fun s -> s)
    ("RESULT " ^ Int64.to_string Int64.min_int)
    (format_exec_result Int64.min_int None)

(* The JIT-server protocol handler must survive a command that throws.
   Pre-fix, an exception while handling one line (e.g. a swap of a
   nonexistent module) escaped to the loop's outer handler and KILLED the
   server with no stdout response — the driving script was left waiting on a
   response that never arrived. `handle_server_line` reports `ERROR <reason>`
   on the protocol channel and returns, so the loop keeps serving. *)

let test_server_line_bad_swap_reports_not_raises _ =
  (* A swap of a nonexistent module raises during file parsing; the handler
     must report it and return — never propagate, never exit. *)
  CliEngine.handle_server_line "swap /nonexistent/mod.aum";
  (* Unrecognized / malformed lines report without raising either. *)
  CliEngine.handle_server_line "garbage input here";
  CliEngine.handle_server_line "call missing_fn";
  CliEngine.handle_server_line "";
  (* Reaching this point proves the server loop would still be alive. *)
  ()

(* The `compile` help is prompt text: it must list every shipped flag and
   only the target values CliParser actually accepts. Pre-fix the help
   advertised `--target-type One of \`bin\`, \`tc\`, \`c\`` (the parser
   rejects `bin` and accepts `exe`) and never mentioned `--jit-server`,
   `--allow-all`, or `--auth-manifest` — shipped capabilities that were
   undiscoverable from the tool's own text. *)
let test_compile_help_lists_shipped_flags_and_real_targets _ =
  let lines = CliEngine.compile_help_lines () in
  let joined = String.concat "\n" lines in
  let contains needle =
    let re = Str.regexp_string needle in
    Str.string_match re joined 0
    || (try ignore (Str.search_forward re joined 0); true
        with Not_found -> false)
  in
  (* The real target vocabulary, not the parser-rejected `bin`. *)
  assert_bool "help must advertise `exe` as the default target"
    (contains "One of `exe`, `tc`, `c`. Default is `exe`.");
  assert_bool "help must not advertise the parser-rejected `bin` value"
    (not (contains "`bin`"));
  (* Every shipped flag is discoverable. *)
  List.iter
    (fun flag ->
       assert_bool
         (Printf.sprintf "help must mention --%s" flag)
         (contains (Printf.sprintf "--%s" flag)))
    ["jit-server"; "allow-all"; "auth-manifest"; "use-cps-jit";
     "no-entrypoint"; "emit-cps"; "target-type"]

(* `--jit-server` on any target but `tc` is silently ignored pre-fix: the
   compile produces an artifact, the stdin `call|swap|exit` protocol never
   starts, and the driving script is left waiting on a READY line that never
   comes. The validator must refuse loudly and name the fix. *)
let test_jit_server_requires_tc_target _ =
  let exe_target =
    CliParser.Executable {
        bin_path = "x";
        entrypoint =
          CliParser.Entrypoint
            (Identifier.make_mod_name "Test", Identifier.make_ident "main");
      }
  in
  (* Fine on the tc target. *)
  CliEngine.validate_jit_server_target true CliParser.TypeCheck;
  (* Refused on the executable target: raises Austral_error naming the fix. *)
  let message =
    try
      CliEngine.validate_jit_server_target true exe_target;
      ""
    with
    | Austral_error (AustralError { text = Text s :: _; _ }) -> s
    | Austral_error _ -> ""
  in
  assert_bool "exe target + --jit-server must refuse loudly" (message <> "");
  assert_bool "error must name the fix"
    (let re = Str.regexp_string "--target-type=tc" in
     try ignore (Str.search_forward re message 0); true
     with Not_found -> false);
  (* No jit-server requested: any target is fine. *)
  CliEngine.validate_jit_server_target false exe_target

let suite =
  "CliEngine" >::: [
    "sentinel_with_reason_is_error" >:: test_sentinel_with_reason_is_error;
    "real_value_is_result" >:: test_real_value_is_result;
    "stale_error_ignored_for_real_value" >:: test_stale_error_ignored_for_real_value;
    "true_min_int_is_result" >:: test_true_min_int_is_result;
    "server_line_bad_swap_reports_not_raises" >:: test_server_line_bad_swap_reports_not_raises;
    "compile_help_lists_shipped_flags_and_real_targets"
    >:: test_compile_help_lists_shipped_flags_and_real_targets;
    "jit_server_requires_tc_target" >:: test_jit_server_requires_tc_target;
  ]

let _ = run_test_tt_main suite