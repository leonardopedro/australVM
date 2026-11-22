(*
   GpuReduceWgslTest — G22b.

   The reduction emitter's structure is checked here at the source level; the
   Rust checker owns Naga validation and the structural audit, and its tests
   mutate this text's shapes. This suite pins what the emitter promises: both
   entries, one 64-lane workgroup, a guarded load, the six strides in order,
   one barrier per step, one group write, and metadata that identifies the
   program without asserting per-run pass counts.
*)

open OUnit2

let count_substring ~(needle: string) (haystack: string): int =
  let n = String.length needle in
  let rec go i acc =
    if i + n > String.length haystack then acc
    else if String.sub haystack i n = needle then go (i + n) acc + 1
    else go (i + 1) acc
  in
  go 0 0

let kernel_with body =
  { Gpu_ir.entry_name = "Fold:map";
    input_arity = 1;
    helpers = [];
    body;
    memory = Gpu_ir.default_memory_plan 1;
    numeric_profile = Gpu_ir.U32ModularV1 }

let mapper_body =
  Gpu_ir.Return
    (Gpu_ir.Intrinsic (Gpu_ir.Add,
                       [ Gpu_ir.Intrinsic (Gpu_ir.Multiply, [ Gpu_ir.Param 0; Gpu_ir.U32 "3" ]);
                         Gpu_ir.U32 "1" ]))

let kernel = kernel_with mapper_body

let test_entries_and_workgroup _ =
  let wgsl = Gpu_reduce_wgsl.render kernel in
  assert_equal 1 (count_substring ~needle:"fn map_sum(" wgsl);
  assert_equal 1 (count_substring ~needle:"fn add_partials(" wgsl);
  assert_equal 2
    (count_substring ~needle:"@compute @workgroup_size(64)" wgsl);
  assert_equal 1
    (count_substring ~needle:"var<workgroup> scratch: array<u32, 64>;" wgsl);
  assert_equal 1 (count_substring ~needle:"fn mapper(p0: u32) -> u32" wgsl)

let test_guard_and_strides _ =
  let wgsl = Gpu_reduce_wgsl.render kernel in
  (* Every load is guarded and the tail lane is the identity, never mapped. *)
  assert_equal 2 (count_substring ~needle:"if (i >= arrayLength(&input0))" wgsl);
  assert_equal 2 (count_substring ~needle:"scratch[lane] = 0u;" wgsl);
  assert_equal 1 (count_substring ~needle:"scratch[lane] = mapper(input0[i]);" wgsl);
  assert_equal 1 (count_substring ~needle:"scratch[lane] = input0[i];" wgsl);
  (* One barrier after the load and after each of the six strides. *)
  assert_equal 14 (count_substring ~needle:"workgroupBarrier();" wgsl);
  let ordered =
    [ "lane + 32u"; "lane + 16u"; "lane + 8u"; "lane + 4u"; "lane + 2u"; "lane + 1u" ]
  in
  let positions =
    List.map
      (fun s ->
        match
          let rec find i =
            if i + String.length s > String.length wgsl then None
            else if String.sub wgsl i (String.length s) = s then Some i
            else find (i + 1)
          in
          find 0
        with
        | Some p -> p
        | None -> assert_failure ("stride not emitted: " ^ s))
      ordered
  in
  let rec nondecreasing = function
    | a :: (b :: _ as rest) -> a < b && nondecreasing rest
    | _ -> true
  in
  assert_bool "strides are in descending order" (nondecreasing positions);
  assert_equal 2 (count_substring ~needle:"if (lane == 0u)" wgsl);
  assert_equal 2
    (count_substring ~needle:"output0[wid.x] = scratch[0u];" wgsl)

