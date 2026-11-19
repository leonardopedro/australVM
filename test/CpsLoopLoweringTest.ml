(*
   The `for` loop's endpoint is part of the language specification: Austral's
   `for i from A to B` executes `B - A + 1` times, and the C renderer emits
   `i <= B` (`CRenderer.CFor`). This lowering used to emit `CmpLt`, so the CPS
   backend ran one iteration fewer and `for i from 0 to 0` did nothing.

   These tests pin the relation on the emitted CPS, both from a hand-built
   monomorphic statement and from real Austral source, so the endpoint cannot
   drift back to the exclusive one without a red suite.
*)

open OUnit2

let source_module ~name functions =
  let aum =
    Printf.sprintf "module body %s is\n" name
    ^ String.concat "" (List.map (fun body -> "    " ^ body ^ "\n") functions)
    ^ "end module body.\n"
  in
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.BodyModuleSource { body_filename = name ^ ".aum"; body_code = aum }
  in
  match Compiler.prepare_module env source with
  | (_, p) -> (p.Compiler.prepared_env, p.Compiler.prepared_mono)
  | exception Error.Austral_error e ->
     assert_failure
       ("loop fixture failed the frontend:\n" ^ Error.render_error_to_plain e)

(* The body the fixture's loop lowers to; find the one While it contains. *)
let rec find_while (s: CpsGen.cps_stmt): CpsGen.cps_expr option =
  match s with
  | CpsGen.While (cond, _) -> Some cond
  | CpsGen.Block (a, b) ->
     (match find_while a with Some c -> Some c | None -> find_while b)
  | CpsGen.If (_, a, b) ->
     (match find_while a with Some c -> Some c | None -> find_while b)
  | CpsGen.Let (_, _, body) -> find_while body
  | CpsGen.Match (_, cases, default) ->
     let rec first = function
       | [] -> find_while default
       | (_, s) :: rest ->
          (match find_while s with Some c -> Some c | None -> first rest)
     in
     first cases
  | _ -> None

let lowered_condition ~entry functions =
  let (env, mono) = source_module ~name:"Loops" functions in
  ignore env;
  let (funcs, _) = Compiler_cps.compile_module_cps mono in
  let fn =
    match List.find_opt (fun (f: CpsGen.function_def) -> f.CpsGen.name = entry) funcs with
    | Some f -> f
    | None -> assert_failure ("no CPS function named " ^ entry)
  in
  match find_while fn.CpsGen.body with
  | Some cond -> cond
  | None -> assert_failure (entry ^ " lowered without a loop")

let assert_inclusive ~entry functions =
  match lowered_condition ~entry functions with
  | CpsGen.CmpLte _ -> ()
  | CpsGen.CmpLt _ ->
     assert_failure
       (entry
        ^ " lowers its `for` to `CmpLt`: the CPS backend would run one \
           iteration fewer than the specification and C (`i <= K`)")
  | other ->
     assert_failure
       (entry ^ " does not lower its `for` to a comparison: "
        ^ CpsGen.string_of_expr other)

let loop_fixture =
  [ "function countZero(): Nat32 is\n\
    \    let one: Nat32 := 1;\n\
    \    var acc: Nat32 := 0;\n\
    \    for i from 0 to 0 do\n\
    \        acc := trappingSubtract(acc, one);\n\
    \    end for;\n\
    \    return acc;\n\
     end;";
    "function countTen(): Nat32 is\n\
    \    let one: Nat32 := 1;\n\
    \    var acc: Nat32 := 0;\n\
    \    for i from 0 to 10 do\n\
    \        acc := trappingSubtract(acc, one);\n\
    \    end for;\n\
    \    return acc;\n\
     end;" ]

(* A bound of 0 is the sharpest case: exclusive semantics run it zero times,
   inclusive semantics exactly once. *)
let test_zero_bound_loop_is_inclusive _ =
  assert_inclusive ~entry:"countZero" loop_fixture

let test_bounded_loop_is_inclusive _ =
  assert_inclusive ~entry:"countTen" loop_fixture

(* The hand-built path: `Mt.MFor` directly, including a start other than 0. *)
let test_hand_built_for_lowers_to_cmplte _ =
  let open Stages.Mtast in
  let stmt =
    MFor (Identifier.make_ident "i",
          MIntConstant "3",
          MIntConstant "7",
          MAssignVar (Identifier.make_qident
                        (Identifier.make_mod_name "Loops",
                         Identifier.make_ident "acc",
                         Identifier.make_ident "acc"),
                      MTemporary (Identifier.make_ident "t", MonoInteger (Unsigned, Width32))))
  in
  match Compiler_cps.convert_stmt stmt with
  | CpsGen.Block (_, CpsGen.While (CpsGen.CmpLte _, _)) -> ()
  | CpsGen.Block (_, CpsGen.While (CpsGen.CmpLt _, _)) ->
     assert_failure "MFor lowered to CmpLt"
  | other ->
     assert_failure
       ("MFor did not lower to a While with a comparison: "
        ^ CpsGen.string_of_stmt other)

let suite =
  "CpsLoopLowering" >::: [
      "a bound of 0 executes once (inclusive)" >:: test_zero_bound_loop_is_inclusive;
      "a bounded loop keeps the inclusive endpoint" >:: test_bounded_loop_is_inclusive;
      "MFor lowers to CmpLte" >:: test_hand_built_for_lowers_to_cmplte;
    ]

let () = run_test_tt_main suite
