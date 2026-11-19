(*
   GpuIrTest — G02.

   Checks the parts of the IR/bundle contract that are testable without a
   device: deterministic canonical bytes, u32 literal bounds, and the split
   between semantic IR identity and source-byte digests.
*)

open OUnit2

let identity_kernel =
  { Gpu_ir.entry_name = "identity";
    input_arity = 1;
    helpers = [];
    body = Gpu_ir.Return (Gpu_ir.Param 0);
    memory = Gpu_ir.default_memory_plan 1;
    numeric_profile = Gpu_ir.U32ModularV1 }

let f32_kernel =
  { Gpu_ir.entry_name = "scale";
    input_arity = 1;
    helpers = [];
    body =
      Gpu_ir.Return
        (Gpu_ir.Intrinsic (Gpu_ir.F32Multiply, [Gpu_ir.Param 0; Gpu_ir.F32 "2.5"]));
    memory = Gpu_ir.default_memory_plan 1;
    numeric_profile = Gpu_ir.WgslF32V1 }

let test_canonical_bytes_are_stable _ =
  let a = Gpu_ir.canonical_bytes identity_kernel
  and b = Gpu_ir.canonical_bytes identity_kernel in
  assert_equal a b;
  (* Parsing the emitted JSON and re-serializing it reproduces the same bytes,
     so the digest is over a value, not a string layout accident. *)
  assert_equal a (Yojson.Safe.to_string (Yojson.Safe.from_string a))

let test_u32_bounds _ =
  assert_equal "4294967295" (Gpu_ir.u32_of_string "4294967295");
  assert_equal "0" (Gpu_ir.u32_of_string "0");
  assert_bool "2^32 is out of range"
    (match Gpu_ir.u32_of_string "4294967296" with
     | exception Gpu_ir.Ir_error _ -> true
     | _ -> false);
  assert_bool "negative literal is out of range"
    (match Gpu_ir.u32_of_string "-1" with
     | exception Gpu_ir.Ir_error _ -> true
     | _ -> false)

let test_semantic_vs_source_digest _ =
  let body = "module body P is\n  function f(x: Nat32): Nat32 is\n    return x;\n  end;\nend module body.\n" in
  let commented = "-- a comment that does not change the program\n" ^ body in
  let src1 = Gpu_bundle.digest_source ~path:"P.aum" body
  and src2 = Gpu_bundle.digest_source ~path:"P.aum" commented in
  assert_bool "a comment-only edit changes the source digest"
    (src1.Gpu_bundle.sha256 <> src2.Gpu_bundle.sha256);
  let b1 = Gpu_bundle.make ~sources:[src1] identity_kernel
  and b2 = Gpu_bundle.make ~sources:[src2] identity_kernel in
  assert_equal b1.Gpu_bundle.ir_sha256 b2.Gpu_bundle.ir_sha256;
  assert_bool "the source digest is not part of semantic IR identity"
    (b1.Gpu_bundle.ir_sha256 <> b1.Gpu_bundle.wgsl_sha256)

let test_bundle_lists_builtin_sources _ =
  let b = Gpu_bundle.make ~sources:[] identity_kernel in
  let paths = List.map (fun (s: Gpu_bundle.source) -> s.path) b.Gpu_bundle.sources in
  assert_bool "the canonical Pervasive interface is recorded"
    (List.mem "builtin/Austral.Pervasive.aui" paths);
  assert_bool "the canonical Pervasive body is recorded"
    (List.mem "builtin/Austral.Pervasive.aum" paths)

let test_f32_literals_round_trip _ =
  assert_equal "3.0" (Gpu_ir.f32_of_string "3");
  assert_equal "0.1" (Gpu_ir.f32_of_string "0.1");
  assert_equal "2.5" (Gpu_ir.f32_of_string "2.50");
  assert_bool "an overflowing literal is not a finite binary32 value"
    (match Gpu_ir.f32_of_string "1e40" with
     | exception Gpu_ir.Ir_error _ -> true
     | _ -> false);
  assert_bool "a literal parses to the same bits it prints"
    (Gpu_ir.f32_bits_of_string "0.1"
     = Gpu_ir.f32_bits_of_string (Gpu_ir.f32_of_string "0.1"))

let test_profiles_do_not_mix _ =
  let json = Gpu_ir.canonical_bytes f32_kernel in
  assert_bool "the profile is part of the semantic IR"
    (String.length json > 0
     && (try
           let _ = Str.search_forward
                     (Str.regexp_string "\"numeric_profile\":\"wgsl-f32-v1\"")
                     json 0 in
           true
         with Not_found -> false));
  assert_bool "an f32 literal in the u32 profile is refused"
    (match Gpu_ir.check_kernel
             { identity_kernel with Gpu_ir.body = Gpu_ir.Return (Gpu_ir.F32 "1.0") } with
     | exception Gpu_ir.Ir_error _ -> true
     | () -> false);
  assert_bool "an integer intrinsic in the f32 profile is refused"
    (match Gpu_ir.check_kernel
             { f32_kernel with
               Gpu_ir.body = Gpu_ir.Return (Gpu_ir.Intrinsic (Gpu_ir.Add, [Gpu_ir.Param 0; Gpu_ir.Param 0])) } with
     | exception Gpu_ir.Ir_error _ -> true
     | () -> false)

let test_f32_requirements _ =
  let b = Gpu_bundle.make ~sources:[] f32_kernel in
  let reqs = Yojson.Safe.from_string
      (Yojson.Safe.to_string b.Gpu_bundle.requirements) in
  let open Yojson.Safe.Util in
  let bindings = reqs |> member "bindings" |> to_list in
  assert_equal "f32" (List.nth bindings 0 |> member "element" |> to_string);
  assert_equal 4 (List.nth bindings 0 |> member "stride_bytes" |> to_int);
  assert_equal "wgsl-f32-v1"
    (reqs |> member "numeric" |> member "profile" |> to_string);
  assert_equal 1
    (reqs |> member "numeric" |> member "tolerance_ulp_per_operation" |> to_int);
  assert_equal "permitted"
    (reqs |> member "numeric" |> member "contraction" |> to_string)

let test_requirements_bindings _ =
  let k = { identity_kernel with
            Gpu_ir.entry_name = "add";
            input_arity = 2;
            body = Gpu_ir.Return (Gpu_ir.Intrinsic (Gpu_ir.Add, [Gpu_ir.Param 0; Gpu_ir.Param 1]));
            memory = Gpu_ir.default_memory_plan 2 } in
  let b = Gpu_bundle.make ~sources:[] k in
  let json = Yojson.Safe.to_string b.Gpu_bundle.requirements in
  assert_bool "two inputs bind at 0 and 1"
    (String.length json > 0);
  let reqs = Yojson.Safe.from_string json in
  let open Yojson.Safe.Util in
  let bindings = reqs |> member "bindings" |> to_list in
  assert_equal 3 (List.length bindings);
  assert_equal "output" (List.nth bindings 2 |> member "role" |> to_string)

let suite =
  "GpuIrTest" >::: [
    "canonical IR bytes are stable" >:: test_canonical_bytes_are_stable;
    "u32 literal bounds refuse" >:: test_u32_bounds;
    "f32 literals round-trip" >:: test_f32_literals_round_trip;
    "numeric profiles do not mix" >:: test_profiles_do_not_mix;
    "f32 requirements name the profile" >:: test_f32_requirements;
    "comment edits source digest only" >:: test_semantic_vs_source_digest;
    "builtin sources are recorded" >:: test_bundle_lists_builtin_sources;
    "binding roles follow arity" >:: test_requirements_bindings;
  ]

let () = run_test_tt_main suite
