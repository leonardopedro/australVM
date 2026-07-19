open OUnit2
open CpsGen
open CamlCompiler_rust_bridge

(* ── helpers ────────────────────────────────────────────────────── *)

let init () =
  if not (initialize ()) then
    failwith "JIT bridge initialize() failed"

let compile_and_execute body expected =
  init ();
  let f = {
    name = "run";
    params = [];
    return_type = I64;
    body = body
  } in
  let bin = serialize_functions [f] in
  let (ptr, err) = compile_binary bin in
  if ptr = Int64.zero then
    failwith ("JIT compile failed: " ^ (match err with Some s -> s | None -> "unknown"))
  else
    assert_equal ~printer:Int64.to_string expected (execute_function ptr)

let compile_and_execute_1 body arg expected =
  init ();
  let f = {
    name = "run";
    params = ["n"];
    return_type = I64;
    body = body
  } in
  let bin = serialize_functions [f] in
  let (ptr, err) = compile_binary bin in
  if ptr = Int64.zero then
    failwith ("JIT compile failed: " ^ (match err with Some s -> s | None -> "unknown"))
  else
    assert_equal ~printer:Int64.to_string expected (execute_function_1 ptr arg)

let compile_and_execute_2 body arg1 arg2 expected =
  init ();
  let f = {
    name = "run";
    params = ["n"; "acc"];
    return_type = I64;
    body = body
  } in
  let bin = serialize_functions [f] in
  let (ptr, err) = compile_binary bin in
  if ptr = Int64.zero then
    failwith ("JIT compile failed: " ^ (match err with Some s -> s | None -> "unknown"))
  else
    assert_equal ~printer:Int64.to_string expected (execute_function_2 ptr arg1 arg2)

(* ── tests ──────────────────────────────────────────────────────── *)

let test_return_42 _ =
  compile_and_execute (Return (IntLit 42L)) 42L

let test_add _ =
  compile_and_execute (Return (Add (IntLit 10L, IntLit 32L))) 42L

let test_factorial _ =
  (* Self-recursive factorial: fact(n) = if n<=1 then 1 else n*fact(n-1) *)
  compile_and_execute_1
    (If (
       CmpLte (Var "n", IntLit 1L),
       Return (IntLit 1L),
       Return (Mul (Var "n", App ("run", [Sub (Var "n", IntLit 1L)])))
     ))
    5L 120L

let test_sum_1_to_n _ =
  compile_and_execute_1
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
  init ();
  let f = {
    name = "run";
    params = ["x"];
    return_type = I64;
    body = Match (
      Var "x",
      [(1L, Return (IntLit 100L)); (2L, Return (IntLit 200L))],
      Return (IntLit 300L)
    )
  } in
  let bin = serialize_functions [f] in
  let (ptr, err) = compile_binary bin in
  if ptr = Int64.zero then
    failwith ("JIT compile failed: " ^ (match err with Some s -> s | None -> "unknown"))
  else begin
    assert_equal ~printer:Int64.to_string 100L (execute_function_1 ptr 1L);
    assert_equal ~printer:Int64.to_string 200L (execute_function_1 ptr 2L);
    assert_equal ~printer:Int64.to_string 300L (execute_function_1 ptr 3L)
  end

let test_tail_rec_sum _ =
  (* Self-recursive: tail_rec_sum(n, acc) = if n=0 then acc else tail_rec_sum(n-1, acc+n) *)
  compile_and_execute_2
    (If (
       CmpEq (Var "n", IntLit 0L),
       Return (Var "acc"),
       Return (App ("run", [Sub (Var "n", IntLit 1L); Add (Var "acc", Var "n")]))
     ))
    1000L 0L 500500L

(* ── suite ──────────────────────────────────────────────────────── *)

let suite =
  "JIT" >::: [
    "return_42" >:: test_return_42;
    "add_10_32" >:: test_add;
    "factorial_5" >:: test_factorial;
    "sum_1_to_n_10" >:: test_sum_1_to_n;
    "match_pick" >:: test_match;
    "tail_rec_sum_1000" >:: test_tail_rec_sum;
  ]

let _ = run_test_tt_main suite
