(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L11c acceptance, for the pilot range wrapper, as
   executable cases. Each case drives the real compiler at `--target-type=tc`,
   because every claim here is about the diagnostic a user sees:

     accept  a caller whose branch consumes both halves, with the callee's
             precondition proved from its own `Requires`
     refuse  an incorrect total length (the right half reuses `length(w)`)
     refuse  overlapping halves (the right half starts one byte late)
     refuse  a call whose precondition the caller cannot prove
     refuse  a negative extent literal at the boundary
     accept  a literal split, where the bound is decided by arithmetic alone

   The fixture is deliberately plain Austral: a `Free` record with the pilot's
   three slots, `length`/`offset` that really read those slots, and a `split`
   whose body the pilot verifies before it believes any of the facts it hands a
   caller.
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
  match Sys.getenv_opt "AUSTRAL_SPLIT_WORKDIR" with
  | Some p -> p
  | None -> Filename.concat (Filename.dirname Sys.executable_name) "split-scratch"

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

(* ── the pilot, and a caller that consumes both halves ───────────────────── *)

let pilot ~module_name ~split_off ~right_len ~use_req = Printf.sprintf {|
module body %s is
    record Window: Free is
        base: Int64;
        off: Int64;
        len: Int64;
    end;

    union Halves: Free is
        case Both is
            left: Window;
            right: Window;
        case NoHalf;
    end;

    function length(w: Window): Int64 is
        return w.len;
    end;

    function offset(w: Window): Int64 is
        return w.off;
    end;

    pragma Liquid_Requires(Contract => "0 <= k && k <= length(w)");
    function split(w: Window, k: Int64): Halves is
        let l: Window := Window(base => w.base, off => offset(w), len => k);
        let r: Window := Window(base => w.base, off => %s, len => %s);
        return Both(left => l, right => r);
    end;

    pragma Liquid_Requires(Contract => "offset(left) + length(left) <= offset(right)");
    function adjacent(left: Window, right: Window): Int64 is
        return length(left) + length(right);
    end;

    pragma Liquid_Requires(Contract => "%s");
    function use(w: Window, k: Int64): Int64 is
        case split(w, k) of
            when Both(left: Window, right: Window) do
                return adjacent(left, right);
            when NoHalf do
                return 0;
        end case;
        return 0;
    end;

    function main(): ExitCode is
        return ExitSuccess();
    end;
end module body.
|}
  module_name split_off right_len use_req

let ok_caller =
  pilot ~module_name:"SplitOk" ~split_off:"offset(w) + k"
    ~right_len:"length(w) - k" ~use_req:"0 <= k && k <= length(w)"

let ok_literal =
  {|
module body SplitLiteral is
    record Window: Free is
        base: Int64;
        off: Int64;
        len: Int64;
    end;

    union Halves: Free is
        case Both is
            left: Window;
            right: Window;
        case NoHalf;
    end;

    function length(w: Window): Int64 is
        return w.len;
    end;

    function offset(w: Window): Int64 is
        return w.off;
    end;

    pragma Liquid_Requires(Contract => "0 <= k && k <= length(w)");
    function split(w: Window, k: Int64): Halves is
        let l: Window := Window(base => w.base, off => offset(w), len => k);
        let r: Window := Window(base => w.base, off => offset(w) + k, len => length(w) - k);
        return Both(left => l, right => r);
    end;

    function main(): ExitCode is
        let w: Window := Window(base => 0, off => 10, len => 8);
        case split(w, 3) of
            when Both(left: Window, right: Window) do
                printLn(length(left));
                printLn(length(right));
            when NoHalf do
                printLn(0);
        end case;
        return ExitSuccess();
    end;
end module body.
|}

let bad_total_length =
  pilot ~module_name:"SplitBadTotal" ~split_off:"offset(w) + k"
    ~right_len:"length(w)" ~use_req:"0 <= k && k <= length(w)"

let overlap =
  pilot ~module_name:"SplitOverlap" ~split_off:"offset(w) + (k + 1)"
    ~right_len:"length(w) - k" ~use_req:"0 <= k && k <= length(w)"

let weak_caller =
  pilot ~module_name:"SplitWeak" ~split_off:"offset(w) + k"
    ~right_len:"length(w) - k" ~use_req:"0 <= k"

let negative_extent =
  {|
module body SplitNegative is
    record Window: Free is
        base: Int64;
        off: Int64;
        len: Int64;
    end;

    union Halves: Free is
        case Both is
            left: Window;
            right: Window;
        case NoHalf;
    end;

    function length(w: Window): Int64 is
        return w.len;
    end;

    function offset(w: Window): Int64 is
        return w.off;
    end;

    pragma Liquid_Requires(Contract => "0 <= k && k <= length(w)");
    function split(w: Window, k: Int64): Halves is
        let l: Window := Window(base => w.base, off => offset(w), len => k);
        let r: Window := Window(base => w.base, off => offset(w) + k, len => length(w) - k);
        return Both(left => l, right => r);
    end;

    function main(): ExitCode is
        let w: Window := Window(base => 0, off => 0, len => 8);
        case split(w, -1) of
            when Both(left: Window, right: Window) do
                printLn(length(left));
            when NoHalf do
                printLn(0);
        end case;
        return ExitSuccess();
    end;
end module body.
|}

let suite =
  "LiquidSplitPilot" >::: [
    "accepts a caller that consumes both halves"
      >:: (fun _ ->
            let rc, err = compile ~name:"ok_caller" ok_caller in
            assert_bool
              (Printf.sprintf
                 "the checked caller should compile, but the compiler exited \
                  %d:\n%s" rc err)
              (rc = 0));
    "discharges a literal extent by arithmetic"
      >:: (fun _ ->
            let rc, err = compile ~name:"ok_literal" ok_literal in
            assert_bool
              (Printf.sprintf
                 "the literal split should compile, but the compiler exited \
                  %d:\n%s" rc err)
              (rc = 0));
    expects_refusal "refuses an incorrect total length" bad_total_length
      ~needles:[ "L11c: `split` builds `r.len`"; "would not partition" ];
    expects_refusal "refuses overlapping halves" overlap
      ~needles:[ "L11c: `split` builds `r.off`"; "would not partition" ];
    expects_refusal "refuses a precondition the caller cannot prove" weak_caller
      ~needles:[ "L11c: `use` cannot discharge" ];
    expects_refusal "refuses a negative extent at the boundary" negative_extent
      ~needles:[ "L11c"; "which is not a natural index"; "typed refusal at the boundary" ];
  ]

let () = run_test_tt_main suite
