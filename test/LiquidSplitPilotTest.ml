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

(* Tests run with cwd inside `_build`, so a fixture kept in the source tree
   has to be found by walking up to the project root (same convention as
   LiquidVCTest). *)
let rec find_root (dir : string) : string =
  let candidate = Filename.concat dir "test-programs/suites/024-liquid-split" in
  if Sys.file_exists candidate then dir
  else
    let parent = Filename.dirname dir in
    if parent = dir then
      failwith "LiquidSplitPilotTest: cannot locate the project root"
    else find_root parent

let fixture path = Filename.concat (find_root (Sys.getcwd ())) path

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

(* L11d.1: the same compile, with the structured evidence pointed at a file.
   The evidence is produced by the compiler under test, not by the test, so a
   drift in what the analyser saw shows up here. *)
let compile_evidence ~(name : string) (source : string) : int * string * string =
  let dir = workdir () in
  (try Unix.mkdir dir 0o755 with _ -> ());
  let src = Filename.concat dir (name ^ ".aum") in
  let err = Filename.concat dir (name ^ ".err") in
  let ev = Filename.concat dir (name ^ ".evidence") in
  (try Sys.remove ev with _ -> ());
  write_file src source;
  let cmd =
    Printf.sprintf "AUSTRAL_LIQUID_EVIDENCE=%s %s compile %s --target-type=tc 2>%s"
      (Filename.quote ev) (Filename.quote (compiler ()))
      (Filename.quote src) (Filename.quote err)
  in
  let rc = Sys.command cmd in
  let stderr = try read_file err with _ -> "" in
  let evidence = try read_file ev with _ -> "" in
  (rc, stderr, evidence)

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

(* ── L11d.1: the held-out law, and the evidence it is compared with ────── *)

(* One fixture, with the parts each case varies made explicit: the element
   width declaration (or none), the right half's base, the case's return order,
   and the divisor the caller passes. *)
let widened ~module_name ~width ~rbase ~ret ~k = Printf.sprintf {|
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

    %s

    pragma Liquid_Requires(Contract => "0 <= k && k <= length(w)");
    function split(w: Window, k: Int64): Halves is
        let l: Window := Window(base => w.base, off => offset(w), len => k);
        let r: Window := Window(base => %s, off => offset(w) + k, len => length(w) - k);
        return %s;
    end;

    function main(): ExitCode is
        let w: Window := Window(base => 0, off => 10, len => 8);
        case split(w, %d) of
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
  module_name width rbase ret k

let width8 =
  "function element_width(): Int64 is\n        return 8;\n    end;"

let evidence_case name source ~needles =
  (* The case name is prose; the file name has to be a path. *)
  let slug =
    String.map (fun c -> if c = ' ' || c = ',' || c = '=' then '-' else c) name
  in
  name >:: fun _ ->
  let rc, err, ev = compile_evidence ~name:slug source in
  assert_bool
    (Printf.sprintf "%s should compile, but the compiler exited %d:\n%s" name rc
       err)
    (rc = 0);
  assert_bool
    (Printf.sprintf "%s: the compiler wrote no evidence file\n%s" name err)
    (String.length ev > 0);
  List.iter
    (fun needle ->
      assert_bool
        (Printf.sprintf "%s: evidence does not contain %S:\n%s" name needle ev)
        (contains ~needle ev))
    needles;
  assert_bool
    (Printf.sprintf "%s: evidence contains a [REFUSED] obligation:\n%s" name ev)
    (not (contains ~needle:"[REFUSED]" ev))

(* L11c says a branch's facts die with it. At the source level that is not
   observable as a *pilot* refusal — the binder the fact mentions is gone, so
   the typer refuses first:

       end case;
       return adjacent(left, right);   -- `left` is not in scope here

   This case pins that ordering. It is why the pilot's dropped-fact path (which
   says "you are using branch-local evidence") stays a diagnostic for the cases
   the typer lets through, instead of something a fixture can trigger at will. *)
