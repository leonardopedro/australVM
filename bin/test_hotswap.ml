open CpsGen
open CamlCompiler_rust_bridge

let () =
  Printf.printf "Starting Live Hot-Swap Test...\n";
  if not (initialize ()) then (Printf.printf "Failed to initialize bridge\n"; exit 1);
  
  (* 1. Compile ManagedCell::cell_step (increments by 1) *)
  let f_managed = {
    name = "ManagedCell::cell_step";
    params = ["state"];
    return_type = Unit;
    body = Block(
        Let("c", Deref(Var "state"), Skip),
        Block(
            Store(Var "state", Add(Var "c", IntLit 1L)),
            Return(IntLit 0L)
        )
    )
  } in
  let (ptr_managed, err_m) = compile_binary (serialize_functions [f_managed]) in
  (match err_m with Some msg -> Printf.printf "Managed Compile Error: %s\n" msg | None -> ());

  (* 2. Compile AdvancedCell::cell_step (increments by 10) *)
  let f_advanced = {
    name = "AdvancedCell::cell_step";
    params = ["state"];
    return_type = Unit;
    body = Block(
        Let("c", Deref(Var "state"), Skip),
        Block(
            Store(Var "state", Add(Var "c", IntLit 10L)),
            Return(IntLit 0L)
        )
    )
  } in
  let (ptr_advanced, err_a) = compile_binary (serialize_functions [f_advanced]) in
  (match err_a with Some msg -> Printf.printf "Advanced Compile Error: %s\n" msg | None -> ());

  if ptr_managed = Int64.zero || ptr_advanced = Int64.zero then begin
    Printf.printf "Compilation failed\n";
    exit 1
  end;

  (* 3. Simulation *)
  Printf.printf "Hotswap Ready: Managed at 0x%Lx, Advanced at 0x%Lx\n" ptr_managed ptr_advanced;
  
  let state = au_alloc 8L in
  store state 0L;
  Printf.printf "Initial state value: %Ld\n" (load state);
  
  (* Step with Managed *)
  ignore (execute_function_1 ptr_managed state);
  Printf.printf "Step 1 (Managed logic): %Ld\n" (load state);
  if (load state) <> 1L then (Printf.printf "FAILURE: Expected 1\n"; exit 1);
  
  (* SWAP! *)
  Printf.printf "--- HOT-SWAPPING to Advanced Logic ---\n";
  (* In this simulation, we swap the JIT pointer used by the 'scheduler' (the test runner) *)
  
  (* Step with Advanced *)
  ignore (execute_function_1 ptr_advanced state);
  Printf.printf "Step 2 (Advanced logic): %Ld\n" (load state);
  
  if (load state) = 11L then
    Printf.printf "\nHOTSWAP SUCCESS: State preserved and behavior updated live!\n"
  else begin
    Printf.printf "\nHOTSWAP FAILURE: Expected 11, got %Ld\n" (load state);
    exit 1
  end;

  Printf.printf "Hot-swap test completed successfully.\n"
