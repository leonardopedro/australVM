(*
   GpuMlirTest — G20a.

   Renders the pointwise fixtures through the same typed KernelIR the WGSL
   emitter consumes and checks the encoding G20a fixes: signless i32 with
   unsigned comparisons, modular arithmetic without wrap flags, an explicit
   static extent, a tail guard that dominates every access, and a named refusal
   for IR this step does not cover. The structural validator is tested against
   the two mistakes it exists to catch.

   This is a test of the *emitter*, not of MLIR: there is no `mlir-opt` here, so
   nothing below claims the module verifies.
*)

open OUnit2

let read_file path =
  let ic = open_in_bin path in
  let n = in_channel_length ic in
  let s = really_input_string ic n in
  close_in ic;
  s

let prepared () =
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.TwoFileModuleSource
      { int_filename = (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aui");
        int_code = read_file (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aui");
        body_filename = (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aum");
        body_code = read_file (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aum") }
  in
  let (_, p) = Compiler.prepare_module env source in
  (p.Compiler.prepared_env, p.Compiler.prepared_mono)

let kernel entry =
  let (env, mono) = prepared () in
  Gpu_subset.build ~entry env mono

let render ?(extent = 65) entry =
  Gpu_mlir.render ~extent (kernel entry)

let contains ~needle haystack =
  let n = String.length needle and h = String.length haystack in
  let rec go i = i + n <= h && (String.sub haystack i n = needle || go (i + 1)) in
  go 0

let test_identity_and_guard _ =
  let mlir = render "clamp" in
  assert_bool "the KernelIR identity is the header"
    (contains ~needle:
       "// kernelir-sha256: 48caf04ea1714fba539ab596cd83d4f7ab537f3eea7837a5b129ccbecb9d8288"
       mlir);
  assert_bool "explicit static extent"
    (contains ~needle:"memref<65xi32>" mlir);
  assert_bool "the element index is block_id * block_dim + thread_id"
    (contains ~needle:"%bid_base = arith.muli %bid, %bdim : index" mlir
     && contains ~needle:"%gid = arith.addi %bid_base, %tid : index" mlir);
  assert_bool "tail guard is unsigned index arithmetic"
    (contains ~needle:"arith.cmpi ult, %gid, %c_extent : index" mlir);
  assert_bool "one load per input"
    (contains ~needle:"%arg0_v = memref.load %arg0[%gid]" mlir);
  assert_bool "one store to the output"
    (contains ~needle:"memref.store %v3, %arg1[%gid]" mlir);
  assert_bool "the guard opens the only access region"
    (contains ~needle:"scf.if %in_bounds {" mlir)

let test_unsigned_comparisons _ =
  let clamp = render "clamp" in
  assert_bool "Nat32 ordering is unsigned"
    (contains ~needle:"arith.cmpi ugt, %arg0_v, %v0 : i32" clamp);
  let minimum = render "minimum" in
  assert_bool "the other direction is unsigned too"
    (contains ~needle:"arith.cmpi ult, %arg0_v, %arg1_v : i32" minimum);
  List.iter
    (fun signed ->
       assert_bool ("no signed predicate " ^ signed)
         (not (contains ~needle:("arith.cmpi " ^ signed) clamp)))
    ["slt"; "sgt"; "sle"; "sge"]

let test_no_wrap_flags _ =
  let mlir = render "multiply" in
  assert_bool "multiplication present"
    (contains ~needle:"arith.muli %arg0_v, %arg1_v : i32" mlir);
  assert_bool "no nuw" (not (contains ~needle:"nuw" mlir));
  assert_bool "no nsw" (not (contains ~needle:"nsw" mlir))

let test_subtract_operand_order _ =
  let sub = render "difference" in
  assert_bool "subtraction keeps KernelIR operand order"
    (contains ~needle:"arith.subi %arg0_v, %arg1_v : i32" sub);
  (* swapping the operands produces a different body; the emitter is not free
     to canonicalise them because Nat32 subtraction is not commutative *)
  assert_bool "operands are not reordered"
    (not (contains ~needle:"arith.subi %arg1_v, %arg0_v : i32" sub))

let test_deterministic _ =
  let a = render "clamp" and b = render "clamp" in
  assert_equal a b ~printer:(fun s -> s)

let test_extent_is_required _ =
  (try
     ignore (Gpu_mlir.render ~extent:0 (kernel "clamp"));
     assert_failure "a zero extent must be refused"
   with Invalid_argument msg ->
     assert_bool "the refusal names the extent"
       (contains ~needle:"static extent must be positive" msg))

let test_named_refusal_for_helpers _ =
  (* the admitted corpus has no helpers; a kernel that references one must be
     refused by name rather than silently inlined or dropped *)
  let k = kernel "clamp" in
  let helper =
    { Gpu_ir.id = 0; params = [Gpu_ir.TyU32]; ret = Gpu_ir.TyU32;
      body = Gpu_ir.Return (Gpu_ir.Param 0) }
  in
  let with_call =
    { k with Gpu_ir.helpers = [helper];
             body = Gpu_ir.Return (Gpu_ir.Call (0, [Gpu_ir.Param 0])) }
  in
  (try
     ignore (Gpu_mlir.render ~extent:65 with_call);
     assert_failure "a helper call must be refused"
   with Invalid_argument msg ->
     assert_bool "the refusal is named"
       (contains ~needle:"helper call is not lowered yet" msg))

let test_validate_accepts_the_emitted_module _ =
  let counts = Gpu_mlir.validate (render "clamp") in
  assert_equal 2 (List.assoc "cmpi" counts);
  assert_equal 1 (List.assoc "loads" counts);
  assert_equal 1 (List.assoc "stores" counts)

let test_validate_refuses_a_signed_comparison _ =
  let mlir = render "clamp" in
  let broken =
    Str.global_replace (Str.regexp "arith.cmpi ugt") "arith.cmpi sgt" mlir
  in
  (try
     ignore (Gpu_mlir.validate broken);
     assert_failure "a signed Nat32 comparison must be refused"
   with Invalid_argument msg ->
     assert_bool "the refusal names the predicate"
       (contains ~needle:"signed ordering comparison" msg))

let test_validate_refuses_an_undefined_value _ =
  let mlir = render "clamp" in
  let broken =
    Str.global_replace (Str.regexp "memref.store %v3") "memref.store %nope" mlir
  in
  (try
     ignore (Gpu_mlir.validate broken);
     assert_failure "an undefined value must be refused"
   with Invalid_argument msg ->
     assert_bool "the refusal names the value"
       (contains ~needle:"%nope is used but never defined" msg))

(* G20a's own mutation test: the two edits a reader would make by hand must
   change the lowered body, or the emitter is not reading the IR it claims. *)
let test_mutations_change_the_body _ =
  let base = kernel "difference" in
  let swapped =
    match base.Gpu_ir.body with
    | Gpu_ir.Return (Gpu_ir.Intrinsic (Gpu_ir.Subtract, [a; b])) ->
       { base with Gpu_ir.body =
                      Gpu_ir.Return (Gpu_ir.Intrinsic (Gpu_ir.Subtract, [b; a])) }
    | _ -> assert_failure "difference is a return of one subtraction"
  in
  assert_bool "swapping subtraction operands changes the body"
    (render "difference" <> Gpu_mlir.render ~extent:65 swapped);
  let clamp = kernel "clamp" in
  let reversed =
    match clamp.Gpu_ir.body with
    | Gpu_ir.Seq [Gpu_ir.Let (id, Gpu_ir.Compare (Gpu_ir.GreaterThan, a, b));
                  Gpu_ir.If (c, t, f)] ->
       { clamp with Gpu_ir.body =
                      Gpu_ir.Seq [Gpu_ir.Let (id, Gpu_ir.Compare (Gpu_ir.LessThan, a, b));
                                  Gpu_ir.If (c, t, f)] }
    | _ -> assert_failure "clamp compares once, then branches"
  in
  let before = render "clamp" and after = Gpu_mlir.render ~extent:65 reversed in
  assert_bool "reversing the comparison changes the body" (before <> after);
  assert_bool "the reversed comparison is unsigned less-than"
    (contains ~needle:"arith.cmpi ult, %arg0_v, %v0 : i32" after);
  assert_bool "the original stays unsigned greater-than"
    (contains ~needle:"arith.cmpi ugt, %arg0_v, %v0 : i32" before)

let suite =
  "GpuMlir" >::: [
    "identity and guard" >:: test_identity_and_guard;
    "unsigned comparisons" >:: test_unsigned_comparisons;
    "no wrap flags" >:: test_no_wrap_flags;
    "subtract operand order" >:: test_subtract_operand_order;
    "deterministic" >:: test_deterministic;
    "extent is required" >:: test_extent_is_required;
    "named refusal for helpers" >:: test_named_refusal_for_helpers;
    "validate accepts" >:: test_validate_accepts_the_emitted_module;
    "validate refuses signed comparison" >:: test_validate_refuses_a_signed_comparison;
    "validate refuses undefined value" >:: test_validate_refuses_an_undefined_value;
    "mutations change the body" >:: test_mutations_change_the_body;
  ]

let () = run_test_tt_main suite