let test_x4_layout _ =
  let wgsl = Gpu_reduce_wgsl.render ~variant:Gpu_reduce_wgsl.Lane4 kernel in
  assert_equal 1 (count_substring ~needle:"fn map_sum(" wgsl);
  assert_equal 1 (count_substring ~needle:"fn add_partials(" wgsl);
  assert_equal 2
    (count_substring ~needle:"@compute @workgroup_size(64)" wgsl);
  assert_equal 1 (count_substring ~needle:"fn mapper(p0: u32) -> u32" wgsl);
  (* i = group*256 + j*64 + lane, four guarded chunks per entry. *)
  assert_equal 2 (count_substring ~needle:"wid.x * 256u + lane" wgsl);
  (* The four chunks are 64 apart, not four copies of the same index. *)
  List.iter
    (fun def -> assert_equal 2 (count_substring ~needle:def wgsl))
    [ "let i1 = i0 + 64u;"; "let i2 = i0 + 128u;"; "let i3 = i0 + 192u;" ];
  for j = 0 to 3 do
    assert_equal 2
      (count_substring
         ~needle:(Printf.sprintf "if (i%d < arrayLength(&input0))" j)
         wgsl);
    assert_equal 1
      (count_substring
         ~needle:(Printf.sprintf "acc = acc + mapper(input0[i%d]);" j)
         wgsl);
    assert_equal 1
      (count_substring
         ~needle:(Printf.sprintf "acc = acc + input0[i%d];" j)
         wgsl)
  done;
  assert_equal 2 (count_substring ~needle:"var acc: u32 = 0u;" wgsl);
  assert_equal 0 (count_substring ~needle:"scratch[lane] = 0u;" wgsl);
  assert_equal 14 (count_substring ~needle:"workgroupBarrier();" wgsl);
  assert_equal 2 (count_substring ~needle:"if (lane == 0u)" wgsl);
  assert_equal 2
    (count_substring ~needle:"output0[wid.x] = scratch[0u];" wgsl)

let test_x4_metadata_is_a_distinct_program _ =
  let json =
    Yojson.Safe.from_string
      (Gpu_reduce_wgsl.metadata ~variant:Gpu_reduce_wgsl.Lane4
         ~sources:[] kernel)
  in
  let open Yojson.Safe.Util in
  assert_equal Gpu_reduce.operation_x4 (json |> member "operation" |> to_string);
  assert_equal Gpu_reduce.profile_x4
    (json |> member "reduction_profile" |> to_string);
  assert_equal 64 (json |> member "group_size" |> to_int);
  assert_equal 256 (json |> member "scratch_bytes" |> to_int);
  assert_bool "the x4 artifact has its own program digest"
    (Gpu_reduce_wgsl.program_id ~variant:Gpu_reduce_wgsl.Lane4 kernel
     <> Gpu_reduce_wgsl.program_id kernel)

let test_deterministic _ =
  let a = Gpu_reduce_wgsl.render kernel in
  let b = Gpu_reduce_wgsl.render kernel in
  assert_equal a b;
  let json = Yojson.Safe.from_string (Gpu_reduce_wgsl.metadata ~sources:[] kernel) in
  assert_equal (Gpu_ir.sha256_hex a)
    (Yojson.Safe.Util.(json |> member "wgsl_sha256" |> to_string))

let test_metadata_is_a_program_not_a_plan _ =
  let json =
    Yojson.Safe.from_string (Gpu_reduce_wgsl.metadata ~sources:[] kernel)
  in
  let open Yojson.Safe.Util in
  assert_equal Gpu_reduce_wgsl.schema (json |> member "schema" |> to_string);
  assert_equal Gpu_reduce.operation (json |> member "operation" |> to_string);
  assert_equal Gpu_reduce.profile (json |> member "reduction_profile" |> to_string);
  assert_equal 64 (json |> member "group_size" |> to_int);
  assert_equal 256 (json |> member "scratch_bytes" |> to_int);
  assert_equal 0 (json |> member "identity" |> to_int);
  assert_equal 2 (json |> member "entries" |> to_list |> List.length);
  assert_bool "metadata must not carry a launch plan"
    (json |> member "passes" = `Null);
  assert_bool
    "a different mapper is a different program"
    (Gpu_reduce_wgsl.program_id kernel
     <> Gpu_reduce_wgsl.program_id (kernel_with (Gpu_ir.Return (Gpu_ir.Param 0))))

let suite =
  "GpuReduceWgslTest"
  >::: [
         "entries_and_workgroup" >:: test_entries_and_workgroup;
         "guard_and_strides" >:: test_guard_and_strides;
         "x4_layout" >:: test_x4_layout;
         "x4_metadata_is_a_distinct_program"
         >:: test_x4_metadata_is_a_distinct_program;
         "deterministic" >:: test_deterministic;
         "metadata_is_a_program_not_a_plan" >:: test_metadata_is_a_program_not_a_plan;
       ]

let () = run_test_tt_main suite
