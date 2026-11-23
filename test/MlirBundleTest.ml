(*
   G20b: the published MLIR artifact is a chained set, and the checker refuses
   a set that is not. These tests pin the pure metadata module — extraction of
   the embedded PTX, the ABI expectation, the derivative chain and its
   tamper refusals — and the CLI's shape and option refusals. The end-to-end
   publication is exercised by `scripts/mlir-bundle`, which runs the compiler.
*)
open OUnit2
open CliUtil
open CliParser

let sha = Gpu_ir.sha256_hex

(* A lowered-module excerpt with the shape `mlir-opt` prints: the assembly is a
   quoted string whose newlines are `\0A` escapes, not real newlines. *)
let lowered_sample =
  let eol = "\\0A" in
  "module attributes {} {\n  gpu.binary @kern [#gpu.object<#nvvm.target, \
   properties {   }, assembly = \"" ^ eol ^ ".version 6.0" ^ eol ^ ".target sm_70" ^ eol
  ^ ".address_size 64" ^ eol ^ eol ^ ".visible .entry main(" ^ eol
  ^ "\t.param .u64 main_param_0," ^ eol ^ "\t.param .u64 main_param_1" ^ eol
  ^ ")" ^ eol ^ "{" ^ eol ^ "\tcvta.to.global.u64 \t%rd1, %rd0;" ^ eol
  ^ "}" ^ eol ^ eol ^ "}]\n}\n"

let test_extract_ptx _ =
  match Gpu_mlir_bundle.extract_assembly lowered_sample with
  | None -> assert_failure "no assembly extracted"
  | Some ptx ->
     assert_bool "target line survives the unescape"
       (Gpu_mlir_bundle.contains ptx ".target sm_70");
     assert_bool "newlines are decoded"
       (Gpu_mlir_bundle.contains ptx "\n.visible .entry main(");
     assert_equal 2 (Gpu_mlir_bundle.ptx_facts ptx).Gpu_mlir_bundle.ptx_params

let test_extract_ptx_absent _ =
  assert_equal None
    (Gpu_mlir_bundle.extract_assembly "module { }")

let test_abi_accepts_one_memref _ =
  let facts =
    { Gpu_mlir_bundle.ptx_entry = "main"; ptx_target = "sm_70";
      ptx_address_size = "64"; ptx_params = 10;
      ptx_param_types = ["u64"];
      ptx_address_space = "generic -> global (cvta.to.global)" }
  in
  assert_equal [] (Gpu_mlir_bundle.abi_mismatches ~arity:1 facts);
  assert_equal 10 (Gpu_mlir_bundle.expected_params ~arity:1)

let test_abi_refuses_each_disagreement _ =
  let base =
    { Gpu_mlir_bundle.ptx_entry = "main"; ptx_target = "sm_70";
      ptx_address_size = "64"; ptx_params = 10;
      ptx_param_types = ["u64"];
      ptx_address_space = "generic -> global (cvta.to.global)" }
  in
  let mutations =
    [ ("wrong target", { base with Gpu_mlir_bundle.ptx_target = "sm_50" });
      ("wrong params", { base with Gpu_mlir_bundle.ptx_params = 5 });
      ("wrong entry", { base with Gpu_mlir_bundle.ptx_entry = "copy" });
      ("wrong width", { base with Gpu_mlir_bundle.ptx_param_types = ["u32"] });
      ("no conversion", { base with Gpu_mlir_bundle.ptx_address_space = "none" }) ]
  in
  List.iter (fun (label, facts) ->
    assert_bool label (Gpu_mlir_bundle.abi_mismatches ~arity:1 facts <> []))
    mutations

