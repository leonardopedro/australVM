(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   L11e acceptance, driven through the real compiler at `--target-type=tc`.
   Every case is about the diagnostic or evidence a user sees: the opt-in
   fragment admits a proper-subterm descent, and refuses the same definition
   with a self-call on the original value, on a rebuilt value, with an effect,
   or with a mutual cycle. The erasure case compiles the same program with and
   without the marker and compares the emitted CPS bytes.
*)

open OUnit2

let compiler () =
  match Sys.getenv_opt "AUSTRAL_BIN" with
  | Some p -> p
  | None ->
     let bin_dir =
       Filename.concat (Filename.dirname (Filename.dirname Sys.executable_name)) "bin"
     in
     Filename.concat bin_dir "austral.exe"

let workdir () =
  match Sys.getenv_opt "AUSTRAL_DESCENT_WORKDIR" with
  | Some p -> p
  | None -> Filename.concat (Filename.dirname Sys.executable_name) "descent-scratch"

let write_file path contents =
  let oc = open_out_bin path in
  output_string oc contents;
  close_out oc

let read_file path =
  let ic = open_in_bin path in
  let n = in_channel_length ic in
  let s = really_input_string ic n in
  close_in ic;
  s

let contains ~(needle : string) (hay : string) : bool =
  let n = String.length needle and h = String.length hay in
  let rec at i =
    if i + n > h then false
    else if String.sub hay i n = needle then true
    else at (i + 1)
  in
  n = 0 || at 0

let compile ~(name : string) ?(evidence : bool = false) (source : string) :
    int * string * string =
  let dir = workdir () in
  (try Unix.mkdir dir 0o755 with _ -> ());
  let src = Filename.concat dir (name ^ ".aum") in
  let err = Filename.concat dir (name ^ ".err") in
  let ev = Filename.concat dir (name ^ ".evidence") in
  (try Sys.remove ev with _ -> ());
  write_file src source;
  let ev_prefix =
    if evidence then
      Printf.sprintf "AUSTRAL_LIQUID_DESCENT_EVIDENCE=%s " (Filename.quote ev)
    else ""
  in
  let cmd =
    Printf.sprintf "%s%s compile %s --target-type=tc 2>%s"
      ev_prefix (Filename.quote (compiler ())) (Filename.quote src)
      (Filename.quote err)
  in
  let rc = Sys.command cmd in
  let stderr = try read_file err with _ -> "" in
  let evidence = try read_file ev with _ -> "" in
  (rc, stderr, evidence)

(* The single mutable piece: the body of the `Cons` branch. *)
let source ~marker ~cons_body ~extra =
  Printf.sprintf {|module body Descent is
    union List: Free is
        case Nil;
        case Cons is
            head: Int64;
            tail: List;
    end;

    pragma Liquid_Fold;
    function foldLength(init: Int64, xs: List): Int64 is
        return init;
    end;

    %s
    %s
    function length(xs: List): Int64 is
        case xs of
            when Nil do
                return 0;
            when Cons(head: Int64, tail: List) do
                %s
        end case;
    end;
|}
    extra marker cons_body
  ^ {|    function main(): ExitCode is
        let xs: List := Cons(head => 1, tail => Cons(head => 2, tail => Nil()));
        printLn(length(xs));
        return ExitSuccess();
    end;
end module body.
|}

let descent =
  "pragma Liquid_Descent;"

let body_good =
  "let one: Int64 := 1;\n                return one + length(tail);"

let body_original = "return length(xs);"

let body_rebuilt =
  "return length(Cons(head => head, tail => Nil()));"

let body_effect =
  "var one: Int64 := 1;\n                one := one;\n                return length(tail);"

let mutual_extra =
  "pragma Liquid_Descent;\n    function other(xs: List): Int64 is\n        return length(xs);\n    end;"

