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

(* Float64 arithmetic, end to end through the bridge's f64 primitives.

   The CPS front-end lowers `+ - * /` on Float64 to `au_f64_*` calls
   (`Compiler_cps.float_arith_symbol`) rather than to the `Add`/`Sub`/`Mul`
   opcodes: those operate on the 64-bit word, which for a float is its IEEE
   bit pattern, so `1.5 + 2.5` as integers is 1.5+2.5's bits added — a
   different number — and `2.5 * 4.0` traps as integer overflow.

   These tests pin the *words*, not the floats: the value built here is the
   f64 bits (`FloatLit` serializes as `Int64.bits_of_float`, one word), so a
   returning primitive that forgot `.to_bits()` shows up as an integer. *)
(* Expected words are the f64 bit patterns written out: 4.0 = 0x4010_0000_0000_0000,
   1.5 = 0x3FF8…, 10.0 = 0x4024…, 3.0 = 0x4008…; an integer 4 would show up as
   0x4 and fail. *)
let compile_f64 fname op args expected_bits =
  compile_and_execute fname (Return (App (op, args))) expected_bits

let test_f64_add _ =
  compile_f64 "run_f64_add" "au_f64_add" [FloatLit 1.5; FloatLit 2.5] 0x4010000000000000L

let test_f64_subtract _ =
  compile_f64 "run_f64_sub" "au_f64_subtract" [FloatLit 4.0; FloatLit 2.5] 0x3FF8000000000000L

let test_f64_multiply _ =
  compile_f64 "run_f64_mul" "au_f64_multiply" [FloatLit 2.5; FloatLit 4.0] 0x4024000000000000L

let test_f64_divide _ =
  compile_f64 "run_f64_div" "au_f64_divide" [FloatLit 12.0; FloatLit 4.0] 0x4008000000000000L

(* Float32: the front-end emits the widen/operate/narrow chain, so the test
   builds the same three symbols by hand — a missing `au_f32_to_f64` or
   `au_f64_to_f32` registration fails here as an unresolved symbol rather than
   only in the live lane. The expected words are the f32 bits zero-extended. *)
let compile_f32 fname op args expected_bits =
  let widen e = App ("au_f32_to_f64", [e]) in
  compile_and_execute fname
    (Return (App ("au_f64_to_f32", [App (op, List.map widen args)])))
    expected_bits

let test_f32_add _ =
  compile_f32 "run_f32_add" "au_f64_add" [FloatLit32 1.5; FloatLit32 2.5] 0x40800000L

let test_f32_subtract _ =
  compile_f32 "run_f32_sub" "au_f64_subtract" [FloatLit32 4.0; FloatLit32 2.5] 0x3FC00000L

let test_f32_multiply _ =
  compile_f32 "run_f32_mul" "au_f64_multiply" [FloatLit32 2.5; FloatLit32 4.0] 0x41200000L

let test_f32_divide _ =
  compile_f32 "run_f32_div" "au_f64_divide" [FloatLit32 12.0; FloatLit32 4.0] 0x40400000L

(* Float comparisons: `au_f64_lt/le/gt/ge/eq/ne`, which the front-end picks
   over the `Cmp*` opcodes when the operands are floats.

   Every expected value here is a *word* the integer opcodes cannot produce.
   The `icmp` opcodes compare the words as signed 64-bit integers, and that
   order is reversed between two negative doubles: -2.0 = 0xC000…, -1.0 =
   0xBFF0…, so `CmpLt` answers `-2.0 < -1.0` with 0. On bits, `-0.0 == 0.0`
   is 0 (the opcodes) and `NaN == NaN` is 1; IEEE says the opposite, which is
   what these pin. `0.0 == -0.0` is 1 and every other NaN comparison is 0. *)
let compile_cmp fname op args expected =
  compile_and_execute fname (Return (App (op, args))) expected

let test_f64_lt_negatives _ =
  compile_cmp "run_f64_lt_neg" "au_f64_lt"
    [FloatLit (-2.0); FloatLit (-1.0)] 1L

let test_f64_gt_negatives _ =
  compile_cmp "run_f64_gt_neg" "au_f64_gt"
    [FloatLit (-2.0); FloatLit (-1.0)] 0L

let test_f64_le_boundary _ =
  compile_cmp "run_f64_le" "au_f64_le" [FloatLit 2.0; FloatLit 2.0] 1L

let test_f64_ge_mixed _ =
  compile_cmp "run_f64_ge" "au_f64_ge" [FloatLit 2.5; FloatLit 2.0] 1L

let test_f64_eq_signed_zero _ =
  compile_cmp "run_f64_eq_zero" "au_f64_eq" [FloatLit 0.0; FloatLit (-0.0)] 1L