(* A complete synthetic published set, written to a temp dir. *)
let write_set ?(ptx_tail = "") ?(kernelir = "{\"entry\":\"main\"}") dir =
  let mlir =
    Printf.sprintf
      "// kernelir-sha256: %s\nmodule attributes {} {\n  gpu.module @main {}\n}\n"
      (sha kernelir)
  in
  let ptx =
    ".version 6.0\n.target sm_70\n.address_size 64\n\n.visible .entry main(\n\
     \t.param .u64 p0,\n\t.param .u64 p1,\n\t.param .u64 p2,\n\t.param .u64 p3,\n\
     \t.param .u64 p4,\n\t.param .u64 p5,\n\t.param .u64 p6,\n\t.param .u64 p7,\n\
     \t.param .u64 p8,\n\t.param .u64 p9\n)\n{\n\tcvta.to.global.u64 %rd1, %rd0;\n}\n"
    ^ ptx_tail
  in
  let json =
    Printf.sprintf
      {|{"schema":"austral.gpu.mlir.v1","entry":"main","input_arity":1,"extent":65,
         "numeric_profile":"u32-modular-v1","sources":[],
         "derivatives":[{"stage":"kernelir","file":"kernel.kernelir","sha256":"%s"},
                        {"stage":"mlir","file":"kernel.mlir","sha256":"%s"},
                        {"stage":"ptx","file":"kernel.ptx","sha256":"%s"}],
         "lowering":{"target":"sm_70","pipeline":"%s","toolchain":"mlir-opt 19.1.7",
                     "ptxas":"not used"},
         "abi":{"entry":"main","params":10},"numeric_contract":null}|}
      (sha kernelir) (sha mlir) (sha ptx) Gpu_mlir_bundle.pipeline
  in
  List.iter (fun (name, body) ->
    let oc = open_out_bin (Filename.concat dir name) in
    output_string oc body; close_out oc)
    [ ("kernel.kernelir", kernelir); ("kernel.mlir", mlir);
      ("kernel.ptx", ptx); ("bundle.json", json) ]

let with_dir f =
  let dir = Filename.temp_file "avm-mlir-bundle-" "" in
  Sys.remove dir;
  Unix.mkdir dir 0o700;
  Fun.protect
    ~finally:(fun () ->
      Array.iter (fun name -> try Sys.remove (Filename.concat dir name)
                  with Sys_error _ -> ())
        (Sys.readdir dir);
      try Unix.rmdir dir with Unix.Unix_error _ -> ())
    (fun () -> f dir)

let test_verify_accepts_consistent_set _ =
  with_dir (fun dir ->
    write_set dir;
    match Gpu_mlir_bundle.verify_dir dir with
    | Ok (b, _) -> assert_equal 10 b.Gpu_mlir_bundle.abi.Gpu_mlir_bundle.ptx_params
    | Error problems ->
       assert_failure ("refused a consistent set: " ^ String.concat "; " problems))

let rewrite dir name f =
  let path = Filename.concat dir name in
  let ic = open_in_bin path in
  let n = in_channel_length ic in
  let s = really_input_string ic n in
  close_in ic;
  let oc = open_out_bin path in
  output_string oc (f s); close_out oc

let test_verify_refuses_tampered_ptx _ =
  with_dir (fun dir ->
    write_set dir;
    rewrite dir "kernel.ptx" (fun s -> s ^ "\\0A.note tampered\n");
    match Gpu_mlir_bundle.verify_dir dir with
    | Ok _ -> assert_failure "a tampered PTX was accepted"
    | Error problems ->
       assert_bool "the refusal names the ptx stage"
         (List.exists (fun p -> Gpu_mlir_bundle.contains p "ptx") problems))

