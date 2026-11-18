(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L11a: the index-closure admissibility audit (WF3).

   WF3 says every function symbol a refinement applies must be
   total-fragment-admissible. L3 only checked functions that carry a
   `Liquid_*` pragma themselves, so an *unannotated* helper reached from a
   contract was never looked at — and an ordinary Austral function may contain
   a `while`, an `@embed`, a foreign call, a recursive call, or a call into
   another module. These tests exercise exactly that hole, and each refusal is
   checked for the three things the plan asks a refusal to carry: the rule, the
   typed call path, and a source location.

   The tests drive the real compiler with `--target-type=tc`, like
   `LiquidVCTest`: the claim is about the diagnostic a user sees, so the test
   reads the compiler's own error output rather than a hand-built AST.

   One case is not a refusal at all: `length(xs)` is a §4.3 *measure*, not a
   function of the module, and WF3's audit must let it through. Pinning it
   guards the exemption against a future "reject every unknown symbol" change.
*)
open OUnit2

(* ── harness ─────────────────────────────────────────────────────────────── *)

let compiler () =
  match Sys.getenv_opt "AUSTRAL_BIN" with
  | Some p -> p
  | None ->
     let bin_dir =
       Filename.concat (Filename.dirname (Filename.dirname Sys.executable_name)) "bin"
     in
     Filename.concat bin_dir "austral.exe"

let workdir () =
  match Sys.getenv_opt "AUSTRAL_INDEX_WORKDIR" with
  | Some p -> p
  | None ->
     Filename.concat (Filename.dirname Sys.executable_name) "index-closure-scratch"

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

(** Compile [source] at the typed level. Returns the exit status and the
    compiler's stderr — the typed pass runs before codegen, so no bridge
    artifact is needed beyond the one the binary already links. *)
let compile ~(name : string) (source : string) : int * string =
  let dir = workdir () in
  (try Unix.mkdir dir 0o755 with _ -> ());
  let src = Filename.concat dir (name ^ ".aum") in
  let err = Filename.concat dir (name ^ ".err") in
  write_file src source;
  let cmd =
    Printf.sprintf "%s compile %s --target-type=tc 2>%s"
      (Filename.quote (compiler ()))
      (Filename.quote src) (Filename.quote err)
  in
  let rc = Sys.command cmd in
  let stderr = try read_file err with _ -> "" in
  (rc, stderr)

let contains ~(needle : string) (hay : string) : bool =
  let n = String.length needle and h = String.length hay in
  let rec at i =
    if i + n > h then false
    else if String.sub hay i n = needle then true
    else at (i + 1)
  in
  n = 0 || at 0

let expects_ok name source =
  name >:: fun _ ->
  let rc, err = compile ~name source in
  assert_bool
    (Printf.sprintf "%s should compile, but the compiler exited %d:\n%s" name rc
       err)
    (rc = 0)

let expects_refusal name source ~needles =
  name >:: fun _ ->
  let rc, err = compile ~name source in
  assert_bool
    (Printf.sprintf "%s should be refused, but the compiler exited 0:\n%s" name
       err)
    (rc <> 0);
  List.iter
    (fun needle ->
      assert_bool
        (Printf.sprintf "%s: stderr does not contain %S:\n%s" name needle err)
        (contains ~needle err))
    needles

(* ── accepted: a pure acyclic helper used as an index function ───────────── *)

