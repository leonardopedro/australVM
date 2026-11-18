(*
   GpuSubsetTest — G03/G04.

   Positive fixtures lower to typed IR. Negative fixtures cover the admitted
   closure's refusals: loops (including hidden in a helper), mutation, the
   trapping/divide arithmetic contracts, non-Nat32 entry signatures, recursion,
   and a user function that merely *looks* like a canonical intrinsic.
*)

open OUnit2

let read_file path =
  let ic = open_in_bin path in
  let n = in_channel_length ic in
  let s = really_input_string ic n in
  close_in ic;
  s

let prepare_module ~name aui_source aum_source =
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.TwoFileModuleSource
      { int_filename = name ^ ".aui"; int_code = aui_source;
        body_filename = name ^ ".aum"; body_code = aum_source }
  in
  let (_, p) = Compiler.prepare_module env source in
  (p.Compiler.prepared_env, p.Compiler.prepared_mono)

let pointwise () =
  prepare_module ~name:"Pointwise"
    (read_file "examples/gpu/pointwise/Pointwise.aui")
    (read_file "examples/gpu/pointwise/Pointwise.aum")

let kernel entry =
  let (env, mono) = pointwise () in
  Gpu_subset.build ~entry env mono

(* Inline negatives are body-only modules: they must typecheck normally and
   then be refused by the GPU subset, not by the frontend. *)
let inline ?(name = "M") functions =
  let env = Compiler.cenv Compiler.empty_compiler in
  let aum =
    Printf.sprintf "module body %s is\n" name
    ^ String.concat "" (List.map (fun (body) -> "    " ^ body ^ "\n") functions)
    ^ "end module body.\n"
  in
  let source =
    Compiler.BodyModuleSource { body_filename = name ^ ".aum"; body_code = aum }
  in
  (match Compiler.prepare_module env source with
   | (_, p) -> (p.Compiler.prepared_env, p.Compiler.prepared_mono)
   | exception Error.Austral_error e ->
      assert_failure ("inline fixture failed the frontend:\n"
                      ^ Error.render_error_to_plain e))

let expect_refusal what f =
  match f () with
  | exception Gpu_subset.Refused _ -> ()
  | exception e ->
     assert_failure
       (Printf.sprintf "%s: expected a GPU subset refusal, got %s"
          what (Printexc.to_string e))
  | _ -> assert_failure (what ^ ": source was accepted")

(* -------------------------------------------------------------------------- *)
(* Positives                                                                   *)
(* -------------------------------------------------------------------------- *)

let test_identity_and_arithmetic _ =
  let k = kernel "identity" in
  assert_equal 1 k.Gpu_ir.input_arity;
  (match k.Gpu_ir.body with
   | Gpu_ir.Return (Gpu_ir.Param 0) -> ()
   | _ -> assert_failure "identity did not lower to its parameter");
  (match kernel "difference" with
   | k ->
      match k.Gpu_ir.body with
      | Gpu_ir.Return (Gpu_ir.Intrinsic (Gpu_ir.Subtract,
                                        [Gpu_ir.Param 0; Gpu_ir.Param 1])) -> ()
      | _ -> assert_failure "subtraction operand order was not preserved")

let test_bitwise_mask _ =
  match (kernel "mask").Gpu_ir.body with
  | Gpu_ir.Return (Gpu_ir.Intrinsic (Gpu_ir.BitwiseAnd,
      [Gpu_ir.Intrinsic (Gpu_ir.BitwiseOr, [Gpu_ir.Param 0; Gpu_ir.Param 1]);
       Gpu_ir.Intrinsic (Gpu_ir.BitwiseNot, [Gpu_ir.Param 1])])) -> ()
  | _ -> assert_failure "bitwise mask did not lower to the admitted intrinsics"

let test_clamp_conditional _ =
  let body = (kernel "clamp").Gpu_ir.body in
  (* The frontend materialises the comparison in a temporary, so the IR is
     `let t = x > 100; if t then return 100 else return x`. Both the
     comparison and the branch structure must survive. *)
  match body with
  | Gpu_ir.Seq [Gpu_ir.Let (0, Gpu_ir.Compare (Gpu_ir.GreaterThan,
                                               Gpu_ir.Param 0, Gpu_ir.U32 "100"));
                Gpu_ir.If (Gpu_ir.Local 0,
                           Gpu_ir.Return (Gpu_ir.U32 "100"),
                           Gpu_ir.Return (Gpu_ir.Param 0))] -> ()
  | _ ->
     assert_failure
       ("clamp did not preserve its comparison and branches: "
        ^ Gpu_ir.canonical_bytes (kernel "clamp"))

let test_local_helper_closure _ =
  let p = inline ~name:"H"
      [ "function twice(x: Nat32): Nat32 is\n        return modularAdd(x, x);\n    end;";
        "function entry(x: Nat32): Nat32 is\n        return twice(x);\n    end;" ] in
  let k = Gpu_subset.build ~entry:"entry" (fst p) (snd p) in
  assert_equal 1 (List.length k.Gpu_ir.helpers);
  match k.Gpu_ir.body with
  | Gpu_ir.Return (Gpu_ir.Call (0, [Gpu_ir.Param 0])) -> ()
  | _ -> assert_failure "helper call did not lower to a resolved Call"