let test_verify_refuses_rehashed_kernelir _ =
  (* The interesting tamper: the digest in bundle.json is recomputed, so the
     derivative hash agrees and only the emitter's own header disagrees. *)
  with_dir (fun dir ->
    write_set dir;
    let forged = "{\"entry\":\"forged\"}" in
    rewrite dir "kernel.kernelir" (fun _ -> forged);
    rewrite dir "bundle.json" (fun s ->
      Str.global_replace (Str.regexp "\"sha256\":\"[0-9a-f]+\"")
        ("\"sha256\":\"" ^ sha forged ^ "\"") s);
    match Gpu_mlir_bundle.verify_dir dir with
    | Ok _ -> assert_failure "a re-hashed kernelir slip past the header check"
    | Error problems ->
       assert_bool "the refusal names the pair"
         (List.exists (fun p -> Gpu_mlir_bundle.contains p "rendered from")
            problems))

let test_verify_refuses_a_foreign_pipeline _ =
  with_dir (fun dir ->
    write_set dir;
    rewrite dir "bundle.json" (fun s ->
      Str.global_replace (Str.regexp_string Gpu_mlir_bundle.pipeline)
        "--gpu-lower-to-nvvm-pipeline=cubin-format=isa cubin-chip=sm_50" s);
    match Gpu_mlir_bundle.verify_dir dir with
    | Ok _ -> assert_failure "a non-pinned pipeline was accepted"
    | Error _ -> ())

(* ---------------------------------------------------------------- *)
(* CLI                                                               *)
(* ---------------------------------------------------------------- *)

let parse_cmd (args: string list): cmd = parse (parse_args args)

(* The parser raises `Austral_error` with kind `CliError`; the tests care that
   the command is refused before any publication, not about the wording. *)
let assert_refused (args: string list) =
  match parse_cmd args with
  | exception Error.Austral_error _ -> ()
  | _ -> assert_failure ("expected a refusal for: " ^ String.concat " " args)

let test_mlir_target_shape _ =
  match parse_cmd [ "austral"; "compile"; "Foo.aum"; "--target-type=mlir";
                    "--entrypoint=Foo:main"; "--output=out" ] with
  | WholeProgramCompile { target = MlirBundle { output_dir; extent; _ }; _ } ->
     assert_equal "out" output_dir;
     assert_equal 65 extent
  | _ -> assert_failure "expected an MlirBundle target"

let test_mlir_extent_is_explicit _ =
  match parse_cmd [ "austral"; "compile"; "Foo.aum"; "--target-type=mlir";
                    "--gpu-static-elements=257"; "--entrypoint=Foo:main";
                    "--output=out" ] with
  | WholeProgramCompile { target = MlirBundle { extent; _ }; _ } ->
     assert_equal 257 extent
  | _ -> assert_failure "expected an MlirBundle target"

let test_mlir_refuses_non_positive_extent _ =
  List.iter (fun raw ->
    assert_refused [ "austral"; "compile"; "Foo.aum"; "--target-type=mlir";
                     "--gpu-static-elements=" ^ raw; "--entrypoint=Foo:main";
                     "--output=out" ])
    ["0"; "-3"; "sixty-five"]

let test_mlir_refuses_the_f32_profile _ =
  assert_refused [ "austral"; "compile"; "Foo.aum"; "--target-type=mlir";
                   "--gpu-numeric-profile=wgsl-f32-v1";
                   "--entrypoint=Foo:main"; "--output=out" ]

let test_mlir_refuses_a_reduction _ =
  assert_refused [ "austral"; "compile"; "Foo.aum"; "--target-type=mlir";
                   "--gpu-operation=map-sum-u32";
                   "--entrypoint=Foo:main"; "--output=out" ]

let suite =
  "MlirBundleTest" >::: [
    "extract ptx" >:: test_extract_ptx;
    "extract ptx absent" >:: test_extract_ptx_absent;
    "abi accepts one memref" >:: test_abi_accepts_one_memref;
    "abi refuses each disagreement" >:: test_abi_refuses_each_disagreement;
    "verify accepts a consistent set" >:: test_verify_accepts_consistent_set;
    "verify refuses tampered ptx" >:: test_verify_refuses_tampered_ptx;
    "verify refuses re-hashed kernelir" >:: test_verify_refuses_rehashed_kernelir;
    "verify refuses a foreign pipeline" >:: test_verify_refuses_a_foreign_pipeline;
    "cli shape" >:: test_mlir_target_shape;
    "cli explicit extent" >:: test_mlir_extent_is_explicit;
    "cli refuses non-positive extent" >:: test_mlir_refuses_non_positive_extent;
    "cli refuses the f32 profile" >:: test_mlir_refuses_the_f32_profile;
    "cli refuses a reduction" >:: test_mlir_refuses_a_reduction;
  ]

let () = run_test_tt_main suite
