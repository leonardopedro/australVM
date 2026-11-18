(*
   GpuFrontendTest — G01.

   The GPU target consumes the same checked frontend stages as the C/CPS
   paths. This test prepares the real pointwise fixtures through
   `Compiler.prepare_module` and observes the resolved monomorphic program:

     * the entry has Nat32 parameters and a Nat32 result;
     * `modularAdd` resolved to a method call whose source module is the
       canonical `Austral.Pervasive`;
     * preparing a module compiles and executes no JIT function;
     * a malformed fixture still fails through the ordinary frontend.

   It does not test WGSL or the subset — those are GpuSubsetTest/GpuWgslTest.
*)

open OUnit2

let read_file path =
  let ic = open_in_bin path in
  let n = in_channel_length ic in
  let s = really_input_string ic n in
  close_in ic;
  s

let prepare aui aum =
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.TwoFileModuleSource
      { int_filename = aui; int_code = read_file aui;
        body_filename = aum; body_code = read_file aum }
  in
  Compiler.prepare_module env source

let find_function name (Stages.Mtast.MonoModule (_, decls)) =
  List.find_map
    (function
      | Stages.Mtast.MFunction (_, n, params, rt, body)
        when Identifier.ident_string n = name -> Some (params, rt, body)
      | _ -> None)
    decls

let param_types params =
  List.map
    (fun (Stages.Mtast.MValueParameter (_, ty)) -> ty)
    params

let test_pointwise_signature _ =
  let (_, prepared) = prepare "examples/gpu/pointwise/Pointwise.aui"
      "examples/gpu/pointwise/Pointwise.aum" in
  match find_function "add" prepared.Compiler.prepared_mono with
  | None -> assert_failure "entry `add` not found in the monomorphic module"
  | Some (params, rt, _body) ->
     assert_equal 2 (List.length params);
     List.iter
       (fun ty ->
         match ty with
         | MonoType.MonoInteger (Type.Unsigned, Type.Width32) -> ()
         | _ -> assert_failure "pointwise `add` parameter is not Nat32")
       (param_types params);
     (match rt with
      | MonoType.MonoInteger (Type.Unsigned, Type.Width32) -> ()
      | _ -> assert_failure "pointwise `add` result is not Nat32")

(* The resolved arithmetic method is observable: its identity resolves to the
   canonical Pervasive module, not a textual name in user source. *)
let test_resolved_arithmetic _ =
  let (_, prepared) = prepare "examples/gpu/pointwise/Pointwise.aui"
      "examples/gpu/pointwise/Pointwise.aum" in
  match find_function "add" prepared.Compiler.prepared_mono with
  | None -> assert_failure "entry `add` not found"
  | Some (_, _, body) ->
     let scan (s: Stages.Mtast.mstmt) =
       let open Stages.Mtast in
       let rec expr = function
         | MConcreteMethodCall (_id, q, _args, _rt) ->
            let module_name =
              Identifier.mod_name_string (Identifier.source_module_name q)
            and method_name =
              Identifier.ident_string (Identifier.original_name q)
            in
            Some (module_name, method_name)
         | MComparison (_, a, b) | MConjunction (a, b) | MDisjunction (a, b)
           -> (match expr a with Some r -> Some r | None -> expr b)
         | MIfExpression (a, b, c) ->
            (match expr a with
             | Some r -> Some r
             | None -> (match expr b with Some r -> Some r | None -> expr c))
         | MNegation a | MCast (a, _) | MTypecast (a, _) | MDeref a -> expr a
         | _ -> None
       in
       let rec stmt = function
         | MSkip | MAssign _ | MAssignVar _ | MInitialAssign _ -> None
         | MLet (_, _, b) | MDestructure (_, _, b) -> stmt b
         | MIf (c, t, f) ->
            (match expr c with
             | Some r -> Some r
             | None -> (match stmt t with Some r -> Some r | None -> stmt f))
         | MBlock (a, b) -> (match stmt a with Some r -> Some r | None -> stmt b)
         | MReturn e | MDiscarding e | MLetTmp (_, _, e) | MAssignTmp (_, e) -> expr e
         | MWhile (c, b) ->
            (match expr c with Some r -> Some r | None -> stmt b)
         | MFor (_, a, b, body) ->
            (match expr a with
             | Some r -> Some r
             | None -> (match expr b with Some r -> Some r | None -> stmt body))
         | MCase (e, _, _) -> expr e
         | MBorrow { body; _ } -> stmt body
       in
       stmt s
     in
     (match scan body with
      | Some (module_name, method_name) ->
         assert_equal "Austral.Pervasive" module_name;
         assert_equal "modularAdd" method_name
      | None -> assert_failure "no resolved arithmetic method call in `add`")

let test_prepare_does_not_run_jit _ =
  Hashtbl.clear Compiler.jit_functions;
  let _ = prepare "examples/gpu/pointwise/Pointwise.aui"
      "examples/gpu/pointwise/Pointwise.aum" in
  assert_equal 0 (Hashtbl.length Compiler.jit_functions)

let test_malformed_fixture_fails _ =
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.TwoFileModuleSource
      { int_filename = "Bad.aui";
        int_code = "module Bad is\n  function bad(x: Nat32): Nat32;\nend module.\n";
        body_filename = "Bad.aum";
        body_code =
          "module body Bad is\n  function bad(x: Nat32): Nat32 is\n    \
           return true;\n  end;\nend module body.\n" }
  in
  match Compiler.prepare_module env source with
  | exception Error.Austral_error _ -> ()
  | exception _ -> ()
  | _ -> assert_failure "a body returning Boolean as Nat32 was accepted"

let suite =
  "GpuFrontendTest" >::: [
    "pointwise signature is Nat32 -> Nat32" >:: test_pointwise_signature;
    "arithmetic resolves to canonical Pervasive" >:: test_resolved_arithmetic;
    "preparing a module runs no JIT function" >:: test_prepare_does_not_run_jit;
    "malformed fixture fails normally" >:: test_malformed_fixture_fails;
  ]

let () = run_test_tt_main suite
