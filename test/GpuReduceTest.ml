(*
   GpuReduceTest — G22a.

   The `map-sum-u32` schedule is arithmetic, so it is checked as arithmetic:
   the pass counts for the sizes the exit criteria name, the identity and the
   empty case, the byte capacities, the refusals, and that the `(group, lane)`
   partition covers every valid index exactly once.

   The C mapper and the Rust schedule are compared by `scripts/gpu-reduce-u32`
   and `safestos/gpu`'s tests respectively; this suite owns the definition both
   of them read.
*)

open OUnit2

let ok = function Ok v -> v | Error e -> failwith (Gpu_reduce.error_message e)

let test_schedule _ =
  let cases =
    [
      (0, []);
      (1, [ 1 ]);
      (63, [ 1 ]);
      (64, [ 1 ]);
      (65, [ 2; 1 ]);
      (4095, [ 64; 1 ]);
      (4096, [ 64; 1 ]);
      (4097, [ 65; 2; 1 ]);
      (1_048_576, [ 16_384; 256; 4; 1 ]);
    ]
  in
  List.iter
    (fun (n, expected) ->
      assert_equal ~msg:(Printf.sprintf "schedule %d" n) expected
        (ok (Gpu_reduce.schedule n)))
    cases

let test_pass_count_at_most_four _ =
  let max = Gpu_reduce.max_inputs in
  assert_equal 4 (Gpu_reduce.pass_count max);
  List.iter
    (fun n ->
      assert_bool (Printf.sprintf "%d needs more than four passes" n)
        (Gpu_reduce.pass_count n <= 4))
    [ 1; 63; 64; 65; 4095; 4096; 4097; max ]

let test_empty_is_identity _ =
  assert_equal (Ok []) (Gpu_reduce.schedule 0);
  assert_equal 0 (Gpu_reduce.identity);
  (* No pass means no binding: the launch plan records zero passes. *)
  assert_equal 0 (Gpu_reduce.pass_count 0)

let test_schedule_x4 _ =
  let cases =
    [
      (0, []);
      (1, [ 1 ]);
      (65, [ 1 ]);
      (255, [ 1 ]);
      (256, [ 1 ]);
      (257, [ 2; 1 ]);
      (4096, [ 16; 1 ]);
      (4097, [ 17; 1 ]);
      (1_048_576, [ 4096; 16; 1 ]);
    ]
  in
  List.iter
    (fun (n, expected) ->
      assert_equal ~msg:(Printf.sprintf "x4 schedule %d" n) expected
        (ok (Gpu_reduce.schedule_x4 n)))
    cases;
  assert_equal 3 (Gpu_reduce.pass_count_x4 Gpu_reduce.max_inputs);
  match Gpu_reduce.schedule_x4 (-1) with
  | Error (Gpu_reduce.Negative_input (-1)) -> ()
  | _ -> failwith "x4 negative input was not refused"

