(*
   G05: `--target-type=wgsl` is one selected entry function and one output
   directory. These tests pin the parser's shape and its refusals; the engine's
   publication path is exercised end to end by `scripts/gpu-source-check`.
*)
open OUnit2
open Identifier
open CliUtil
open CliParser

let parse_cmd (args: string list): cmd = parse (parse_args args)

let test_wgsl_target _ =
  let cmd =
    parse_cmd
      [ "austral"; "compile"; "Foo.aum"; "--target-type=wgsl";
        "--entrypoint=Foo:main"; "--output=out" ]
  in
  match cmd with
  | WholeProgramCompile
      { target = WgslBundle { output_dir; entrypoint; numeric_profile; _ }; modules; _ } ->
     assert_equal "out" output_dir;
     assert_equal
       (Entrypoint (make_mod_name "Foo", make_ident "main"))
       entrypoint;
     assert_equal Gpu_ir.U32ModularV1 numeric_profile;
     assert_equal 1 (List.length modules)
  | _ -> assert_failure "expected a WgslBundle target"

let test_wgsl_f32_profile_is_opt_in _ =
  let cmd =
    parse_cmd
      [ "austral"; "compile"; "Foo.aum"; "--target-type=wgsl";
        "--gpu-numeric-profile=wgsl-f32-v1";
        "--entrypoint=Foo:main"; "--output=out" ]
  in
  match cmd with
  | WholeProgramCompile { target = WgslBundle { numeric_profile; _ }; _ } ->
     assert_equal Gpu_ir.WgslF32V1 numeric_profile
  | _ -> assert_failure "expected a WgslBundle target"

let refused f =
  match f () with
  | _ -> false
  | exception Error.Austral_error _ -> true

let test_profile_requires_the_gpu_target _ =
  assert_bool "an unknown profile value is a CLI error"
    (refused (fun () ->
       parse_cmd
         [ "austral"; "compile"; "Foo.aum"; "--target-type=wgsl";
           "--gpu-numeric-profile=f32";
           "--entrypoint=Foo:main"; "--output=out" ]));
  assert_bool "the profile is refused without --target-type=wgsl"
    (refused (fun () ->
       parse_cmd
         [ "austral"; "compile"; "Foo.aum"; "--target-type=tc";
           "--gpu-numeric-profile=wgsl-f32-v1" ]))

let test_bounded_fold_source_profile_is_opt_in _ =
  let cmd =
    parse_cmd
      [ "austral"; "compile"; "Foo.aum"; "--target-type=wgsl";
        "--gpu-source-profile=bounded-fold-v1";
        "--entrypoint=Foo:main"; "--output=out" ]
  in
  (match cmd with
   | WholeProgramCompile
       { target = WgslBundle { source_profile; numeric_profile; _ }; _ } ->
      assert_equal Gpu_subset.BoundedFoldV1 source_profile;
      (* The source profile does not change the numeric contract. *)
      assert_equal Gpu_ir.U32ModularV1 numeric_profile
   | _ -> assert_failure "expected a WgslBundle target");
  assert_bool "an unknown source profile is a CLI error"
    (refused (fun () ->
       parse_cmd
         [ "austral"; "compile"; "Foo.aum"; "--target-type=wgsl";
           "--gpu-source-profile=bounded-fold-v2";
           "--entrypoint=Foo:main"; "--output=out" ]));
  assert_bool "the source profile is refused without --target-type=wgsl"
    (refused (fun () ->
       parse_cmd
         [ "austral"; "compile"; "Foo.aum"; "--target-type=tc";
           "--gpu-source-profile=bounded-fold-v1" ]))

let test_wgsl_requires_entrypoint _ =
  assert_bool "--no-entrypoint must be refused"
    (refused (fun () ->
       parse_cmd
         [ "austral"; "compile"; "Foo.aum"; "--target-type=wgsl";
           "--no-entrypoint"; "--output=out" ]))

let test_unknown_target_names_wgsl _ =
  assert_bool "unknown targets are a CLI error"
    (refused (fun () ->
       parse_cmd
         [ "austral"; "compile"; "Foo.aum"; "--target-type=metal";
           "--entrypoint=Foo:main"; "--output=out" ]))

let suite =
  "GpuCli" >::: [
      "wgsl target carries entrypoint and output directory" >:: test_wgsl_target;
      "wgsl-f32-v1 is an explicit opt-in" >:: test_wgsl_f32_profile_is_opt_in;
      "bounded-fold-v1 is an explicit source opt-in" >:: test_bounded_fold_source_profile_is_opt_in;
      "the numeric profile requires the wgsl target" >:: test_profile_requires_the_gpu_target;
      "--no-entrypoint is refused for wgsl" >:: test_wgsl_requires_entrypoint;
      "unknown target is a CLI error" >:: test_unknown_target_names_wgsl;
    ]

let () = run_test_tt_main suite