let test_f64_ne_signed_zero _ =
  compile_cmp "run_f64_ne_zero" "au_f64_ne" [FloatLit 0.0; FloatLit (-0.0)] 0L

let test_f64_nan_eq_itself _ =
  compile_cmp "run_f64_nan_eq" "au_f64_eq" [FloatLit nan; FloatLit nan] 0L

let test_f64_nan_ne_itself _ =
  compile_cmp "run_f64_nan_ne" "au_f64_ne" [FloatLit nan; FloatLit nan] 1L

let test_f64_nan_lt _ =
  compile_cmp "run_f64_nan_lt" "au_f64_lt" [FloatLit nan; FloatLit 1.0] 0L

(* Float32 comparisons take the same widen-first route (exact: every f32 is
   an f64), so the lowering test above covers this one too; these pin the
   width conversion on the comparison path, not only on the arithmetic one. *)
let compile_cmp_f32 fname op a b expected =
  let widen e = App ("au_f32_to_f64", [e]) in
  compile_and_execute fname (Return (App (op, [widen a; widen b]))) expected

let test_f32_lt_negatives _ =
  compile_cmp_f32 "run_f32_lt_neg" "au_f64_lt"
    (FloatLit32 (-2.0)) (FloatLit32 (-1.0)) 1L

let test_f32_eq_signed_zero _ =
  compile_cmp_f32 "run_f32_eq_zero" "au_f64_eq"
    (FloatLit32 0.0) (FloatLit32 (-0.)) 1L

(* Cross-cell imports are not compile errors for C output: the cell is linked
   later, so the compiler binds the symbol to a trap thunk. JIT execution
   targets must keep the hard failure (an unresolved call would be a runtime
   hole). This pins both halves through the same FFI the CLI uses. *)
let test_link_time_imports_bind_cross_cell_symbols _ =
  init ();
  let f = {
    name = "link_time_probe";
    params = [];
    return_type = I64;
    body = Return (App ("CrossCell::compute", [IntLit 42L]));
  } in
  let bin = serialize_functions [f] in
  let contains needle hay =
    let re = Str.regexp_string needle in
    try ignore (Str.search_forward re hay 0); true with Not_found -> false
  in
  let (strict_ptr, strict_err) = compile_binary bin in
  assert_bool "strict (tc/exe) compile refuses the unresolved import"
    (strict_ptr = Int64.zero
     && (match strict_err with
         | Some m -> contains "resolve symbol" m
         | None -> false));
  set_link_time_imports true;
  let (bound_ptr, _bound_err) = compile_binary bin in
  set_link_time_imports false;
  assert_bool "C-target (link-time) compile binds the cross-cell import"
    (bound_ptr <> Int64.zero)

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
    (* Float64 arithmetic: the bridge's f64 primitives, not integer ops. *)
    "f64_add_1_5_2_5" >:: test_f64_add;
    "f64_subtract_4_0_2_5" >:: test_f64_subtract;
    "f64_multiply_2_5_4_0" >:: test_f64_multiply;
    "f64_divide_12_0_4_0" >:: test_f64_divide;
    (* Float32 arithmetic: widen, the same f64 primitive, narrow. *)
    "f32_add_1_5_2_5" >:: test_f32_add;
    "f32_subtract_4_0_2_5" >:: test_f32_subtract;
    "f32_multiply_2_5_4_0" >:: test_f32_multiply;
    "f32_divide_12_0_4_0" >:: test_f32_divide;
    (* Float comparisons: IEEE order and equality, not word order. *)
    "f64_lt_negatives" >:: test_f64_lt_negatives;
    "f64_gt_negatives" >:: test_f64_gt_negatives;
    "f64_le_boundary" >:: test_f64_le_boundary;
    "f64_ge_mixed" >:: test_f64_ge_mixed;
    "f64_eq_signed_zero" >:: test_f64_eq_signed_zero;
    "f64_ne_signed_zero" >:: test_f64_ne_signed_zero;
    "f64_nan_eq_itself" >:: test_f64_nan_eq_itself;
    "f64_nan_ne_itself" >:: test_f64_nan_ne_itself;
    "f64_nan_lt" >:: test_f64_nan_lt;
    "f32_lt_negatives" >:: test_f32_lt_negatives;
    "f32_eq_signed_zero" >:: test_f32_eq_signed_zero;
    (* Cross-cell imports: a JIT target refuses one, a C target binds it. *)
    "link_time_imports_bind_cross_cell_symbols"
    >:: test_link_time_imports_bind_cross_cell_symbols;
  ]

let _ = run_test_tt_main suite