let test_x4_total_bytes _ =
  (* 4096 + 16 + 1 pass outputs, plus the caller's input. *)
  assert_equal (4_194_304 + ((4096 + 16 + 1) * 4))
    (ok (Gpu_reduce.total_bytes_x4 Gpu_reduce.max_inputs))

let x4_partition_covers n =
  let seen = Array.make n 0 in
  let groups = (n + Gpu_reduce.group_elements - 1) / Gpu_reduce.group_elements in
  for group = 0 to groups - 1 do
    for j = 0 to Gpu_reduce.elements_per_lane - 1 do
      for lane = 0 to Gpu_reduce.tile - 1 do
        match Gpu_reduce.partition_index_x4 ~group ~lane ~j with
        | Some i when i >= 0 && i < n -> seen.(i) <- seen.(i) + 1
        | _ -> ()
      done
    done
  done;
  Array.for_all (fun count -> count = 1) seen

let test_x4_partition_covers_every_index_once _ =
  assert_bool "empty input has no x4 index to cover" (x4_partition_covers 0);
  List.iter
    (fun n ->
      assert_bool
        (Printf.sprintf "x4 partition misses or repeats an index at %d" n)
        (x4_partition_covers n))
    [ 1; 63; 64; 65; 255; 256; 257; 4095; 4096; 4097; 1_048_576 ];
  assert_equal (Some 0) (Gpu_reduce.partition_index_x4 ~group:0 ~lane:0 ~j:0);
  assert_equal (Some 63) (Gpu_reduce.partition_index_x4 ~group:0 ~lane:63 ~j:0);
  assert_equal (Some 64) (Gpu_reduce.partition_index_x4 ~group:0 ~lane:0 ~j:1);
  assert_equal (Some 256) (Gpu_reduce.partition_index_x4 ~group:1 ~lane:0 ~j:0);
  assert_equal None (Gpu_reduce.partition_index_x4 ~group:0 ~lane:0 ~j:4)

let test_refusals _ =
  (match Gpu_reduce.schedule (-1) with
  | Error (Gpu_reduce.Negative_input (-1)) -> ()
  | other ->
      failwith
        (Printf.sprintf "negative input was not refused: %s"
           (match other with Ok _ -> "Ok" | Error e -> Gpu_reduce.error_message e)));
  (match Gpu_reduce.schedule (Gpu_reduce.max_inputs + 1) with
  | Error (Gpu_reduce.Oversized_input n) when n = Gpu_reduce.max_inputs + 1 -> ()
  | _ -> failwith "oversized input was not refused");
  match Gpu_reduce.capacity_bytes (Gpu_reduce.max_inputs + 1) with
  | Error (Gpu_reduce.Oversized_input _) -> ()
  | _ -> failwith "oversized capacity was not refused"

let test_capacities _ =
  assert_equal 0 (ok (Gpu_reduce.capacity_bytes 0));
  assert_equal 4 (ok (Gpu_reduce.capacity_bytes 1));
  assert_equal 256 (ok (Gpu_reduce.capacity_bytes Gpu_reduce.tile));
  assert_equal 4_194_304
    (ok (Gpu_reduce.capacity_bytes Gpu_reduce.max_inputs));
  (* 16_384 + 256 + 4 + 1 pass outputs, plus the caller's input. *)
  assert_equal (4_194_304 + ((16_384 + 256 + 4 + 1) * 4))
    (ok (Gpu_reduce.total_bytes Gpu_reduce.max_inputs));
  (* Scratch is one workgroup of u32 words, independent of the input size. *)
  assert_equal (Gpu_reduce.tile * 4) Gpu_reduce.scratch_bytes

let partition_covers n =
  let seen = Array.make n 0 in
  let groups = (n + Gpu_reduce.tile - 1) / Gpu_reduce.tile in
  for group = 0 to groups - 1 do
    for lane = 0 to Gpu_reduce.tile - 1 do
      match Gpu_reduce.partition_index ~group ~lane with
      | Some i when i >= 0 && i < n -> seen.(i) <- seen.(i) + 1
      | _ -> ()
    done
  done;
  Array.for_all (fun count -> count = 1) seen

let test_partition_covers_every_index_once _ =
  assert_bool "empty input has no index to cover" (partition_covers 0);
  List.iter
    (fun n ->
      assert_bool
        (Printf.sprintf "partition misses or repeats an index at %d" n)
        (partition_covers n))
    [ 1; 63; 64; 65; 4095; 4096; 4097; 1_048_576 ]

let test_partition_shape _ =
  assert_equal (Some 0) (Gpu_reduce.partition_index ~group:0 ~lane:0);
  assert_equal (Some 63) (Gpu_reduce.partition_index ~group:0 ~lane:63);
  assert_equal (Some 64) (Gpu_reduce.partition_index ~group:1 ~lane:0);
  assert_equal (Some 65) (Gpu_reduce.partition_index ~group:1 ~lane:1);
  assert_equal None (Gpu_reduce.partition_index ~group:0 ~lane:64);
  assert_equal None (Gpu_reduce.partition_index ~group:0 ~lane:(-1));
  assert_equal None (Gpu_reduce.partition_index ~group:(-1) ~lane:0)

let test_identities _ =
  assert_bool "program id is empty" (String.length Gpu_reduce.program_id = 64);
  let plan n = ok (Gpu_reduce.launch_plan_id n) in
  let zero = plan 0 and one = plan 1 and tile = plan 64 and max = plan 1_048_576 in
  assert_bool "launch plan id does not depend on the input length"
    (zero <> one && one <> tile && tile <> max);
  assert_bool "program and launch plan share an identity"
    (Gpu_reduce.program_id <> zero);
  assert_bool "launch plan id is empty" (String.length max = 64);
  assert_bool "the x4 program is a different program"
    (Gpu_reduce.program_id_x4 <> Gpu_reduce.program_id);
  assert_bool "the x4 launch plan id is empty"
    (String.length (ok (Gpu_reduce.launch_plan_id_x4 257)) = 64);
  let text = ok (Gpu_reduce.launch_plan_text 65) in
  assert_bool "launch plan omits the program id"
    (String.contains text '\n' && String.length text > 0)

let suite =
  "GpuReduceTest"
  >::: [
         "schedule" >:: test_schedule;
         "x4 schedule" >:: test_schedule_x4;
         "x4 capacities" >:: test_x4_total_bytes;
         "x4 partition covers each index once"
         >:: test_x4_partition_covers_every_index_once;
         "at most four passes" >:: test_pass_count_at_most_four;
         "empty is identity" >:: test_empty_is_identity;
         "refusals" >:: test_refusals;
         "capacities" >:: test_capacities;
         "partition covers each index once"
         >:: test_partition_covers_every_index_once;
         "partition shape" >:: test_partition_shape;
         "identities" >:: test_identities;
       ]

let () = run_test_tt_main suite
