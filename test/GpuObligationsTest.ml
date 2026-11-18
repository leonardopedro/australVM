(*
   GpuObligationsTest — G13.

   The obligations must come from the actual bundle, not from a retyped plan.
   These tests build a real bundle for the pointwise fixture, then check that
   the replay accepts it and that a changed memory plan, launch extent or
   shader byte is refused. The WhyML goals themselves are checked by
   `scripts/gpu-obligations` with Why3; this test only asserts their shape and
   that no axiom is emitted.
*)

open OUnit2

let read_file path =
  let ic = open_in_bin path in
  let n = in_channel_length ic in
  let s = really_input_string ic n in
  close_in ic;
  s

let write_file path contents =
  let oc = open_out_bin path in
  output_string oc contents;
  close_out oc

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

let with_bundle f =
  let dir = Filename.temp_file "gpu-obligations" ".avkg" in
  Sys.remove dir;
  Unix.mkdir dir 0o700;
  Fun.protect
    ~finally:(fun () ->
      List.iter (fun name ->
          try Sys.remove (Filename.concat dir name) with Sys_error _ -> ())
        ["kernel.json"; "kernel.wgsl"];
      (try Unix.rmdir dir with Unix.Unix_error _ -> ()))
    (fun () ->
      let (env, mono) = prepared () in
      let kernel = Gpu_subset.build ~entry:"add" env mono in
      let wgsl = Gpu_wgsl.render kernel in
      let bundle = Gpu_bundle.make ~wgsl ~sources:[] kernel in
      write_file (Filename.concat dir "kernel.json")
        (Gpu_bundle.render_kernel_json bundle);
      write_file (Filename.concat dir "kernel.wgsl") wgsl;
      f dir)

let error f =
  match f () with
  | _ -> None
  | exception Gpu_obligations.Error msg -> Some msg

let test_accepts_actual_bundle _ =
  with_bundle (fun dir ->
      let t = Gpu_obligations.of_bundle dir in
      assert_equal "add" t.Gpu_obligations.entry;
      assert_equal 2 t.Gpu_obligations.input_arity;
      assert_equal 64 t.Gpu_obligations.workgroup_size;
      assert_equal 1048576 t.Gpu_obligations.max_elements;
      let whyml = Gpu_obligations.render_whyml t in
      assert_bool "eight goals are emitted"
        (List.for_all
           (fun goal -> Gpu_obligations.contains whyml goal)
           ["goal groups_nonneg"; "goal index_representable";
            "goal guard_implies_bound"; "goal flatten_injective";
            "goal distinct_active_writes"; "goal write_index_unique"]);
      assert_bool "the WhyML file has no axiom declaration"
        (not (Gpu_obligations.contains whyml "\naxiom "));
      assert_bool "the WhyML file has no theorem declaration"
        (not (Gpu_obligations.contains whyml "\ntheorem ")))

let test_refuses_changed_plan _ =
  with_bundle (fun dir ->
      let json =
        Yojson.Safe.from_file (Filename.concat dir "kernel.json") in
      let open Yojson.Safe.Util in
      let kernel = member "kernel_ir" json in
      let memory = member "memory" kernel in
      let changed =
        `Assoc (List.map
                  (fun (k, v) ->
                    if k = "output_index" then (k, `String "linear") else (k, v))
                  (to_assoc memory)) in
      let json =
        `Assoc (List.map
                  (fun (k, v) ->
                    if k = "kernel_ir" then
                      (k, `Assoc (List.map
                                    (fun (kk, vv) ->
                                      if kk = "memory" then (kk, changed) else (kk, vv))
                                    (to_assoc kernel)))
                    else (k, v))
                  (to_assoc json)) in
      write_file (Filename.concat dir "kernel.json") (Yojson.Safe.to_string json);
      match error (fun () -> Gpu_obligations.of_bundle dir) with
      | Some _ -> ()
      | None -> assert_failure "a non-identity output index was accepted")

let test_refuses_removed_guard _ =
  with_bundle (fun dir ->
      let json =
        Yojson.Safe.from_file (Filename.concat dir "kernel.json") in
      let open Yojson.Safe.Util in
      let kernel = member "kernel_ir" json in
      let memory = member "memory" kernel in
      let changed =
        `Assoc (List.map
                  (fun (k, v) ->
                    if k = "guard_before_load" then (k, `Bool false) else (k, v))
                  (to_assoc memory)) in
      let json =
        `Assoc (List.map
                  (fun (k, v) ->
                    if k = "kernel_ir" then
                      (k, `Assoc (List.map
                                    (fun (kk, vv) ->
                                      if kk = "memory" then (kk, changed) else (kk, vv))
                                    (to_assoc kernel)))
                    else (k, v))
                  (to_assoc json)) in
      write_file (Filename.concat dir "kernel.json") (Yojson.Safe.to_string json);
      match error (fun () -> Gpu_obligations.of_bundle dir) with
      | Some _ -> ()
      | None -> assert_failure "a removed pre-load guard was accepted")

let test_refuses_changed_extent _ =
  with_bundle (fun dir ->
      let json =
        Yojson.Safe.from_file (Filename.concat dir "kernel.json") in
      let open Yojson.Safe.Util in
      let req = member "requirements" json in
      let changed =
        `Assoc (List.map
                  (fun (k, v) ->
                    if k = "max_elements" then (k, `Int 1048575) else (k, v))
                  (to_assoc req)) in
      let json =
        `Assoc (List.map
                  (fun (k, v) ->
                    if k = "requirements" then (k, changed) else (k, v))
                  (to_assoc json)) in
      write_file (Filename.concat dir "kernel.json") (Yojson.Safe.to_string json);
      match error (fun () -> Gpu_obligations.of_bundle dir) with
      | Some _ -> ()
      | None -> assert_failure "a changed launch extent was accepted")

let test_refuses_tampered_shader _ =
  with_bundle (fun dir ->
      let path = Filename.concat dir "kernel.wgsl" in
      write_file path (read_file path ^ "// tampered\n");
      match error (fun () -> Gpu_obligations.of_bundle dir) with
      | Some _ -> ()
      | None -> assert_failure "a shader digest mismatch was accepted")

let suite =
  "GpuObligationsTest" >::: [
    "accepts the actual compiled bundle" >:: test_accepts_actual_bundle;
    "refuses a changed memory plan" >:: test_refuses_changed_plan;
    "refuses a removed guard" >:: test_refuses_removed_guard;
    "refuses a changed launch extent" >:: test_refuses_changed_extent;
    "refuses a tampered shader" >:: test_refuses_tampered_shader;
  ]

let () = run_test_tt_main suite
