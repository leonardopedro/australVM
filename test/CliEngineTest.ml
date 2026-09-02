open OUnit2
open CliEngine

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

let suite =
  "CliEngine" >::: [
    "sentinel_with_reason_is_error" >:: test_sentinel_with_reason_is_error;
    "real_value_is_result" >:: test_real_value_is_result;
    "stale_error_ignored_for_real_value" >:: test_stale_error_ignored_for_real_value;
    "true_min_int_is_result" >:: test_true_min_int_is_result;
  ]

let _ = run_test_tt_main suite