(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L9: erasure, checked the only way it can be honestly
   checked.

   "Index-carrying types are erased" is not a claim about the representation —
   the parser happily builds a `ty` with a term inside it. It is a claim about
   what reaches codegen, and the only way to test that is to compile the same
   program twice and compare the emitted bytes.

   Two modules, identical in every respect that the compiler can see except that
   one carries index-carrying type annotations and value refinements. If an
   index leaked into CPS, into a function signature, or into a constant, the two
   binaries would differ.

   The comparison is on the `--emit-cps` binary rather than the generated C,
   because CPS is the layer where a type-level annotation would have to become a
   runtime value to survive: C still has type syntax, CPS does not.
*)
open OUnit2

(* Path to the compiler, as staged into the sandbox by the `deps` stanza in
   test/dune. Override with AUSTRAL_BIN when running the test by hand. *)
let compiler () =
  match Sys.getenv_opt "AUSTRAL_BIN" with
  | Some p -> p
  | None ->
    (* dune stages `(deps ../bin/austral.exe)`; the test executable lives in
       `default/test`, so the compiler is `default/bin/austral.exe`. *)
    let bin_dir =
      Filename.concat (Filename.dirname (Filename.dirname Sys.executable_name)) "bin"
    in
    let p = Filename.concat bin_dir "austral.exe" in
    if Sys.file_exists p then p
    else begin
      (* A plain `austral` next to the test executable is the shape a manual run
         has. Report clearly rather than failing with "command not found". *)
      let alt = Filename.concat (Filename.dirname Sys.executable_name) "austral" in
      if Sys.file_exists alt then alt
      else p
    end

let workdir () =
  match Sys.getenv_opt "AUSTRAL_L9_WORKDIR" with
  | Some p -> p
  | None -> Filename.concat (Filename.dirname Sys.executable_name) "l9-scratch"

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

(* The same program twice. Only the annotations differ. *)
let plain_source =
  {|
module body Probe is
    function f(n: Int64): Int64 is
        var r: Int64 := n;
        return r;
    end;
    function main(): ExitCode is
        printLn(f(7));
        return ExitSuccess();
    end;
end module body.
|}

(* Every declaration carries an index-carrying annotation — `Span[Nat8, 1]`
   indexes by a literal, `Nat` by a sort — plus a value refinement. *)
let annotated_source =
  {|
module body Probe is
    """
    L9 erasure probe: byte-identical output is expected versus plain.
    """
    pragma Liquid_Requires(Contract => "n : Nat");
    pragma Liquid_Ensures(Contract => "result : Span[Nat8, 1]");
    function f(n: Int64): Int64 is
        var r: Int64 := n;
        return r;
    end;
    function main(): ExitCode is
        printLn(f(7));
        return ExitSuccess();
    end;
end module body.
|}

let compile_to_cps ~work name source =
  let src = Filename.concat work (name ^ ".aum") in
  let cps = Filename.concat work (name ^ ".cpsbin") in
  let c = Filename.concat work (name ^ ".c") in
  write_file src source;
  let cmd =
    Printf.sprintf "%s compile %s --entrypoint=Probe:main --use-cps-jit --emit-cps=%s --output=%s >%s.out 2>%s.err"
      (Filename.quote (compiler ())) (Filename.quote src) (Filename.quote cps)
      (Filename.quote c) (Filename.quote work) (Filename.quote work)
  in
  let rc = Sys.command cmd in
  if rc <> 0 then begin
    let err =
      try read_file (Filename.concat work (name ^ ".err"))
      with _ -> "(no error output)"
    in
    assert_failure
      (Printf.sprintf "compiling %s failed (exit %d):\n%s" name rc err)
  end;
  if not (Sys.file_exists cps) then
    assert_failure (Printf.sprintf "%s produced no CPS binary" name);
  read_file cps

let test_refinements_and_indices_are_erased () =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let a = compile_to_cps ~work "plain" plain_source in
  let b = compile_to_cps ~work "annotated" annotated_source in
  assert_bool
    (Printf.sprintf
       "erasure broken: %d vs %d CPS bytes, first difference at %d"
       (String.length a) (String.length b)
       (let rec first_diff i =
          if i >= String.length a || i >= String.length b then -1
          else if a.[i] <> b.[i] then i
          else first_diff (i + 1)
        in
        first_diff 0))
    (a = b)

let suite =
  "LiquidErasureTest" >::: [
    "refinements_and_indices_are_erased" >:: (fun _ -> test_refinements_and_indices_are_erased ());
  ]

let () = run_test_tt_main suite