let scope_escape =
  Printf.sprintf {|
module body SplitScope is
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

    pragma Liquid_Requires(Contract => "offset(left) + length(left) <= offset(right)");
    function adjacent(left: Window, right: Window): Int64 is
        return length(left) + length(right);
    end;

    pragma Liquid_Requires(Contract => "0 <= k && k <= length(w)");
    function split(w: Window, k: Int64): Halves is
        let l: Window := Window(base => w.base, off => offset(w), len => k);
        let r: Window := Window(base => w.base, off => offset(w) + k, len => length(w) - k);
        return Both(left => l, right => r);
    end;

    function use(w: Window, k: Int64): Int64 is
        case split(w, k) of
            when Both(left: Window, right: Window) do
                return adjacent(left, right);
            when NoHalf do
                return 0;
        end case;
        return adjacent(left, right);
    end;

    function main(): ExitCode is
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
    (* The structured obligations are read back from a real compilation, so the
       law's vocabulary is checked against what the analyser resolved. *)
    evidence_case "reflects the held-out law for off=10, n=8, k=3"
      (widened ~module_name:"SplitEvidence" ~width:width8 ~rbase:"w.base"
         ~ret:"Both(left => l, right => r)" ~k:3)
      ~needles:[ "law: liquid-split.extent-conservation";
                 "record: Window fields base,off,len";
                 "union: Halves case Both slots left,right";
                 "width: 8 (module declaration)";
                 "obligation liquid-split.extent-conservation: left = (w.off, k)";
                 "obligation liquid-split.extent-conservation: right = ((+ w.off k), (- w.len k))";
                 "obligation conservation: left.len + right.len = n [stated: L11d.2]";
                 "element ranges: left [10,13) len=3, right [13,18) len=5";
                 "conservation: 3 + 5 = 8 [checked]";
                 "disjoint: 13 <= 13 [checked]";
                 "byte ranges: left [80,104) len=24, right [104,144) len=40" ];
    evidence_case "the empty left half at k=0"
      (widened ~module_name:"SplitZero" ~width:width8 ~rbase:"w.base"
         ~ret:"Both(left => l, right => r)" ~k:0)
      ~needles:[ "element ranges: left [10,10) len=0, right [10,18) len=8";
                 "conservation: 0 + 8 = 8 [checked]";
                 "disjoint: 10 <= 10 [checked]";
                 "byte ranges: left [80,80) len=0, right [80,144) len=64" ];
    evidence_case "the empty right half at k=n"
      (widened ~module_name:"SplitFull" ~width:width8 ~rbase:"w.base"
         ~ret:"Both(left => l, right => r)" ~k:8)
      ~needles:[ "element ranges: left [10,18) len=8, right [18,18) len=0";
                 "conservation: 8 + 0 = 8 [checked]";
                 "disjoint: 18 <= 18 [checked]";
                 "byte ranges: left [80,144) len=64, right [144,144) len=0" ];
    evidence_case "states the width premise when the module declares none"
      (widened ~module_name:"SplitNoWidth" ~width:"" ~rbase:"w.base"
         ~ret:"Both(left => l, right => r)" ~k:3)
      ~needles:[ "width: host-supplied";
                 "byte ranges: [premise] width supplied by the host" ];
    expects_refusal "refuses halves returned in the wrong slot order"
      (widened ~module_name:"SplitReversed" ~width:width8 ~rbase:"w.base"
         ~ret:"Both(left => r, right => l)" ~k:3)
      ~needles:[ "L11c: `split`"; "slot order" ];
    expects_refusal "refuses a right half on another base"
      (widened ~module_name:"SplitOtherBase" ~width:width8 ~rbase:"w.off"
         ~ret:"Both(left => l, right => r)" ~k:3)
      ~needles:[ "L11c: `split` gives the halves different origins" ];
    expects_refusal "a branch's binders are gone after the case" scope_escape
      ~needles:[ "I can't find a variable named `left`" ];
    (* The end-to-end fixture is the admitted module the card names, so the
       evidence is checked against the *suite's* source and not only against a
       copy in this file. `printLn` output stays the suite's concern. *)
    "emits evidence for the 001-accept-split fixture"
      >:: (fun _ ->
            let path =
              fixture "test-programs/suites/024-liquid-split/001-accept-split/Test.aum"
            in
            let rc, err, ev =
              compile_evidence ~name:"suite-001-window" (read_file path)
            in
            assert_bool
              (Printf.sprintf
                 "%s should compile, but the compiler exited %d:\n%s" path rc err)
              (rc = 0);
            List.iter
              (fun needle ->
                assert_bool
                  (Printf.sprintf "%s: evidence does not contain %S:\n%s" path
                     needle ev)
                  (contains ~needle ev))
              [ "module: Test";
                "width: 8 (module declaration)";
                "element ranges: left [10,13) len=3, right [13,18) len=5";
                "conservation: 3 + 5 = 8 [checked]";
                "disjoint: 13 <= 13 [checked]";
                "byte ranges: left [80,104) len=24, right [104,144) len=40" ])
  ]

let () = run_test_tt_main suite
