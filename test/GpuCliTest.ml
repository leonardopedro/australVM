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
  | WholeProgramCompile { target = WgslBundle { output_dir; entrypoint }; modules; _ } ->
     assert_equal "out" output_dir;
     assert_equal
       (Entrypoint (make_mod_name "Foo", make_ident "main"))
       entrypoint;
     assert_equal 1 (List.length modules)
  | _ -> assert_failure "expected a WgslBundle target"

let refused f =
  match f () with
  | _ -> false
  | exception Error.Austral_error _ -> true

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
      "--no-entrypoint is refused for wgsl" >:: test_wgsl_requires_entrypoint;
      "unknown target is a CLI error" >:: test_unknown_target_names_wgsl;
    ]

let () = run_test_tt_main suite
