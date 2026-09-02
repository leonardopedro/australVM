open OUnit2
open CpsGen
open CamlCompiler_rust_bridge

(* ── helpers ────────────────────────────────────────────────────── *)

let init () =
  if not (initialize ()) then
    failwith "JIT bridge initialize() failed"

(* Every test compiles into the SAME JIT module, so each function must have
   a unique name: redefining a name with a different signature (or at all)
   fails the whole compile (`DuplicateDefinition` / `IncompatibleSignature`).
   The `fname` argument keeps definitions distinct across tests. *)
let compile_named fname params body =
  init ();
  let f = {
    name = fname;
    params = params;
    return_type = I64;
    body = body
  } in
  let bin = serialize_functions [f] in
  let (ptr, err) = compile_binary bin in
  if ptr = Int64.zero then
    failwith ("JIT compile failed: " ^ (match err with Some s -> s | None -> "unknown"))
  else
    ptr

let compile_and_execute fname body expected =
  let ptr = compile_named fname [] body in
  assert_equal ~printer:Int64.to_string expected (execute_function ptr)

let compile_and_execute_1 fname body arg expected =
  let ptr = compile_named fname ["n"] body in
  assert_equal ~printer:Int64.to_string expected (execute_function_1 ptr arg)

let compile_and_execute_2 fname body arg1 arg2 expected =
  let ptr = compile_named fname ["n"; "acc"] body in
  assert_equal ~printer:Int64.to_string expected (execute_function_2 ptr arg1 arg2)

(* ── tests ──────────────────────────────────────────────────────── *)

let test_return_42 _ =
  compile_and_execute "run_0" (Return (IntLit 42L)) 42L

let test_add _ =
  compile_and_execute "run_add" (Return (Add (IntLit 10L, IntLit 32L))) 42L

let test_factorial _ =
  (* Self-recursive factorial: fact(n) = if n<=1 then 1 else n*fact(n-1) *)
  compile_and_execute_1 "run_fact"
    (If (
       CmpLte (Var "n", IntLit 1L),
       Return (IntLit 1L),
       Return (Mul (Var "n", App ("run_fact", [Sub (Var "n", IntLit 1L)])))
     ))
    5L 120L

let test_sum_1_to_n _ =
  compile_and_execute_1 "run_sum"
    (Block (
       Let ("s", IntLit 0L, Skip),
       Block (
         While (
           CmpGt (Var "n", IntLit 0L),
           Block (
             Assign ("s", Add (Var "s", Var "n")),
             Assign ("n", Sub (Var "n", IntLit 1L))
           )
         ),
         Return (Var "s")
       )
     ))
    10L 55L

let test_match _ =
  let ptr = compile_named "run_match" ["x"] (Match (
    Var "x",
    [(1L, Return (IntLit 100L)); (2L, Return (IntLit 200L))],
    Return (IntLit 300L)
  )) in
  assert_equal ~printer:Int64.to_string 100L (execute_function_1 ptr 1L);
  assert_equal ~printer:Int64.to_string 200L (execute_function_1 ptr 2L);
  assert_equal ~printer:Int64.to_string 300L (execute_function_1 ptr 3L)

let test_tail_rec_sum _ =
  (* Self-recursive: tail_rec_sum(n, acc) = if n=0 then acc else tail_rec_sum(n-1, acc+n) *)
  compile_and_execute_2 "run_tail"
    (If (
       CmpEq (Var "n", IntLit 0L),
       Return (Var "acc"),
       Return (App ("run_tail", [Sub (Var "n", IntLit 1L); Add (Var "acc", Var "n")]))
     ))
    1000L 0L 500500L

(* The panic guard's null-pointer branch is reachable from OCaml: a zero
   pointer returns -1 through the guard (fail-visible) instead of crashing.
   The Rust-side panic_guard_tests cover the JIT_PANIC sentinel itself; this
   smoke test pins the end-to-end contract at the bridge boundary. *)
let test_null_ptr_execute_returns_neg_one _ =
  init ();
  assert_equal ~printer:Int64.to_string (-1L) (execute_function 0L);
  assert_equal ~printer:Int64.to_string (-1L) (execute_function_1 0L 1L);
  assert_equal ~printer:Int64.to_string (-1L) (execute_function_2 0L 1L 2L)

(* ── suite ──────────────────────────────────────────────────────── *)

let suite =
  "JIT" >::: [
    "return_42" >:: test_return_42;
    "add_10_32" >:: test_add;
    "factorial_5" >:: test_factorial;
    "sum_1_to_n_10" >:: test_sum_1_to_n;
    "match_pick" >:: test_match;
    "tail_rec_sum_1000" >:: test_tail_rec_sum;
    "null_ptr_execute_returns_neg_one" >:: test_null_ptr_execute_returns_neg_one;
  ]

let _ = run_test_tt_main suite