(* Intrinsic identity is the resolved method ID, not a textual name. A user
   function with another name is an ordinary helper: the IR contains a Call.
   (A bare `modularAdd` in user source still resolves, in the frontend, to the
   canonical Pervasive method — which is exactly the identity rule v1 relies
   on, so it is not a spoof.) *)
let test_local_helper_is_not_an_intrinsic _ =
  let p = inline ~name:"Local"
      [ "function double(x: Nat32): Nat32 is\n        return x;\n    end;";
        "function entry(x: Nat32, y: Nat32): Nat32 is\n        return double(x);\n    end;" ] in
  let k = Gpu_subset.build ~entry:"entry" (fst p) (snd p) in
  match k.Gpu_ir.body with
  | Gpu_ir.Return (Gpu_ir.Call (_, [_])) -> ()
  | Gpu_ir.Return (Gpu_ir.Intrinsic _) ->
     assert_failure "a local helper gained intrinsic status"
  | _ -> assert_failure "local helper did not lower as a helper call"

(* -------------------------------------------------------------------------- *)
(* Refusals                                                                    *)
(* -------------------------------------------------------------------------- *)

let test_refuses_loop _ =
  let p = inline ~name:"Loop"
      [ "function entry(x: Nat32): Nat32 is\n        var y: Nat32 := x;\n        while y > 0 do\n            y := modularSubtract(y, y);\n        end while;\n        return y;\n    end;" ] in
  expect_refusal "while loop"
    (fun () -> Gpu_subset.build ~entry:"entry" (fst p) (snd p))

let test_refuses_loop_hidden_in_helper _ =
  let p = inline ~name:"HiddenLoop"
      [ "function spin(x: Nat32): Nat32 is\n        var y: Nat32 := x;\n        while y > 0 do\n            y := modularSubtract(y, y);\n        end while;\n        return y;\n    end;";
        "function entry(x: Nat32): Nat32 is\n        return spin(x);\n    end;" ] in
  expect_refusal "loop hidden in helper"
    (fun () -> Gpu_subset.build ~entry:"entry" (fst p) (snd p))

let test_refuses_mutation _ =
  let p = inline ~name:"Mutation"
      [ "function entry(x: Nat32): Nat32 is\n        var y: Nat32 := x;\n        y := modularAdd(y, y);\n        return y;\n    end;" ] in
  expect_refusal "mutable assignment"
    (fun () -> Gpu_subset.build ~entry:"entry" (fst p) (snd p))

let test_refuses_trapping_arithmetic _ =
  let p = inline ~name:"Trapping"
      [ "function entry(x: Nat32): Nat32 is\n        return trappingAdd(x, x);\n    end;" ] in
  expect_refusal "trappingAdd"
    (fun () -> Gpu_subset.build ~entry:"entry" (fst p) (snd p))

let test_refuses_division _ =
  let p = inline ~name:"Division"
      [ "function entry(x: Nat32): Nat32 is\n        return modularDivide(x, x);\n    end;" ] in
  expect_refusal "modularDivide"
    (fun () -> Gpu_subset.build ~entry:"entry" (fst p) (snd p))

let test_refuses_non_nat32_entry _ =
  let p = inline ~name:"Wide"
      [ "function entry(x: Nat64): Nat64 is\n        return x;\n    end;" ] in
  expect_refusal "Nat64 entry"
    (fun () -> Gpu_subset.build ~entry:"entry" (fst p) (snd p))

let test_refuses_recursion _ =
  let p = inline ~name:"Recursive"
      [ "function entry(x: Nat32): Nat32 is\n        return entry(x);\n    end;" ] in
  expect_refusal "recursive helper"
    (fun () -> Gpu_subset.build ~entry:"entry" (fst p) (snd p))

let suite =
  "GpuSubsetTest" >::: [
    "identity and asymmetric subtraction" >:: test_identity_and_arithmetic;
    "bitwise mask lowers" >:: test_bitwise_mask;
    "clamp preserves comparison and branches" >:: test_clamp_conditional;
    "local helper closure" >:: test_local_helper_closure;
    "local helper is not an intrinsic" >:: test_local_helper_is_not_an_intrinsic;
    "refuses while" >:: test_refuses_loop;
    "refuses loop hidden in a helper" >:: test_refuses_loop_hidden_in_helper;
    "refuses mutation" >:: test_refuses_mutation;
    "refuses trapping arithmetic" >:: test_refuses_trapping_arithmetic;
    "refuses division" >:: test_refuses_division;
    "refuses non-Nat32 entry" >:: test_refuses_non_nat32_entry;
    "refuses recursion" >:: test_refuses_recursion;
  ]

let () = run_test_tt_main suite
