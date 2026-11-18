(*
   GpuProofInterfaceTest — PC04.

   The reflection must come from the *actual* checked fixture and its actual
   KernelIR: the emitted artifact is compared byte-for-byte with the committed
   one, and reversing the source condition in the fixture changes the reflected
   operator (so the bound proof no longer applies). A changed IR parameter or
   comparison is refused, and `check_artifact` ignores digest edits but refuses
   a semantic mismatch.
*)

open OUnit2

let read_file path =
  let ic = open_in_bin path in
  Fun.protect ~finally:(fun () -> close_in ic) (fun () ->
      let n = in_channel_length ic in
      really_input_string ic n)

let write_file path contents =
  let oc = open_out_bin path in
  output_string oc contents;
  close_out oc

let compiled ~aui ~aum =
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.TwoFileModuleSource
      { int_filename = aui;
        int_code = read_file aui;
        body_filename = aum;
        body_code = read_file aum }
  in
  let (_, p) = Compiler.prepare_module env source in
  (p.Compiler.prepared_env, p.Compiler.prepared_mono)

let fixture ~entry =
  let aui = TestUtil.fixture "examples/gpu/pointwise/Pointwise.aui" in
  let aum = TestUtil.fixture "examples/gpu/pointwise/Pointwise.aum" in
  let (env, mono) = compiled ~aui ~aum in
  let kernel = Gpu_subset.build ~entry env mono in
  (aui, aum, mono, kernel)

let render ~aui ~aum ~mono ~kernel ~entry =
  let source = Gpu_proof_interface.of_typed ~entry mono in
  let ir = Gpu_proof_interface.of_ir ~entry kernel in
  let r = Gpu_proof_interface.witness source ir in
  let sha256 = Digestif.SHA256.(to_hex (digest_string (read_file aui ^ read_file aum))) in
  (r, Gpu_proof_interface.render ~source_sha256:sha256
        ~ir_sha256:(Gpu_ir.sha256_hex (Gpu_ir.canonical_bytes kernel)) r)

let test_emitted_artifact_is_committed _ =
  let (aui, aum, mono, kernel) = fixture ~entry:"clamp" in
  let (r, (mm0, mm1)) =
    render ~aui ~aum ~mono ~kernel ~entry:"clamp"
  in
  let committed_mm0 = read_file (TestUtil.fixture "mm0/proof_chain/clamp_source.mm0") in
  let committed_mm1 = read_file (TestUtil.fixture "mm0/proof_chain/clamp_source.mm1") in
  assert_equal "umeet" r.Gpu_proof_interface.operator;
  assert_equal ~msg:"clamp_source.mm0 is stale; regenerate with bin/gpu_proof"
    committed_mm0 mm0;
  assert_equal ~msg:"clamp_source.mm1 is stale; regenerate with bin/gpu_proof"
    committed_mm1 mm1