let ok_pure_helper =
  {|
module body LiquidIndexOk is
    function width(n: Int64): Int64 is
        let m: Int64 := n + 1;
        return m;
    end;

    function outer(n: Int64): Int64 is
        return width(n);
    end;

    pragma Liquid_Requires(Contract => "outer(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;

    function main(): ExitCode is
        printLn(use(3));
        return ExitSuccess();
    end;
end module body.
|}

(* A §4.3 measure is not a function of the module; the audit must not mistake
   it for an unknown callee. *)
let ok_measure =
  {|
module body LiquidIndexMeasure is
    pragma Liquid_Requires(Contract => "length(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;

    function main(): ExitCode is
        return ExitSuccess();
    end;
end module body.
|}

(* ── refused: a forbidden transitive callee ──────────────────────────────── *)

let bad_transitive_loop =
  {|
module body LiquidIndexLoop is
    function spin(n: Int64): Int64 is
        var i: Int64 := 0;
        while i < n do
            i := i + 1;
        end while;
        return i;
    end;

    function outer(n: Int64): Int64 is
        return spin(n);
    end;

    pragma Liquid_Requires(Contract => "outer(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;

    function main(): ExitCode is
        return ExitSuccess();
    end;
end module body.
|}

(* The offending construct is the *argument* of the outer call, so the walk has
   to descend into argument positions before it reports the call. *)
let bad_nested_embed =
  {|
pragma Unsafe_Module;

module body LiquidIndexEmbed is
    function tag(m: Int64): Int64 is
        return m;
    end;

    function peek(n: Int64): Int64 is
        return @embed(Int64, "$1", n);
    end;

    function outer(n: Int64): Int64 is
        return tag(peek(n));
    end;

    pragma Liquid_Ensures(Contract => "outer(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;

    function main(): ExitCode is
        return ExitSuccess();
    end;
end module body.
|}

let bad_cycle =
  {|
module body LiquidIndexCycle is
    function down(n: Int64): Int64 is
        if n <= 0 then
            return 0;
        else
            return down(n - 1);
        end if;
    end;

    pragma Liquid_Requires(Contract => "down(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;

    function main(): ExitCode is
        return ExitSuccess();
    end;
end module body.
|}

let bad_unknown =
  {|
module body LiquidIndexUnknown is
    pragma Liquid_Requires(Contract => "nosuch(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;
end module body.
|}

(* `trappingDivide` traps on a zero divisor, so it is deliberately outside the
   primitive allowlist. *)
let bad_primitive =
  {|
module body LiquidIndexDiv is
    function half(n: Int64): Int64 is
        return n / 2;
    end;

    pragma Liquid_Requires(Contract => "half(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;
end module body.
|}

(* A call into a built-in module: `printLn`'s declaration is not in this module,
   and auditing it from here is not possible, so it is refused rather than
   assumed pure. *)
let bad_import =
  {|
module body LiquidIndexImport is
    function say(n: Int64): Int64 is
        printLn(n);
        return n;
    end;

    pragma Liquid_Requires(Contract => "say(n) >= 0");
    function use(n: Int64): Int64 is
        return n;
    end;

    function main(): ExitCode is
        return ExitSuccess();
    end;
end module body.
|}

(* ── the suite ───────────────────────────────────────────────────────────── *)

let suite =
  "liquid-index-closure"
  >::: [
         expects_ok "ok-pure-helper" ok_pure_helper;
         expects_ok "ok-measure" ok_measure;
         expects_refusal "transitive-loop" bad_transitive_loop
           ~needles:
             [
               "WF3 (§2.3)";
               "`spin` uses `while`";
               "typed call path: outer -> spin";
               "contract: on `use` (Liquid_Requires";
               ":4:8";
             ];
         expects_refusal "nested-embed" bad_nested_embed
           ~needles:
             [
               "WF3 (§2.3)";
               "uses `@embed";
               "typed call path: outer -> peek";
               ":9:8";
             ];
         expects_refusal "self-cycle" bad_cycle
           ~needles:
             [
               "WF3 (§2.3)";
               "index call cycle: down -> down";
               "typed call path: down";
             ];
         expects_refusal "unknown-callee" bad_unknown
           ~needles:
             [
               "WF3 (§2.3)";
               "no function of that name is declared in this module";
               "characters 0..9";
             ];
         expects_refusal "non-primitive" bad_primitive
           ~needles:
             [
               "WF3 (§2.3)";
               "calls `trappingDivide`";
               "primitive allowlist";
               "typed call path: half";
               ":3:8";
             ];
         expects_refusal "imported-callee" bad_import
           ~needles:
             [
               "WF3 (§2.3)";
               "calls `printLn` from module";
               "may only call functions of this module";
             ];
       ]

let () = run_test_tt_main suite