let expects_refusal name source ~needles =
  name >:: fun _ ->
  let rc, err, _ = compile ~name source in
  assert_bool
    (Printf.sprintf "%s should be refused, but exited 0:\n%s" name err)
    (rc <> 0);
  List.iter
    (fun needle ->
      assert_bool
        (Printf.sprintf "%s: stderr does not contain %S:\n%s" name needle err)
        (contains ~needle err))
    needles

let compile_to_cps ~work name source =
  let src = Filename.concat work (name ^ ".aum") in
  let cps = Filename.concat work (name ^ ".cpsbin") in
  let c = Filename.concat work (name ^ ".c") in
  write_file src source;
  let cmd =
    Printf.sprintf
      "CPS_JIT_ADVISORY=1 %s compile %s --entrypoint=Descent:main \
       --use-cps-jit --emit-cps=%s --output=%s >%s 2>%s"
      (Filename.quote (compiler ())) (Filename.quote src) (Filename.quote cps)
      (Filename.quote c)
      (Filename.quote (Filename.concat work (name ^ ".out")))
      (Filename.quote (Filename.concat work (name ^ ".err")))
  in
  let rc = Sys.command cmd in
  if rc <> 0 then begin
    let err =
      try read_file (Filename.concat work (name ^ ".err"))
      with _ -> "(no error output)"
    in
    assert_failure (Printf.sprintf "compiling %s failed (exit %d):\n%s" name rc err)
  end;
  read_file cps

let suite =
  "LiquidDescent" >::: [
    "admits a proper-subterm descent"
      >:: (fun _ ->
            let rc, err, ev =
              compile ~name:"ok_descent" ~evidence:true
                (source ~marker:descent ~cons_body:body_good ~extra:"")
            in
            assert_bool
              (Printf.sprintf "the descent should compile, but exited %d:\n%s" rc
                 err)
              (rc = 0);
            List.iter
              (fun needle ->
                assert_bool
                  (Printf.sprintf "evidence does not contain %S:\n%s" needle ev)
                  (contains ~needle ev))
              [ "descent: length arg=tail self-calls=1 case-depth=1 verdict=held";
                "evaluator-fuel: 256 index steps";
                "fold: foldLength";
                "correspondence: stated" ]);
    expects_refusal "refuses a self-call on the original argument"
      (source ~marker:descent ~cons_body:body_original ~extra:"")
      ~needles:[ "L11e: `length`"; "original argument" ];
    expects_refusal "refuses a self-call on a rebuilt value"
      (source ~marker:descent ~cons_body:body_rebuilt ~extra:"")
      ~needles:[ "L11e: `length`"; "rebuilt value" ];
    expects_refusal "refuses an effectful descent"
      (source ~marker:descent ~cons_body:body_effect ~extra:"")
      ~needles:[ "L11e: `length`"; "uses an effect" ];
    expects_refusal "refuses mutual recursion even when one function is marked"
      (source ~marker:descent
         ~cons_body:"return other(tail);" ~extra:mutual_extra)
      ~needles:[ "T1 (§4.1)"; "length -> other -> length" ];
    "the marker erases: marked and unmarked programs have identical CPS bytes"
      >:: (fun _ ->
            let work = workdir () in
            (try Unix.mkdir work 0o755 with _ -> ());
            (* The recursive union fixture is checked at `--target-type=tc`:
               its by-value tail has no C representation. Erasure is measured
               on a small marked-but-non-recursive module, where the marker is
               the only difference and codegen succeeds. *)
            let erasure_source ~marker =
              Printf.sprintf {|module body Descent is
    %s
    function id(x: Int64): Int64 is
        return x;
    end;

    function main(): ExitCode is
        printLn(id(3));
        return ExitSuccess();
    end;
end module body.
|} marker
            in
            let marked =
              compile_to_cps ~work "descent_marked" (erasure_source ~marker:descent)
            in
            let plain =
              compile_to_cps ~work "descent_plain" (erasure_source ~marker:"")
            in
            assert_bool
              (Printf.sprintf
                 "the marker reached codegen: %d vs %d CPS bytes"
                 (String.length marked) (String.length plain))
              (marked = plain))
  ]

let () = run_test_tt_main suite