let with_reversed_source f =
  let dir = Filename.temp_file "pc04" ".dir" in
  Sys.remove dir;
  Unix.mkdir dir 0o700;
  let aui = Filename.concat dir "Pointwise.aui"
  and aum = Filename.concat dir "Pointwise.aum" in
  write_file aui (read_file (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aui"));
  let body = read_file (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aum") in
  let reversed =
    Str.global_replace (Str.regexp "if x > 100 then") "if x < 100 then" body
  in
  assert_bool "the reversed fixture changed" (reversed <> body);
  write_file aum reversed;
  Fun.protect
    ~finally:(fun () ->
        List.iter (fun p -> try Sys.remove p with Sys_error _ -> ()) [aui; aum];
        try Unix.rmdir dir with Unix.Unix_error _ -> ())
    (fun () -> f ~aui ~aum)

let test_reversed_source_reflects_the_join _ =
  with_reversed_source (fun ~aui ~aum ->
      let (env, mono) = compiled ~aui ~aum in
      let kernel = Gpu_subset.build ~entry:"clamp" env mono in
      let (r, (mm0, _)) = render ~aui ~aum ~mono ~kernel ~entry:"clamp" in
      assert_equal "ujoin" r.Gpu_proof_interface.operator;
      assert_bool "the emitted definition follows the reversed comparison"
        (try
           ignore
             (Str.search_forward
                (Str.regexp "def ir_clamp.*ujoin x u100") mm0 0);
           true
         with Not_found -> false))

let refuses f =
  match f () with
  | _ -> false
  | exception Gpu_proof_interface.Refused _ -> true

(* The comparison is hoisted into a `let`, so tampering rewrites that value. *)
let tamper_value f k =
  let rec go = function
    | Gpu_ir.Let (n, e) -> Gpu_ir.Let (n, f e)
    | Gpu_ir.Seq items -> Gpu_ir.Seq (List.map go items)
    | Gpu_ir.If (c, t, e) -> Gpu_ir.If (c, go t, go e)
    | s -> s
  in
  { k with Gpu_ir.body = go k.Gpu_ir.body }

let test_wrong_ir_parameter_is_refused _ =
  let (_, _, mono, kernel) = fixture ~entry:"clamp" in
  let source = Gpu_proof_interface.of_typed ~entry:"clamp" mono in
  let tampered =
    tamper_value
      (function
        | Gpu_ir.Compare (op, Gpu_ir.Param _, b) ->
           Gpu_ir.Compare (op, Gpu_ir.Param 1, b)
        | e -> e)
      kernel
  in
  assert_bool "a wrong IR parameter is refused"
    (refuses (fun () ->
         Gpu_proof_interface.witness source
           (Gpu_proof_interface.of_ir ~entry:"clamp" tampered)))

let test_changed_ir_comparison_is_refused _ =
  let (_, _, mono, kernel) = fixture ~entry:"clamp" in
  let source = Gpu_proof_interface.of_typed ~entry:"clamp" mono in
  let tampered =
    tamper_value
      (function
        | Gpu_ir.Compare (Gpu_ir.GreaterThan, a, b) ->
           Gpu_ir.Compare (Gpu_ir.LessThan, a, b)
        | e -> e)
      kernel
  in
  assert_bool "a changed IR comparison is refused"
    (refuses (fun () ->
         Gpu_proof_interface.witness source
           (Gpu_proof_interface.of_ir ~entry:"clamp" tampered)))

let test_check_artifact_ignores_digests _ =
  let (aui, aum, mono, kernel) = fixture ~entry:"clamp" in
  let (r, (mm0, _)) = render ~aui ~aum ~mono ~kernel ~entry:"clamp" in
  let edited =
    Str.global_replace (Str.regexp "source_sha256=[0-9a-f]*")
      "source_sha256=0000000000000000000000000000000000000000000000000000000000000000"
      mm0
  in
  assert_bool "editing a digest keeps the fresh reflection valid"
    (edited <> mm0);
  Gpu_proof_interface.check_artifact r edited

let test_check_artifact_refuses_a_false_correspondence _ =
  let (aui, aum, mono, kernel) = fixture ~entry:"clamp" in
  let (r, (mm0, _)) = render ~aui ~aum ~mono ~kernel ~entry:"clamp" in
  let false_artifact =
    Str.global_replace (Str.regexp "umeet x u100") "ujoin x u100" mm0
  in
  assert_bool "the artifact was mutated" (false_artifact <> mm0);
  assert_bool "a false correspondence with consistent digests is refused"
    (refuses (fun () -> Gpu_proof_interface.check_artifact r false_artifact))

let suite =
  "gpu_proof_interface" >::: [
    "emitted artifact is committed" >:: test_emitted_artifact_is_committed;
    "reversed source reflects the join" >:: test_reversed_source_reflects_the_join;
    "wrong IR parameter is refused" >:: test_wrong_ir_parameter_is_refused;
    "changed IR comparison is refused" >:: test_changed_ir_comparison_is_refused;
    "check ignores digests" >:: test_check_artifact_ignores_digests;
    "check refuses a false correspondence"
    >:: test_check_artifact_refuses_a_false_correspondence;
  ]

let () = run_test_tt_main suite
