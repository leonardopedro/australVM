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

(* ── PLAN_valen §4 Tier 1: extern rust calls through the bridge ───── *)

(* The auth engine is process-global and `load_auth_manifest` MERGES, so
   these tests must run in a fixed order — denial first, grant last — and no
   test may rely on an engine state it did not install itself. They sit at
   the end of the suite for the same reason. *)

let rs_manifest_toml callables =
  let quoted = String.concat ", " (List.map (fun c -> "\"" ^ c ^ "\"") callables) in
  "[module]\nname = \"rs_jit_widget\"\n\n[grants]\nkernel = [" ^ quoted ^ "]\n"

let contains haystack needle =
  let nh = String.length haystack and nn = String.length needle in
  let rec go i =
    if i + nn > nh then false
    else if String.sub haystack i nn = needle then true
    else go (i + 1)
  in
  go 0

(* A CPS module with one function `fname(buf, len) = <callee>(buf, len)` and
   an explicit v2 module header, so the JIT attributes the gated call to
   module `rs_jit_widget` — the A10 module principal. *)
let rs_cps_binary fname callee =
  let f = {
    name = fname;
    params = ["buf"; "len"];
    return_type = I64;
    body = Return (App (callee, [Var "buf"; Var "len"]))
  } in
  CpsGen.serialize_functions ~module_name:"rs_jit_widget" [f]

let compile_rs fname callee =
  init ();
  CamlCompiler_rust_bridge.compile_binary (rs_cps_binary fname callee)

(* Denial, no manifest: the documented default is DenyAll, so an ungranted
   extern rust call never reaches the reference crate — UK-4001 at the live
   JIT gate (`cps.rs::a10_live_grant_tests` shape, from the OCaml side). *)
let test_rs_denied_without_manifest _ =
  (match compile_rs "run_rs_no_manifest" "rs_checksum" with
   | (0L, Some err) ->
       assert_bool ("expected UK-4001, got: " ^ err) (contains err "UK-4001")
   | (0L, None) -> assert_failure "compile failed without an error message"
   | (ptr, _) -> assert_failure
       ("an ungranted rs_ call must not compile, got pointer " ^ Int64.to_string ptr))

(* Denial, uk-only manifest: PLAN_valen §4 — "a manifest that grants
   `uk_push` does not thereby grant a tier-1 call", from the OCaml side. *)
let test_rs_denied_by_uk_only_grant _ =
  init ();
  assert_bool "uk-only manifest must load"
    (CamlCompiler_rust_bridge.load_auth_manifest (rs_manifest_toml ["uk_version"]));
  (match compile_rs "run_rs_uk_only" "rs_checksum" with
   | (0L, Some err) ->
       assert_bool ("expected UK-4001, got: " ^ err) (contains err "UK-4001")
   | (0L, None) -> assert_failure "compile failed without an error message"
   | (ptr, _) -> assert_failure
       ("a uk-only manifest must deny rs_checksum, got pointer " ^ Int64.to_string ptr))

(* Granted, end-to-end: the manifest names rs_checksum, the JIT resolves it
   to the reference crate's rs_checksum, and it runs over a live buffer
   allocated from OCaml. Expected = FNV-1a 64 of "au!", independently
   pinned here (the Rust-side rsffi crate tests pin the same vector
   against its canonical sources). *)
let test_rs_granted_executes_checksum _ =
  init ();
  assert_bool "rs-granting manifest must load"
    (CamlCompiler_rust_bridge.load_auth_manifest (rs_manifest_toml ["rs_checksum"]));
  let ptr =
    match compile_rs "run_rs_granted" "rs_checksum" with
    | (0L, err) ->
        assert_failure ("a granted rs_ call must compile: " ^
          (match err with Some e -> e | None -> "unknown error"))
    | (p, _) -> p
  in
  (* Buffer "au!" as one LE word: 0x61 0x75 0x21 followed by zeroes. *)
  let buf = CamlCompiler_rust_bridge.au_alloc 8L in
  assert_bool "au_alloc must return a live pointer" (buf <> 0L);
  CamlCompiler_rust_bridge.store buf 0x217561L;
  let got = CamlCompiler_rust_bridge.execute_function_2 ptr buf 3L in
  assert_equal ~printer:Int64.to_string (-1781872834555808882L) got

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
    (* rs_ tests ordered denial-before-grant: the auth engine merges. *)
    "rs_denied_without_manifest" >:: test_rs_denied_without_manifest;
    "rs_denied_by_uk_only_grant" >:: test_rs_denied_by_uk_only_grant;
    "rs_granted_executes_checksum" >:: test_rs_granted_executes_checksum;
  ]

let _ = run_test_tt_main suite
