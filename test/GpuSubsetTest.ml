(*
   GpuSubsetTest — G03/G04.

   Positive fixtures lower to typed IR. Negative fixtures cover the admitted
   closure's refusals: loops (including hidden in a helper), mutation, the
   trapping/divide arithmetic contracts, non-Nat32 entry signatures, recursion,
   and a user function that merely *looks* like a canonical intrinsic.
   G17a/G17b add the `bounded-fold-v1` clause: the probe over the typed AST,
   the unrolled KernelIR (one immutable `let` per body execution, values
   checked with a tiny interpreter), and its refusals.
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
    (read_file (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aui"))
    (read_file (TestUtil.fixture "examples/gpu/pointwise/Pointwise.aum"))

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


(* -------------------------------------------------------------------------- *)
(* G17a: bounded-fold source admission                                         *)
(* -------------------------------------------------------------------------- *)

let fold_module ~name =
  prepare_module ~name
    (read_file (TestUtil.fixture "examples/gpu/bounded-fold/Fold.aui"))
    (read_file (TestUtil.fixture "examples/gpu/bounded-fold/Fold.aum"))

let fold ~entry =
  let (env, mono) = fold_module ~name:"Fold" in
  Gpu_subset.recognize_bounded_fold ~entry env mono

let fold_kernel ?(entry = "entry") functions =
  let p = inline ~name:"Fold" functions in
  Gpu_subset.build ~source_profile:Gpu_subset.BoundedFoldV1 ~entry (fst p) (snd p)

let fixture_kernel ~entry =
  let (env, mono) = fold_module ~name:"Fold" in
  Gpu_subset.build ~source_profile:Gpu_subset.BoundedFoldV1 ~entry env mono

(* A tiny KernelIR interpreter, enough to compare the unrolled body with the C
   oracle and the hand count. Every u32 operation is modular by construction
   (Int32 arithmetic wraps). *)
let rec eval_expr (locals: (int, int32) Hashtbl.t) (params: int32 array)
    (e: Gpu_ir.expr): int32 =
  match e with
  | Gpu_ir.Param i -> params.(i)
  | Gpu_ir.Local id ->
     (match Hashtbl.find_opt locals id with
      | Some v -> v
      | None -> assert_failure (Printf.sprintf "read of unbound local %d" id))
  | Gpu_ir.U32 s -> Int64.to_int32 (Int64.of_string (Gpu_ir.u32_of_string s))
  | Gpu_ir.F32 _ | Gpu_ir.Bool _ -> assert_failure "non-u32 value in the v1 fold"
  | Gpu_ir.Intrinsic (Gpu_ir.Add, [a; b]) ->
     Int32.add (eval_expr locals params a) (eval_expr locals params b)
  | Gpu_ir.Intrinsic (Gpu_ir.Subtract, [a; b]) ->
     Int32.sub (eval_expr locals params a) (eval_expr locals params b)
  | Gpu_ir.Intrinsic (Gpu_ir.Multiply, [a; b]) ->
     Int32.mul (eval_expr locals params a) (eval_expr locals params b)
  | Gpu_ir.Intrinsic (Gpu_ir.BitwiseAnd, [a; b]) ->
     Int32.logand (eval_expr locals params a) (eval_expr locals params b)
  | Gpu_ir.Intrinsic (Gpu_ir.BitwiseOr, [a; b]) ->
     Int32.logor (eval_expr locals params a) (eval_expr locals params b)
  | Gpu_ir.Intrinsic (Gpu_ir.BitwiseXor, [a; b]) ->
     Int32.logxor (eval_expr locals params a) (eval_expr locals params b)
  | Gpu_ir.Intrinsic (op, _) ->
     assert_failure ("interpreted intrinsic " ^ Gpu_ir.show_intrinsic op)
  | Gpu_ir.Compare _ | Gpu_ir.BooleanOp _ | Gpu_ir.Not _
  | Gpu_ir.Conditional _ | Gpu_ir.Call _ ->
     assert_failure "unexpected expression in the interpreted v1 fold"

let rec eval_stmt (locals: (int, int32) Hashtbl.t) (params: int32 array)
    (s: Gpu_ir.stmt): int32 option =
  match s with
  | Gpu_ir.Let (id, e) ->
     Hashtbl.replace locals id (eval_expr locals params e);
     None
  | Gpu_ir.Seq items ->
     List.fold_left
       (fun acc s -> match acc with Some _ -> acc | None -> eval_stmt locals params s)
       None items
  | Gpu_ir.If (c, t, f) ->
     if eval_expr locals params c <> 0l then eval_stmt locals params t
     else eval_stmt locals params f
  | Gpu_ir.Return e -> Some (eval_expr locals params e)

let fold_value (k: Gpu_ir.kernel) (x: int32): int32 =
  match eval_stmt (Hashtbl.create 64) [| x |] k.Gpu_ir.body with
  | Some v -> v
  | None -> assert_failure "the fold kernel does not return"

let rec count_lets (s: Gpu_ir.stmt): int =
  match s with
  | Gpu_ir.Let _ -> 1
  | Gpu_ir.Seq items -> List.fold_left (fun n s -> n + count_lets s) 0 items
  | Gpu_ir.If (_, t, f) -> count_lets t + count_lets f
  | Gpu_ir.Return _ -> 0

let fold_source ?(name = "Fold") functions =
  inline ~name functions

(* The probe records the resolved typed-AST decisions the emitter would
   consume: the accumulator, its literal bound, the update, the result. *)
let test_bounded_fold_probe _ =
  let f0 = fold ~entry:"upToZero" in
  assert_equal "acc" f0.Gpu_subset.fold_binder;
  assert_equal 0 f0.Gpu_subset.fold_literal_bound;
  assert_equal 1 f0.Gpu_subset.fold_iterations;
  assert_equal 1 f0.Gpu_subset.fold_params;
  (match f0.Gpu_subset.fold_init with
   | Gpu_ir.U32 "0" -> ()
   | _ -> assert_failure "upToZero's initializer is not the literal 0");
  (match f0.Gpu_subset.fold_update with
   | Gpu_ir.Intrinsic (Gpu_ir.Add,
                       [Gpu_ir.Local id; Gpu_ir.Param 0]) ->
      assert_equal f0.Gpu_subset.fold_binder_local id
   | _ ->
      assert_failure
        "upToZero's update is not modularAdd(accumulator, parameter)");
  (match f0.Gpu_subset.fold_result with
   | Gpu_ir.Local id -> assert_equal f0.Gpu_subset.fold_binder_local id
   | _ -> assert_failure "upToZero's result is not the accumulator");
  let f31 = fold ~entry:"upTo31" in
  assert_equal 31 f31.Gpu_subset.fold_literal_bound;
  assert_equal 32 f31.Gpu_subset.fold_iterations;
  Printf.printf
    "[probe] G17a upToZero: binder=%s init=%s count=%d update=Intrinsic(Add,[Local %d;Param 0]) result=Local %d\n%!"
    f0.Gpu_subset.fold_binder "0" f0.Gpu_subset.fold_iterations
    f0.Gpu_subset.fold_binder_local f0.Gpu_subset.fold_binder_local

let test_source_profile_names _ =
  assert_equal (Gpu_subset.BoundedFoldV1)
    (Gpu_subset.source_profile_of_string "bounded-fold-v1");
  assert_equal "bounded-fold-v1"
    (Gpu_subset.source_profile_name Gpu_subset.BoundedFoldV1);
  expect_refusal "unknown source profile"
    (fun () -> ignore (Gpu_subset.source_profile_of_string "bounded-fold-v2"))

let test_fold_refuses_bound_32 _ =
  let p = fold_source
      [ "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 32 do\n            acc := modularAdd(acc, x);\n        end for;\n        return acc;\n    end;" ] in
  expect_refusal "fold bound 32"
    (fun () -> Gpu_subset.recognize_bounded_fold ~entry:"entry" (fst p) (snd p))

let test_fold_refuses_dynamic_bound _ =
  (* A loop bound must be an Index expression. The only Index values a
     one-parameter Nat32 entry can name are literals, so `to x` is refused by
     the frontend while building the bound; the fold walker separately refuses
     an Index temporary with no literal initializer. *)
  let env = Compiler.cenv Compiler.empty_compiler in
  let aum =
    "module body Dyn is\n"
    ^ "    function entry(x: Nat32): Nat32 is\n"
    ^ "        var acc: Nat32 := 0;\n"
    ^ "        for i from 0 to x do\n"
    ^ "            acc := modularAdd(acc, x);\n"
    ^ "        end for;\n"
    ^ "        return acc;\n"
    ^ "    end;\n"
    ^ "end module body.\n"
  in
  let source =
    Compiler.BodyModuleSource
      { body_filename = "Dyn.aum"; body_code = aum }
  in
  (match Compiler.prepare_module env source with
   | exception Error.Austral_error e ->
      let message = Error.render_error_to_plain e in
      assert_bool "a dynamic bound must be refused while typing Index"
        (let rec contains i =
           i + 5 <= String.length message
           && (String.sub message i 5 = "Index" || contains (i + 1))
         in
         contains 0)
   | _ ->
      assert_failure
        "a Nat32 loop bound reached the GPU subset; the Index typing should \
         have refused it")

let test_fold_refuses_second_mutable_binding _ =
  let p = fold_source
      [ "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        var other: Nat32 := x;\n        for i from 0 to 3 do\n            acc := modularAdd(acc, x);\n        end for;\n        return modularAdd(acc, other);\n    end;" ] in
  expect_refusal "second mutable binding"
    (fun () -> Gpu_subset.recognize_bounded_fold ~entry:"entry" (fst p) (snd p))

let test_fold_refuses_second_write _ =
  (* A write other than the accumulator's: an indexed store is refused as an
     address write by the fold walker rather than being mistaken for the one
     admitted assignment. *)
  let p = fold_source
      [ "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 3 do\n            acc := modularAdd(acc, x);\n            acc := modularAdd(acc, x);\n        end for;\n        return acc;\n    end;" ] in
  expect_refusal "second write in the fold body"
    (fun () -> Gpu_subset.recognize_bounded_fold ~entry:"entry" (fst p) (snd p))

let test_fold_refuses_loop_in_helper _ =
  let p = fold_source
      [ "function spin(x: Nat32): Nat32 is\n        var y: Nat32 := x;\n        while y > 0 do\n            y := modularSubtract(y, y);\n        end while;\n        return y;\n    end;";
        "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 3 do\n            acc := modularAdd(acc, spin(x));\n        end for;\n        return acc;\n    end;" ] in
  expect_refusal "loop hidden in the fold's helper"
    (fun () -> Gpu_subset.recognize_bounded_fold ~entry:"entry" (fst p) (snd p))

let test_fold_refuses_conditional_return _ =
  let p = fold_source
      [ "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 1 do\n            acc := modularAdd(acc, x);\n        end for;\n        if x > 0 then\n            return x;\n        end if;\n        return acc;\n    end;" ] in
  expect_refusal "conditional/early return in the entry body"
    (fun () -> Gpu_subset.recognize_bounded_fold ~entry:"entry" (fst p) (snd p))

let test_fold_refuses_non_accumulator_return _ =
  let p = fold_source
      [ "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 3 do\n            acc := modularAdd(acc, x);\n        end for;\n        return modularAdd(acc, x);\n    end;" ] in
  expect_refusal "fold returning something other than the accumulator"
    (fun () -> Gpu_subset.recognize_bounded_fold ~entry:"entry" (fst p) (snd p))

(* G17b: the loop is gone from the body; it is one KernelIR `Let` per body
   execution, threaded through immutable locals, plus the initializer. *)
let test_bounded_fold_expands_to_immutable_locals _ =
  let k0 = fixture_kernel ~entry:"upToZero" in
  (* The initializer's `let` plus one `let` per body execution. *)
  assert_equal 2 (count_lets k0.Gpu_ir.body);
  assert_equal 7l (fold_value k0 7l);
  let k31 = fixture_kernel ~entry:"upTo31" in
  assert_equal 33 (count_lets k31.Gpu_ir.body);
  assert_equal 224l (fold_value k31 7l);
  (* Several iterations, including an operand order that would show a reversed
     update. *)
  let p = fold_source
      [ "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 4 do\n            acc := modularAdd(acc, x);\n        end for;\n        return acc;\n    end;" ] in
  let k4 =
    Gpu_subset.build ~source_profile:Gpu_subset.BoundedFoldV1
      ~entry:"entry" (fst p) (snd p)
  in
  assert_equal 6 (count_lets k4.Gpu_ir.body);
  assert_equal 15l (fold_value k4 3l);
  (* Every local is bound once and read from the previous one: no two lets
     share an id. *)
  let ids = Hashtbl.create 8 in
  let rec check s =
    match s with
    | Gpu_ir.Let (id, _) ->
       assert_bool "an unrolled local is bound twice"
         (not (Hashtbl.mem ids id));
       Hashtbl.replace ids id ()
    | Gpu_ir.Seq items -> List.iter check items
    | Gpu_ir.If (_, t, f) -> check t; check f
    | Gpu_ir.Return _ -> ()
  in
  check k31.Gpu_ir.body;
  assert_equal 33 (Hashtbl.length ids)

let test_bounded_fold_is_refused_by_strict_build _ =
  let (env, mono) = fold_module ~name:"Fold" in
  expect_refusal "strict build still refuses the fixture's for loop"
    (fun () -> Gpu_subset.build ~entry:"upToZero" env mono)

let test_bounded_fold_refuses_second_loop _ =
  let p = fold_source
      [ "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 2 do\n            acc := modularAdd(acc, x);\n        end for;\n        for i from 0 to 2 do\n            acc := modularAdd(acc, x);\n        end for;\n        return acc;\n    end;" ] in
  expect_refusal "a second loop"
    (fun () ->
      Gpu_subset.build ~source_profile:Gpu_subset.BoundedFoldV1
        ~entry:"entry" (fst p) (snd p))

let test_bounded_fold_refuses_loop_in_helper_at_build _ =
  let p = fold_source
      [ "function spin(x: Nat32): Nat32 is\n        var y: Nat32 := x;\n        for i from 0 to 2 do\n            y := modularAdd(y, x);\n        end for;\n        return y;\n    end;";
        "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 2 do\n            acc := modularAdd(acc, spin(x));\n        end for;\n        return acc;\n    end;" ] in
  expect_refusal "a loop in a helper under bounded-fold-v1"
    (fun () ->
      Gpu_subset.build ~source_profile:Gpu_subset.BoundedFoldV1
        ~entry:"entry" (fst p) (snd p))

(* The projected expansion is checked before it is built: a helper whose body
   is large, called in a 32-iteration fold, projects past the 100000-node
   budget and is refused. *)
let test_bounded_fold_refuses_projected_budget _ =
  let lets =
    List.init 2000 (fun i ->
        Printf.sprintf "        let a%d: Nat32 := modularAdd(x, x);\n" i)
  in
  let heavy_body =
    "function heavy(x: Nat32): Nat32 is\n"
    ^ String.concat "" lets
    ^ Printf.sprintf "        return a%d;\n" 1999
    ^ "    end;"
  in
  let p = fold_source
      [ heavy_body;
        "function entry(x: Nat32): Nat32 is\n        var acc: Nat32 := 0;\n        for i from 0 to 31 do\n            acc := modularAdd(acc, heavy(x));\n        end for;\n        return acc;\n    end;" ] in
  expect_refusal "a fold whose projected expansion exceeds the node budget"
    (fun () ->
      Gpu_subset.build ~source_profile:Gpu_subset.BoundedFoldV1
        ~entry:"entry" (fst p) (snd p))

let test_default_profile_still_refuses_for _ =
  let (env, mono) = fold_module ~name:"Fold" in
  expect_refusal "for loop under the default profile"
    (fun () -> Gpu_subset.build ~entry:"upToZero" env mono);
  expect_refusal "strict source profile"
    (fun () ->
      Gpu_subset.recognize_bounded_fold
        ~source_profile:Gpu_subset.StrictV1 ~entry:"upToZero" env mono)

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
    "bounded fold probe records the resolved shape" >:: test_bounded_fold_probe;
    "source profile names" >:: test_source_profile_names;
    "refuses a fold bound of 32" >:: test_fold_refuses_bound_32;
    "refuses a dynamic fold bound" >:: test_fold_refuses_dynamic_bound;
    "refuses a second mutable binding" >:: test_fold_refuses_second_mutable_binding;
    "refuses a second write in the fold body" >:: test_fold_refuses_second_write;
    "refuses a loop in the fold's helper" >:: test_fold_refuses_loop_in_helper;
    "refuses a conditional early return" >:: test_fold_refuses_conditional_return;
    "refuses a non-accumulator return" >:: test_fold_refuses_non_accumulator_return;
    "bounded fold expands to immutable locals" >:: test_bounded_fold_expands_to_immutable_locals;
    "strict build still refuses the fold" >:: test_bounded_fold_is_refused_by_strict_build;
    "bounded fold refuses a second loop" >:: test_bounded_fold_refuses_second_loop;
    "bounded fold refuses a loop in a helper at build" >:: test_bounded_fold_refuses_loop_in_helper_at_build;
    "bounded fold refuses a projected over-budget expansion" >:: test_bounded_fold_refuses_projected_budget;
    "default profile still refuses for" >:: test_default_profile_still_refuses_for;
  ]

let () = run_test_tt_main suite
