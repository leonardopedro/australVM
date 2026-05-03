open CpsGen
open CamlCompiler_rust_bridge

let () =
  Printf.printf "Starting JIT manual test...\n";
  
  (* 1. Initialize *)
  if not (initialize ()) then begin
    Printf.printf "Failed to initialize JIT bridge\n";
    exit 1
  end;
  
  (* 2. Test 1: Return 42 *)
  let f1 = {
    name = "return_42";
    params = [];
    return_type = I64;
    body = Return (IntLit 42L)
  } in
  
  let binary1 = serialize_functions [f1] in
  let (ptr, _err) = compile_binary binary1 in
  if ptr = Int64.zero then Printf.printf "Test 1: Compilation failed\n"
  else
      let res = execute_function ptr in
      Printf.printf "Test 1 (Return 42) result: %Ld\n" res;
      if res = 42L then Printf.printf "SUCCESS\n" else Printf.printf "FAILURE\n";
      
  (* 3. Test 2: Arithmetic *)
  let f2 = {
    name = "add_test";
    params = [];
    return_type = I64;
    body = Return (Add (IntLit 10L, IntLit 32L))
  } in
  
  let binary2 = serialize_functions [f2] in
  let (ptr, _err) = compile_binary binary2 in
  if ptr = Int64.zero then Printf.printf "Test 2: Compilation failed\n"
  else
      let res = execute_function ptr in
      Printf.printf "Test 2 (10 + 32) result: %Ld\n" res;
      if res = 42L then Printf.printf "SUCCESS\n" else Printf.printf "FAILURE\n";

  (* 4. Test 3: Factorial (If/Rec/Mul) *)
  let f3 = {
    name = "fact";
    params = ["n"];
    return_type = I64;
    body = If (
      CmpLte (Var "n", IntLit 1L),
      Return (IntLit 1L),
      Return (Mul (Var "n", App ("fact", [Sub (Var "n", IntLit 1L)])))
    )
  } in
  
  let binary3 = serialize_functions [f3] in
  let (ptr, _err) = compile_binary binary3 in
  if ptr = Int64.zero then Printf.printf "Test 3: Compilation failed\n"
  else
      let res = execute_function_1 ptr 5L in
      Printf.printf "Test 3 (fact(5)) result: %Ld\n" res;
      if res = 120L then Printf.printf "SUCCESS\n" else Printf.printf "FAILURE\n";

  (* 5. Test 4: While loop (sum 1 to 10) *)
  let f4 = {
    name = "sum_1_to_n";
    params = ["n"];
    return_type = I64;
    body = Block (
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
    )
  } in
  
  let binary4 = serialize_functions [f4] in
  let (ptr, _err) = compile_binary binary4 in
  if ptr = Int64.zero then Printf.printf "Test 4: Compilation failed\n"
  else
      let res = execute_function_1 ptr 10L in
      Printf.printf "Test 4 (sum 1 to 10) result: %Ld\n" res;
      if res = 55L then Printf.printf "SUCCESS\n" else Printf.printf "FAILURE\n";

  (* 6. Test 5: Match statement *)
  let f5 = {
    name = "pick_number";
    params = ["x"];
    return_type = I64;
    body = Match (
      Var "x",
      [
        (1L, Return (IntLit 100L));
        (2L, Return (IntLit 200L))
      ],
      Return (IntLit 300L)
    )
  } in
  
  let binary5 = serialize_functions [f5] in
  let (ptr, _err) = compile_binary binary5 in
  if ptr = Int64.zero then Printf.printf "Test 5: Compilation failed\n"
  else
      let res1 = execute_function_1 ptr 1L in
      let res2 = execute_function_1 ptr 2L in
      let res3 = execute_function_1 ptr 3L in
      Printf.printf "Test 5 (pick(1,2,3)) results: %Ld, %Ld, %Ld\n" res1 res2 res3;
      if res1 = 100L && res2 = 200L && res3 = 300L then Printf.printf "SUCCESS\n" else Printf.printf "FAILURE\n";

  (* 7. Test 6: Runtime Integration (au_print_int) *)
  let f6 = {
    name = "test_print";
    params = ["n"];
    return_type = Unit;
    body = Block (
      While (
        CmpGt (Var "n", IntLit 0L),
        Block (
          Discard (App ("au_print_int", [Var "n"])),
          Assign ("n", Sub (Var "n", IntLit 1L))
        )
      ),
      Return (IntLit 0L)
    )
  } in
  
  let binary6 = serialize_functions [f6] in
  let (ptr, _err) = compile_binary binary6 in
  if ptr = Int64.zero then Printf.printf "Test 6: Compilation failed\n"
  else
      Printf.printf "Test 6 (print 3 to 1):\n";
      let res = execute_function_1 ptr 3L in
      Printf.printf "Test 6 finished (result=%Ld)\n" res;

  (* 8. Test 7: Records (__record_new, __slot_get) *)
  let f7 = {
    name = "test_record";
    params = [];
    return_type = I64;
    body = Block (
      (* Create a record with 2 fields (size 16) *)
      Let ("p", App ("__record_new", [IntLit 16L; IntLit 11L; IntLit 22L]), Skip),
      (* Return field 2 (offset 8) *)
      Return (App ("__slot_get", [Var "p"; IntLit 8L]))
    )
  } in
  
  let binary7 = serialize_functions [f7] in
  let (ptr, _err) = compile_binary binary7 in
  if ptr = Int64.zero then Printf.printf "Test 7: Compilation failed\n"
  else
      let res = execute_function ptr in
      Printf.printf "Test 7 (Record Slot) result: %Ld\n" res;
      if res = 22L then Printf.printf "SUCCESS\n" else Printf.printf "FAILURE\n";

  (* 9. Test 8: Pointers (Store, Deref) *)
  let f8 = {
    name = "test_pointer";
    params = [];
    return_type = I64;
    body = Block (
      (* Allocate space for one I64 *)
      Let ("ptr", App ("au_alloc", [IntLit 8L]), Skip),
      Block (
        (* Store 42 at ptr *)
        Store (Var "ptr", IntLit 42L),
        (* Deref ptr *)
        Return (Deref (Var "ptr"))
      )
    )
  } in
  
  let binary8 = serialize_functions [f8] in
  let (ptr, _err) = compile_binary binary8 in
  if ptr = Int64.zero then Printf.printf "Test 8: Compilation failed\n"
  else
      let res = execute_function ptr in
      Printf.printf "Test 8 (Pointer Store/Load) result: %Ld\n" res;
      if res = 42L then Printf.printf "SUCCESS\n" else Printf.printf "FAILURE\n";

  (* 10. Test 9: Union construction and discriminant matching *)
  (* Construct a union: (size=16, tag, field) where:
       tag 0 => Just(value)  returns value
       tag 1 => Nothing      returns -1
     Then match on tag from offset 0, field from offset 8 *)
  let f9 = {
    name = "test_union_match";
    params = ["tag_in"; "val_in"];
    return_type = I64;
    body = Block (
      (* alloc 16 bytes, store tag at [0], value at [8] *)
      Let ("u", App ("au_alloc", [IntLit 16L]), Skip),
      Block (
        Store (Var "u", Var "tag_in"),
        Block (
          Store (Add (Var "u", IntLit 8L), Var "val_in"),
          (* Now match: load tag, compare *)
          Block (
            Let ("t", App ("__slot_get", [Var "u"; IntLit 0L]), Skip),
            If (
              CmpEq (Var "t", IntLit 0L),
              (* Just: return field *)
              Return (App ("__slot_get", [Var "u"; IntLit 8L])),
              (* Nothing: return -1 *)
              Return (IntLit (-1L))
            )
          )
        )
      )
    )
  } in

  let binary9 = serialize_functions [f9] in
  let (ptr, _err) = compile_binary binary9 in
  if ptr = Int64.zero then Printf.printf "Test 9: Compilation failed\n"
  else
      let res_just    = execute_function_2 ptr 0L 99L in
      let res_nothing = execute_function_2 ptr 1L 0L  in
      Printf.printf "Test 9 (Union Just/Nothing): just=%Ld nothing=%Ld\n" res_just res_nothing;
      if res_just = 99L && res_nothing = (-1L)
        then Printf.printf "SUCCESS\n"
        else Printf.printf "FAILURE\n";

  (* 11. Test 10: Deep Tail Recursion (O(1) Stack Verification) *)
  let f10 = {
    name = "tail_rec_sum";
    params = ["n"; "acc"];
    return_type = I64;
    body = If (
      CmpEq (Var "n", IntLit 0L),
      Return (Var "acc"),
      Return (App ("tail_rec_sum", [Sub (Var "n", IntLit 1L); Add (Var "acc", Var "n")]))
    )
  } in

  let binary10 = serialize_functions [f10] in
  match compile_binary binary10 with
  | (0L, err) -> Printf.printf "Test 10: Compilation failed: %s\n" (match err with Some s -> s | None -> "Unknown")
  | (ptr, _) ->
      (* Sum 1 to 1000. Expected: 500500. *)
      let res = execute_function_2 ptr 1000L 0L in
      Printf.printf "Test 10 (Deep Recursion): sum 1..1000 = %Ld\n" res;
      if res = 500500L
        then Printf.printf "SUCCESS\n"
        else Printf.printf "FAILURE\n";

  (* 12. Test 11: Cedar Static Authorization *)
  let f11 = {
    name = "cedar_test";
    params = [];
    return_type = I64;
    body = Return (App ("SafeFunc", []))
  } in
  
  let f12 = {
    name = "cedar_fail_test";
    params = [];
    return_type = I64;
    body = Return (App ("ForbiddenFunc", []))
  } in

  (* Load Cedar policy *)
  let policy = "permit(principal == Module::\"cedar_test\", action == Action::\"Call\", resource == Module::\"SafeFunc\");\n" ^
               "forbid(principal == Module::\"cedar_fail_test\", action == Action::\"Call\", resource == Module::\"ForbiddenFunc\");" in
  Printf.printf "Test 11: Loading Cedar policy...\n";
  if not (cedar_load_policy policy) then
    Printf.printf "Test 11: Failed to load Cedar policy (Error: %s)\n" (match last_jit_error () with Some s -> s | None -> "Unknown")
  else begin
    (* Test allowed call *)
    let binary11 = serialize_functions [f11] in
    let (ptr11, err11) = compile_binary binary11 in
    if ptr11 <> Int64.zero then Printf.printf "Test 11 (Cedar Allowed): SUCCESS (JIT allowed compilation)\n"
    else Printf.printf "Test 11 (Cedar Allowed): FAILURE - %s\n" (match err11 with Some s -> s | None -> "Unknown");

    (* Test forbidden call *)
    let binary12 = serialize_functions [f12] in
    let (ptr12, err12) = compile_binary binary12 in
    if ptr12 = Int64.zero then begin
      Printf.printf "Test 11 (Cedar Forbidden): SUCCESS (Caught denial)\n";
      Printf.printf "Caught Error: %s\n" (match err12 with Some s -> s | None -> "Unknown")
    end else
      Printf.printf "Test 11 (Cedar Forbidden): FAILURE (Allowed forbidden call)\n"
  end;

  Printf.printf "\nJIT manual tests completed.\n"
