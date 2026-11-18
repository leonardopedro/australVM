(*
   GpuWgslTest — G06.

   Renders the pointwise fixtures through typed KernelIR and checks the
   structure the contract requires: concrete u32 literals, an identity-index
   write, a pre-load guard, fixed workgroup size 64, and no user identifier
   leaking into WGSL. Validity/meaning are G07/G08's concern.
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

let render entry =
  let (env, mono) = prepared () in
  Gpu_wgsl.render (Gpu_subset.build ~entry env mono)

let contains ~needle haystack =
  let n = String.length needle and h = String.length haystack in
  let rec go i = i + n <= h && (String.sub haystack i n = needle || go (i + 1)) in
  go 0

let test_wrapper_structure _ =
  let wgsl = render "add" in
  assert_bool "fixed workgroup size" (contains ~needle:"@workgroup_size(64)" wgsl);
  assert_bool "guard before load" (contains ~needle:"if (i >= arrayLength(&input0)) { return; }" wgsl);
  assert_bool "identity output index" (contains ~needle:"output0[i] = scalar_entry(input0[i], input1[i]);" wgsl);
  assert_bool "read-only inputs" (contains ~needle:"var<storage, read> input0: array<u32>" wgsl);
  assert_bool "fresh read_write output" (contains ~needle:"var<storage, read_write> output0" wgsl)

let test_arithmetic_derives_from_ir _ =
  let add = render "add" and sub = render "difference" in
  assert_bool "add renders u32 addition" (contains ~needle:"(p0 + p1)" add);
  assert_bool "subtraction keeps operand order" (contains ~needle:"(p0 - p1)" sub)

let test_literals_and_branches _ =
  let clamp = render "clamp" in
  assert_bool "u32 literal suffix" (contains ~needle:"100u" clamp);
  assert_bool "comparison preserved" (contains ~needle:"(p0 > 100u)" clamp);
  assert_bool "conditional structure preserved" (contains ~needle:"if " clamp)

let test_no_user_identifiers _ =
  let wgsl = render "mask" in
  assert_bool "entry body is compiler-named" (contains ~needle:"scalar_entry" wgsl);
  assert_bool "user function name does not appear"
    (not (contains ~needle:"mask(" wgsl))

let test_helper_module_renders_helpers _ =
  let wgsl = render "polynomial" in
  assert_bool "helper function is emitted" (contains ~needle:"fn helper_" wgsl);
  assert_bool "helper is called" (contains ~needle:"helper_" wgsl);
  assert_bool "helper name is compiler-generated"
    (not (contains ~needle:"twice" wgsl))

let test_render_is_deterministic _ =
  assert_equal (render "add") (render "add")

let suite =
  "GpuWgslTest" >::: [
    "wrapper structure" >:: test_wrapper_structure;
    "arithmetic derives from IR" >:: test_arithmetic_derives_from_ir;
    "literals and branches are preserved" >:: test_literals_and_branches;
    "no user identifiers leak" >:: test_no_user_identifiers;
    "helper-based kernel renders helpers" >:: test_helper_module_renders_helpers;
    "render is deterministic" >:: test_render_is_deterministic;
  ]

let () = run_test_tt_main suite
