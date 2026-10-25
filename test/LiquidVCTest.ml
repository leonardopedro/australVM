(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   Verification conditions, tested where they are observable.

   The generator's output is a `.mlw` file, so the honest test is "compile this
   source, dump the constraints, read what it says". Two of the three checks
   below are about the VC set itself:

   - **§5.6 APPLICATION (caller side)** — a caller must prove the callee's
     `Requires` with its actuals substituted. Before this, the callee's
     contract entered the `.mlw` as an axiom and nothing checked what callers
     passed it, so `docs/LIQUID.md` §11's soundness sentence was ahead of the
     code.
   - **§5.9 FOLD (result half)** — `Liquid_Invariant` used to be parsed,
     type-checked and silently dropped.

   The third is about the *spelling*: TRL and Why3 disagree on `==`, `&&`,
   `||`, `!=` and `!`, and an untranslated operator makes the file fail Why3's
   type checker, which exits 1 and is then read as "the prover refused" — the
   same false-rejection class the golden gates exist to catch.

   T5 (refined variables are immutable) is tested by calling the gate directly,
   not through source, and the reason is worth stating: `TypingPass` already
   rejects assigning to a function parameter ("Cannot assign a value to a
   function parameter") and mutable-borrowing one
   (`cannot_borrow_param_mutably`), so no source program can reach the gate
   with a violated T5. The gate's own accounting of the rule is still checked
   here — it is the second line, and it is the one that would fail closed if
   the first ever loosened.
*)
open OUnit2

(* ── harness (same shape as LiquidErasureTest: drive the real compiler) ──── *)

let compiler () =
  match Sys.getenv_opt "AUSTRAL_BIN" with
  | Some p -> p
  | None ->
     let bin_dir =
       Filename.concat (Filename.dirname (Filename.dirname Sys.executable_name)) "bin"
     in
     Filename.concat bin_dir "austral.exe"

let rec find_root (dir : string) : string =
  let candidate = Filename.concat dir "lib/liquid/golden" in
  if Sys.file_exists candidate then dir
  else
    let parent = Filename.dirname dir in
    if parent = dir then failwith "LiquidVCTest: cannot locate the project root"
    else find_root parent

let golden_dir () =
  match Sys.getenv_opt "AUSTRAL_GOLDEN_DIR" with
  | Some p -> p
  | None -> Filename.concat (find_root (Sys.getcwd ())) "lib/liquid/golden"

let workdir () =
  match Sys.getenv_opt "AUSTRAL_VC_WORKDIR" with
  | Some p -> p
  | None -> Filename.concat (Filename.dirname Sys.executable_name) "vc-scratch"

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

(* ── the fixture module ─────────────────────────────────────────────────── *)

let probe_source =
  {|
module body LiquidVCProbe is
    pragma Liquid_Requires(Contract => "n >= 0");
    pragma Liquid_Ensures(Contract => "result >= n");
    function atLeast(n: Int64): Int64 is
        return n;
    end;

    pragma Liquid_Requires(Contract => "m >= 0");
    pragma Liquid_Invariant(Contract => "result >= 0");
    pragma Liquid_Ensures(Contract => "result >= 0");
    function caller(m: Int64): Int64 is
        return atLeast(m);
    end;

    function main(): ExitCode is
        printLn(caller(3));
        return ExitSuccess();
    end;
end module body.
|}

(* Every operator the two languages spell differently, in one contract: `==`,
   `!=`, `&&`, `||`, `!`. All five are in the pinned golden this test compares
   against, so the claim "the emitter translates them" is a byte comparison
   rather than an absence of the wrong spelling. The body returns `0` — the
   least the contract could be satisfied by — so the golden proves under a
   prover as well as parses: an `Ensures` no body of this fixture could meet
   would pin an unprovability, not a translation. *)
let operators_source =
  {|
module body LiquidVCOps is
    pragma Liquid_Requires(Contract => "n >= 0 && !(n > 100)");
    pragma Liquid_Ensures(Contract => "result == 0 || (result >= 5 && result != 7)");
    function f(n: Int64): Int64 is
        return 0;
    end;

    function main(): ExitCode is
        printLn(f(3));
        return ExitSuccess();
    end;
end module body.
|}

(* A call whose actual is arithmetic. The desugarer hoists `n - 1` into a
   temporary (`_t = n - 1; atLeast(_t)`) and encodes the subtraction as a
   `TMethodCall` on `Austral.Pervasive`, so this exercises two things at once:
   resolving a hoisted actual, and rendering a `pervasive` arithmetic intrinsic
   as a TRL term. Before either, the obligation was silently skipped. *)
let arithmetic_source =
  {|
module body LiquidVCArith is
    pragma Liquid_Requires(Contract => "n >= 0");
    pragma Liquid_Ensures(Contract => "result >= n");
    function atLeast(n: Int64): Int64 is
        return n;
    end;

    pragma Liquid_Requires(Contract => "n >= 1");
    pragma Liquid_Ensures(Contract => "result >= 0");
    function caller(n: Int64): Int64 is
        return atLeast(n - 1);
    end;

    function main(): ExitCode is
        printLn(caller(5));
        return ExitSuccess();
    end;
end module body.
|}

(* A call whose actual is a *projection* of a record parameter: `atLeast(r.X)`.
   Desugaring lowers it to a borrow-and-load (`r_tmp_ref1: &r`), whose local is
   not a parameter — so until the borrow rename resolves back to `r` and the
   slot becomes an uninterpreted `__slot_X(r)`, this obligation could not be
   stated at all. The declaration of `__slot_X` is part of the claim: an
   undeclared symbol makes Why3 exit 1, which reads as "the prover refused".

   The callee's contract is the reflexive `n == n`, deliberately: what is
   under test is that the *projection is the actual* — substituted into the
   callee's `Requires` and declared — and a non-trivial claim about the slot
   (`n >= 0`) could not be discharged from the caller's context at all, since
   TRL v1 has no projection syntax for the caller to state `r.X >= 0` with
   (docs/cas.md §4). The program would be genuinely unsound under that
   contract; it is satisfiable under this one, and the golden proves. *)
let projection_source =
  {|
module body LiquidVCProj is
    record R: Free is
        X: Int32;
    end;

    pragma Liquid_Requires(Contract => "n == n");
    function atLeast(n: Int32): Int32 is
        return n;
    end;

    pragma Liquid_Requires(Contract => "true");
    function caller(r: R): Int32 is
        return atLeast(r.X);
    end;

    function main(): ExitCode is
        let rec: R := R(X => 10);
        printLn(caller(rec));
        return ExitSuccess();
    end;
end module body.
|}

(* §4.4's combinator, marked with the marker pragma, called with the shape
   the §5.9 rule is about: step, seed, list. The result half of §5.9 was
   already emitted (it is the `Liquid_Invariant` return goal); this fixture
   exists so the two *premises* — `I(init)` and `I(a) -> I(f(e,a))` — are
   stated at the call site, where they are actually checkable. The step actual
   is a function value, so the goal has to name the theory function it
   applies, and `__elem`/`__acc` are this goal's binders: nothing in the
   source introduces them. `callerViaLocal` binds the same step through a
   local first — the premise must be about *that* step function too, or the
   local would be a silent gap — and `fold2`'s `Requires` is the seed
   condition its own invariant needs (`init >= 0`), which is what makes the
   result half provable rather than merely emitted. *)
let fold_source =
  {|
module body LiquidFoldPremises is
    pragma Liquid_Requires(Contract => "true");
    pragma Liquid_Ensures(Contract => "result >= acc");
    function step(e: Int64, acc: Int64): Int64 is
        return acc;
    end;

    pragma Liquid_Fold;
    pragma Liquid_Invariant(Contract => "result >= 0");
    pragma Liquid_Requires(Contract => "init >= 0");
    function fold2(f: Fn[Int64, Int64, Int64], init: Int64, xs: Int64): Int64 is
        return init;
    end;

    pragma Liquid_Requires(Contract => "n >= 0");
    function caller(n: Int64): Int64 is
        return fold2(step, 0, n);
    end;

    pragma Liquid_Requires(Contract => "n >= 0");
    function callerViaLocal(n: Int64): Int64 is
        let s: Fn[Int64, Int64, Int64] := step;
        return fold2(s, 0, n);
    end;

    function main(): ExitCode is
        printLn(caller(3));
        printLn(callerViaLocal(3));
        return ExitSuccess();
    end;
end module body.
|}

(* A wrapper whose fold step is *its own* parameter: `callerParam` cannot
   name a function for its premise — there is nothing to name until someone
   calls it — so the obligation follows the parameter: `outer` supplies
   `step`, and the step half is emitted *there*, in outer's context. Without
   that propagation the premise would be dropped exactly when the fold is
   written abstractly, which is when it matters most. *)
let fold_param_source =
  {|
module body LiquidFoldParamStep is
    pragma Liquid_Requires(Contract => "true");
    pragma Liquid_Ensures(Contract => "result >= acc");
    function step(e: Int64, acc: Int64): Int64 is
        return acc;
    end;

    pragma Liquid_Fold;
    pragma Liquid_Invariant(Contract => "result >= 0");
    pragma Liquid_Requires(Contract => "init >= 0");
    function fold2(f: Fn[Int64, Int64, Int64], init: Int64, xs: Int64): Int64 is
        return init;
    end;

    pragma Liquid_Requires(Contract => "n >= 0");
    function callerParam(f: Fn[Int64, Int64, Int64], n: Int64): Int64 is
        return fold2(f, 0, n);
    end;

    pragma Liquid_Requires(Contract => "n >= 0");
    function outer(n: Int64): Int64 is
        return callerParam(step, n);
    end;

    function main(): ExitCode is
        printLn(outer(3));
        return ExitSuccess();
    end;
end module body.
|}

let dump_mlw ~work ~name source =
  let dir = Filename.concat work name in
  (try Unix.mkdir dir 0o755 with _ -> ());
  let src = Filename.concat dir (name ^ ".aum") in
  write_file src source;
  (* The dump directory is passed in the child's environment, not through
     `Unix.putenv`: ounit asserts that a test leaves the process environment
     unchanged, and it is right to — a test that mutates it leaks into every
     later test in the same binary.

     `--target-type=tc` stops at the typed AST: the pass runs (it is a
     `typed_pass`), and nothing reaches codegen — so the test needs neither the
     Cranelift bridge nor a JIT. *)
  let cmd =
    Printf.sprintf "AUSTRAL_LIQUID_DUMP=%s %s compile %s --target-type=tc >%s.out 2>%s.err"
      (Filename.quote dir) (Filename.quote (compiler ())) (Filename.quote src)
      (Filename.quote dir) (Filename.quote dir)
  in
  let rc = Sys.command cmd in
  let err = try read_file (Filename.concat dir (name ^ ".err")) with _ -> "" in
  assert_bool
    (Printf.sprintf "compiling %s failed (exit %d):\n%s" name rc err)
    (rc = 0);
  (* The dump is named after the *module*, not the file, so find it rather
     than assume `name ^ ".mlw"`. One module per source, so one file. *)
  let d = Unix.opendir dir in
  let found = ref [] in
  let rec scan () =
    match Unix.readdir d with
    | entry ->
       if Filename.check_suffix entry ".mlw" then found := entry :: !found;
       scan ()
    | exception End_of_file -> ()
  in
  scan ();
  Unix.closedir d;
  match !found with
  | [ file ] -> read_file (Filename.concat dir file)
  | [] -> assert_failure (Printf.sprintf "%s produced no .mlw (dump at %s)" name dir)
  | many ->
     assert_failure
       (Printf.sprintf "%s dumped %d .mlw files: %s" name (List.length many)
          (String.concat ", " many))

let contains ~(needle : string) (hay : string) : bool =
  let n = String.length needle and h = String.length hay in
  let rec at i =
    if i + n > h then false
    else if String.sub hay i n = needle then true
    else at (i + 1)
  in
  n = 0 || at 0

(** How many disjoint occurrences of [needle] appear — non-overlapping, so a
    count of premises is a count of premises, not of substrings. *)
let count_occurrences ~(needle : string) (hay : string) : int =
  let n = String.length needle and h = String.length hay in
  let rec at i acc =
    if i + n > h then acc
    else if String.sub hay i n = needle then at (i + n) (acc + 1)
    else at (i + 1) acc
  in
  if n = 0 then 0 else at 0 0

(** Only the lines that are *logic*, not commentary.

    `to_mlw` echoes every contract string as a comment above the module, so
    asserting on the whole file would report `==` as untranslated when it is
    merely quoted back at the reader. The claim under test is about the goals
    and the theory axioms — the terms Why3 has to parse as logic. *)
let logical_lines (mlw : string) : string =
  String.concat "\n"
    (List.filter
       (fun line ->
         let s = String.trim line in
         String.length s > 5
         && (String.sub s 0 5 = "goal "
             || (String.length s > 6 && String.sub s 0 6 = "axiom ")
             || (String.length s > 9 && String.sub s 0 9 = "function ")))
       (String.split_on_char '\n' mlw))

(* ── 1. the pinned golden ───────────────────────────────────────────────── *)

let test_golden_matches_emitter (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let actual = dump_mlw ~work ~name:"probe" probe_source in
  let golden = Filename.concat (golden_dir ()) "vc-callsite-and-invariant.mlw" in
  if not (Sys.file_exists golden) then
    assert_failure
      (Printf.sprintf "missing golden %s — regenerate with AUSTRAL_LIQUID_DUMP"
         golden);
  let expected = read_file golden in
  assert_bool
    ("the emitted .mlw drifted from the pinned golden.\n--- golden ---\n"
     ^ expected ^ "\n--- emitted ---\n" ^ actual)
    (expected = actual)

(* The fold premises are pinned the same way: the emitter's §5.9 output is a
   fixed artefact, so a change in any of the three halves — the binder, the
   substituted seed, the applied step — shows up as a byte diff, and the
   `why3 prove --type-only` gate in `scripts/verify-invariants` runs over this
   file along with the others. The multi-parameter theory function it contains
   is what surfaced the curried-parameters defect: `function step (e: int,
   acc: int)` is a Why3 syntax error, and every other golden carried a
   one-parameter function only. *)
let test_golden_fold_matches_emitter (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let actual = dump_mlw ~work ~name:"foldgolden" fold_source in
  let golden = Filename.concat (golden_dir ()) "vc-fold-premises.mlw" in
  if not (Sys.file_exists golden) then
    assert_failure
      (Printf.sprintf "missing golden %s — regenerate with AUSTRAL_LIQUID_DUMP"
         golden);
  let expected = read_file golden in
  assert_bool
    ("the emitted fold .mlw drifted from the pinned golden.\n--- golden ---\n"
     ^ expected ^ "\n--- emitted ---\n" ^ actual)
    (expected = actual)

(* ── 2. the two new obligations are actually stated ─────────────────────── *)

let test_callsite_obligation_is_stated (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let mlw = dump_mlw ~work ~name:"callsite" probe_source in
  assert_bool "§5.6 goal missing from the emitted .mlw"
    (contains ~needle:"§5.6 APPLICATION (the caller proves the callee's Requires)" mlw);
  (* The obligation must talk about the *actual*, not the callee's formal `n`:
     `caller` passes `m`, and its own Requires is the assumption. *)
  assert_bool "the call-site goal does not substitute the actual argument"
    (contains ~needle:"forall m: int. (m >= 0) -> ((m >= 0))" mlw)

let test_invariant_obligation_is_stated (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let mlw = dump_mlw ~work ~name:"invariant" probe_source in
  assert_bool "§5.9 goal missing: Liquid_Invariant was dropped"
    (contains ~needle:"§5.9 FOLD (result half; the step obligation needs §4.4)" mlw)

(* §5.9's two premises, from the fold call in `fold_source`. Both were
   documented and neither was emitted: the rule's result half already had a
   goal (asserted above), and the comment on it said the step obligation
   needed §4.4 — which this module provides. *)
let test_fold_premises_are_stated (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let mlw = dump_mlw ~work ~name:"foldpremises" fold_source in
  assert_bool "§5.9 init premise missing from the emitted .mlw"
    (contains ~needle:"§5.9 FOLD (init half)" mlw);
  assert_bool "the init premise does not conclude I(seed)"
    (contains ~needle:"-> ((0 >= 0))" mlw);
  assert_bool "§5.9 step premise missing from the emitted .mlw"
    (contains ~needle:"§5.9 FOLD (step half)" mlw);
  (* The step goal binds its own `__elem`/`__acc` and applies the theory
     function by the name the step actual resolved to. *)
  assert_bool "the step premise does not bind e and a"
    (contains ~needle:"forall n: int, __elem: int, __acc: int. " mlw);
  assert_bool "the step premise does not state I(a) -> I(f(e,a))"
    (contains ~needle:"((__acc >= 0)) -> (((step (__elem) (__acc)) >= 0))" mlw);
  (* The same premise for the caller that binds the step through a local:
     exactly two step halves, one per fold call, not one per combinator. *)
  let steps = count_occurrences ~needle:"§5.9 FOLD (step half)" mlw in
  assert_bool
    (Printf.sprintf "expected one step premise per fold call (found %d)" steps)
    (steps = 2)

(* The deferred premise: `callerParam`'s step is its parameter, so its own
   fold call states only the init half — the step half appears once, at the
   call site that supplies the function, and in *that* caller's context
   (outer's `n`, not callerParam's `f`). *)
let test_fold_param_step_is_discharged_where_the_actual_is (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let mlw = dump_mlw ~work ~name:"foldparamstep" fold_param_source in
  assert_bool
    ("the wrapper's own init half is missing:\n" ^ mlw)
    (contains ~needle:"§5.9 FOLD (init half)" mlw);
  assert_bool
    ("the deferred step half was not emitted at the site that supplies the \
     function:\n" ^ mlw)
    (contains ~needle:"§5.9 FOLD (step half)" mlw
     && contains ~needle:"((step (__elem) (__acc)) >= 0)" mlw);
  assert_bool
    ("the step half is not in the supplying caller's context (or was emitted \
     twice):\n" ^ mlw)
    (contains ~needle:"forall n: int, __elem: int, __acc: int. " mlw);
  let steps = count_occurrences ~needle:"§5.9 FOLD (step half)" mlw in
  assert_bool
    (Printf.sprintf
       "expected exactly one step premise (at outer's site), found %d:\n%s"
       steps mlw)
    (steps = 1)

(* ── 3. TRL connectives are spelled the way Why3 parses them ────────────── *)

let test_operators_are_translated (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let mlw = dump_mlw ~work ~name:"operators" operators_source in
  let logic = logical_lines mlw in
  assert_bool ("`==` reached the goals (Why3: unbound symbol (==)):\n" ^ logic)
    (not (contains ~needle:"==" logic));
  assert_bool ("`!=` reached the goals:\n" ^ logic)
    (not (contains ~needle:"!=" logic));
  assert_bool ("`&&` reached the goals:\n" ^ logic)
    (not (contains ~needle:"&&" logic));
  assert_bool ("`||` reached the goals:\n" ^ logic)
    (not (contains ~needle:"||" logic));
  assert_bool ("conjunction is not spelled /\\:\n" ^ logic)
    (contains ~needle:"(n >= 0 /\\ not (n > 100))" logic);
  (* One assertion per operator, spelled the way Why3 reads it. *)
  assert_bool ("disjunction/equality are not spelled \\/ and =:\n" ^ logic)
    (contains ~needle:"__result = 0 \\/ (__result >= 5" logic);
  assert_bool ("inequality is not spelled <>:\n" ^ logic)
    (contains ~needle:"__result <> 7" logic);
  (* And pinned: the translated output is a fixed artefact, so a change in
     *any* of the five spellings — not only the ones asserted above — shows up
     as a diff. This is the golden cas.md §2.2 item 5 names; the per-operator
     assertions stay because they name the failure when the diff fires. *)
  let golden = Filename.concat (golden_dir ()) "vc-operators.mlw" in
  if not (Sys.file_exists golden) then
    assert_failure
      (Printf.sprintf "missing golden %s — regenerate with AUSTRAL_LIQUID_DUMP"
         golden);
  let expected = read_file golden in
  assert_bool
    ("the emitted .mlw drifted from the pinned operator golden.\n--- golden ---\n"
     ^ expected ^ "\n--- emitted ---\n" ^ mlw)
    (expected = mlw)

(* ── 3b. a hoisted arithmetic actual still gets an obligation ───────────── *)

let test_arithmetic_actuals_are_stated (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let mlw = dump_mlw ~work ~name:"arithmetic" arithmetic_source in
  (* The point of the assertion is the *substituted* formula: `n - 1` in place
     of the callee's formal, with the caller's own Requires as the
     assumption. A skipped obligation would remove the whole goal. *)
  assert_bool
    ("the call-site obligation for an arithmetic actual is missing:\n" ^ mlw)
    (contains ~needle:"((n - 1) >= 0)" mlw);
  (* §6: the same operation, checked for overflow rather than for its
     contract. The bound comes from the sort environment (`int_bounds_of_ty`)
     and the `min ≤ n ≤ max` assumption about the parameter is what makes the
     obligation provable rather than merely emitted. *)
  assert_bool
    ("the §6 range obligation for `n - 1` is missing:\n" ^ mlw)
    (contains ~needle:"§6 RANGE" mlw
     && contains ~needle:"(-9223372036854775808 <= (n - 1))" mlw
     && contains ~needle:"(-9223372036854775808 <= n)" mlw);
  let golden = Filename.concat (golden_dir ()) "vc-arithmetic.mlw" in
  if not (Sys.file_exists golden) then
    assert_failure
      (Printf.sprintf "missing golden %s — regenerate with AUSTRAL_LIQUID_DUMP"
         golden);
  let expected = read_file golden in
  assert_bool
    ("the emitted .mlw drifted from the pinned arithmetic golden.\n--- golden ---\n"
     ^ expected ^ "\n--- emitted ---\n" ^ mlw)
    (expected = mlw)

(* ── 4. a record projection can be stated, and declared ──────────────────── *)

let test_projection_actuals_are_stated (_ : OUnit2.test_ctxt) =
  let work = workdir () in
  (try Unix.mkdir work 0o755 with _ -> ());
  let mlw = dump_mlw ~work ~name:"projection" projection_source in
  (* The goal itself: the callee's contract over the actual `r.X`, with the
     borrow rename resolved back to the parameter `r`. *)
  assert_bool
    ("the projection obligation is missing (the actual was skipped):\n" ^ mlw)
    (contains ~needle:"__slot_X (r)" mlw);
  (* And the symbol has to be declared, or `why3 prove` exits 1 on an unbound
     symbol — which the driver reads as a refusal, i.e. a fabricated
     rejection. This is why the emitter drops comparison terms instead. *)
  assert_bool
    ("__slot_X is used but not declared:\n" ^ mlw)
    (contains ~needle:"function __slot_X (x0: int) : int" mlw);
  let golden = Filename.concat (golden_dir ()) "vc-projection.mlw" in
  if not (Sys.file_exists golden) then
    assert_failure
      (Printf.sprintf "missing golden %s — regenerate with AUSTRAL_LIQUID_DUMP"
         golden);
  let expected = read_file golden in
  assert_bool
    ("the emitted .mlw drifted from the pinned projection golden.\n--- golden ---\n"
     ^ expected ^ "\n--- emitted ---\n" ^ mlw)
    (expected = mlw)

(* ── 5. T5 by direct call (see the header for why not through source) ───── *)

let span = TestUtil.span 1 1 1 2

let ident (s : string) : Identifier.identifier =
  Identifier.make_ident s

let qident (s : string) : Identifier.qident =
  Identifier.make_qident
    ( Identifier.make_mod_name "Probe",
      ident s,
      ident s )

let graph () : (int, int list) Hashtbl.t = Hashtbl.create 4

let foreign () : (int, unit) Hashtbl.t = Hashtbl.create 4

let names () : (int, string) Hashtbl.t =
  let h = Hashtbl.create 4 in
  Hashtbl.replace h 0 "f";
  h

let int64_ty : Type.ty =
  Type.Integer (Type.Signed, Type.Width64)

let run_t5 ~(refined : string list) (body : Stages.Tast.tstmt) : string option =
  TotalityCheck.check_function ~name:"f" ~entry:(Id.DeclId 0) ~body
    ~graph:(graph ()) ~foreign:(foreign ()) ~names:(names ()) ~refined

let test_t5_rejects_assignment_to_refined_parameter (_ : OUnit2.test_ctxt) =
  (* `n := 0;` — the shape TypingPass rejects first. *)
  let body =
    Stages.Tast.TAssignVar
      ( span,
        qident "n",
        Stages.Tast.TIntConstant "0" )
  in
  match run_t5 ~refined:[ "n" ] body with
  | Some msg ->
     assert_bool ("T5 diagnostic does not name the parameter: " ^ msg)
       (contains ~needle:"`n`" msg);
     assert_bool ("T5 diagnostic is not T5: " ^ msg)
       (contains ~needle:"T5 (§4.1)" msg)
  | None -> assert_failure "T5 did not fire on an assignment to a refined parameter"

let test_t5_rejects_write_borrow_of_refined_parameter (_ : OUnit2.test_ctxt) =
  let body =
    Stages.Tast.TBorrow
      {
        span;
        original = ident "n";
        rename = ident "w";
        region = ident "r";
        orig_type = int64_ty;
        ref_type = Type.WriteRef (int64_ty, int64_ty);
        body = Stages.Tast.TSkip span;
        mode = Common.Write;
      }
  in
  match run_t5 ~refined:[ "n" ] body with
  | Some msg ->
     assert_bool ("T5 diagnostic does not mention T5: " ^ msg)
       (contains ~needle:"T5 (§4.1)" msg)
  | None -> assert_failure "T5 did not fire on a write borrow of a refined parameter"

let test_t5_ignores_unrefined_parameters (_ : OUnit2.test_ctxt) =
  (* The same assignment, but `n` carries no contract: outside the fragment's
     assertions, so the gate must stay silent. This is what keeps the rule from
     becoming "parameters are immutable" — they already are, elsewhere. *)
  let body =
    Stages.Tast.TAssignVar
      ( span,
        qident "n",
        Stages.Tast.TIntConstant "0" )
  in
  assert_bool "T5 fired for a parameter no contract refines"
    (run_t5 ~refined:[] body = None)

let suite =
  "LiquidVCTest" >::: [
    "golden_matches_emitter" >:: test_golden_matches_emitter;
    "golden_fold_matches_emitter" >:: test_golden_fold_matches_emitter;
    "callsite_obligation_is_stated" >:: test_callsite_obligation_is_stated;
    "invariant_obligation_is_stated" >:: test_invariant_obligation_is_stated;
    "fold_premises_are_stated" >:: test_fold_premises_are_stated;
    "fold_param_step_discharged_at_call_site"
    >:: test_fold_param_step_is_discharged_where_the_actual_is;
    "operators_are_translated" >:: test_operators_are_translated;
    "arithmetic_actuals_are_stated" >:: test_arithmetic_actuals_are_stated;
    "projection_actuals_are_stated" >:: test_projection_actuals_are_stated;
    "t5_rejects_assignment_to_refined_parameter"
    >:: test_t5_rejects_assignment_to_refined_parameter;
    "t5_rejects_write_borrow_of_refined_parameter"
    >:: test_t5_rejects_write_borrow_of_refined_parameter;
    "t5_ignores_unrefined_parameters" >:: test_t5_ignores_unrefined_parameters;
  ]

let () = run_test_tt_main suite
