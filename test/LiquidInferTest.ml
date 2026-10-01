(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L6: qualifier inference, tested without a prover.

   The elimination logic is the part that can be *wrong* — a fixed point that
   does not terminate, a clause that drops a premise, an oracle consulted on the
   wrong side — and none of that needs Why3 to check. These tests drive
   `LiquidInfer.houdini` directly with fake oracles, which is the reason the
   oracle is a parameter rather than a hard-coded `why3 prove`.
*)
open OUnit2
open TestUtil
open LiquidInfer
open LiquidTypes
open Stages.Tast

let sp = { LiquidTypes.start = 0; stop = 0 }

let var n = TVar (n, None, sp)
let lit i = TInt (Z.of_int i, sp)

(* q for: i < n *)
let cond_q name =
  { qname = name;
    qargs = [ "i"; "n" ];
    qbody = FRel (RLt, var "i", var "n", sp);
    qspan = "test" }

let plain_q name =
  { qname = name; qargs = []; qbody = FTrue sp; qspan = "test" }

(* An oracle that refutes every qualifier it is shown. *)
let refute_all _ = false

(* An oracle that proves everything: nothing is ever eliminated. *)
let prove_all _ = true

(* An oracle that refutes only names containing "bad". *)
let refute_named bad q =
  match q.qname with
  | _ when String.length q.qname >= String.length bad ->
     String.sub q.qname 0 (String.length bad) = bad
  | _ -> false

let suite =
  "LiquidInferTest" >::: [

    "no_clauses_means_no_elimination" >:: (fun _ ->
       (* With an empty clause set Houdini has nothing to refute, so every
          candidate survives — even against an all-refuting oracle. This is the
          property that makes a missing prover safe: it cannot drop a goal. *)
       let ts = [ plain_q "a"; plain_q "b" ] in
       let (surv, rounds) = houdini refute_all ts [] in
       eq 2 (List.length surv);
       eq 0 rounds);

    "one_clause_eliminates_its_conclusion" >:: (fun _ ->
       (* premises = [q], conclusion = q: q cannot hold, so Houdini drops it. *)
       let q = plain_q "p" in
       let ts = [ q ] in
       let cl = [ { premises = [ q ]; conclusion = q } ] in
       let (surv, _) = houdini refute_all ts cl in
       eq 0 (List.length surv));

    "a_refuted_premise_saves_the_conclusion" >:: (fun _ ->
       (* p ⇒ q, where only `p` is refutable. Round 1 drops p. In round 2 q's
          only premise is gone, so q is no longer a candidate and survives.
          This is the whole reason Houdini iterates: a one-pass rule would
          either keep p (unsound) or take q down with it (wrong). *)
       let p = plain_q "p" in
       let q = plain_q "q" in
       let cl =
         [ { premises = [ p ]; conclusion = p };
           { premises = [ p ]; conclusion = q } ]
       in
       let oracle_fn (x : qualifier) =
         match x.qname with "p" -> false | _ -> true
       in
       let (surv, rounds) = houdini oracle_fn [ p; q ] cl in
       eq 1 (List.length surv);
       (match surv with
        | [ x ] -> eq "q" x.qname
        | _ -> assert_failure "expected exactly q to survive");
       (* `rounds` counts passes that actually eliminated something: one here.
          The pass after it found no candidate and exited, which is the
          termination condition. *)
       assert_bool "exactly one elimination pass" (rounds = 1));

    "axioms_are_never_eliminated" >:: (fun _ ->
       (* A clause with no premises is an axiom; its conclusion is
          untouchable however hostile the oracle. *)
       let a = plain_q "ax" in
       let cl = [ { premises = []; conclusion = a } ] in
       let (surv, _) = houdini refute_all [ a ] cl in
       eq 1 (List.length surv));

    "a_proving_oracle_keeps_everything" >:: (fun _ ->
       let q = plain_q "p" in
       let cl = [ { premises = [ q ]; conclusion = q } ] in
       let (surv, _) = houdini prove_all [ q ] cl in
       eq 1 (List.length surv));

    "elimination_is_order_independent" >:: (fun _ ->
       (* The fixed point must not depend on the order candidates were
          discovered in — the walk order is an implementation detail. *)
       let a = plain_q "a" in
       let b = plain_q "b" in
       let c = plain_q "c" in
       let cl =
         [ { premises = [ a ]; conclusion = a };
           { premises = [ a ]; conclusion = b };
           { premises = [ b ]; conclusion = c } ]
       in
       let fwd, _ = houdini refute_all [ a; b; c ] cl in
       let rev, _ = houdini refute_all [ c; b; a ] cl in
       eq (List.map (fun q -> q.qname) fwd) (List.map (fun q -> q.qname) rev));

    "the_fixed_point_terminates" >:: (fun _ ->
       (* A 40-long chain q0 ⇒ q1 ⇒ … ⇒ q39, with an oracle that refutes only
          q00. Everything downstream of the refuted premise is *saved* rather
          than swept away, and the loop stops. The bound is the point: with no
          widening required (docs/LIQUID.md §5.1 — the fragment is
          recursion-free) the rounds cannot exceed the candidate count. *)
       let qs = List.init 40 (fun i -> plain_q (Printf.sprintf "q%02d" i)) in
       let cl =
         List.mapi
           (fun i q ->
             let premise = if i = 0 then q else List.nth qs (i - 1) in
             { premises = [ premise ]; conclusion = q })
           qs
       in
       let oracle_fn (x : qualifier) = x.qname <> "q00" in
       let (surv, rounds) = houdini oracle_fn qs cl in
       assert_bool "q00 is the only elimination"
         (not (List.exists (fun q -> q.qname = "q00") surv));
       assert_bool "everything downstream survives"
         (List.length surv >= 39);
       assert_bool
         ("rounds must be bounded by the candidate count, got "
          ^ string_of_int rounds)
         (rounds <= 41));

    "an_all_refuting_oracle_clears_the_whole_set" >:: (fun _ ->
       (* The degenerate case worth pinning: with every candidate refutable,
          one pass empties the set and the loop exits. It must terminate
          rather than iterate over an empty assumption set forever. *)
       let qs = List.init 10 (fun i -> plain_q (Printf.sprintf "r%d" i)) in
       let cl =
         List.map (fun q -> { premises = [ q ]; conclusion = q }) qs
       in
       let (surv, rounds) = houdini refute_all qs cl in
       eq 0 (List.length surv);
       assert_bool "and it stops promptly" (rounds <= 2));

    "equivalence_ignores_span_noise" >:: (fun _ ->
       (* Two candidates that differ only in their recorded span are the same
          qualifier: Tast carries no real spans, so keying on them would make
          every template unique and defeat elimination entirely. *)
       let a = { (plain_q "z") with qspan = "one" } in
       let b = { (plain_q "z") with qspan = "two" } in
       assert_bool "span must not affect equivalence" (qual_equiv a b));

    "dedup_keeps_one_copy" >:: (fun _ ->
       let q = cond_q "dup" in
       let q' = { q with qspan = "elsewhere" } in
       let ts = List.sort_uniq compare_qualifier [ q; q' ] in
       eq 1 (List.length ts));

    "default_measures_are_declared_once" >:: (fun _ ->
       let names = List.map fst default_measures in
       eq 3 (List.length default_measures);
       assert_bool "length is a default measure" (is_measure "length");
       assert_bool "prob_in is a default measure" (is_measure "prob_in");
       assert_bool "an unknown symbol is not" (not (is_measure "not_a_measure"));
       (* The parser must agree: `prob_in(0.5)` is a TCall, and the emitter
          only needs the name to match one of these. *)
       let f =
         match LiquidParse.parse_formula "prob_in(1) <= 1" with
         | f -> f
         | exception Failure _ -> assert_failure "a default measure must parse"
       in
       ignore names;
       match f with
       | FRel (RLe, TCall (name, _, _), _, _) -> eq "prob_in" name
       | _ -> assert_failure "expected a call to prob_in in the parsed formula");

    "translation_only_covers_conditions" >:: (fun _ ->
       (* The translator is deliberately partial: an expression it cannot
          translate must yield None rather than a wrong formula, so no
          unjustified template is ever created. A string constant is not a
          condition, and `formula_of` says so instead of guessing. *)
       let str_e = TStringConstant (Escape.escape_string "not a condition") in
       assert_bool "a string constant is not translatable"
         (LiquidInfer.formula_of str_e = None);
       (* ...while a real condition is, and carries the variables it mentions
          so the template gets the right arity. *)
       let cmp = TComparison (Common.LessThan, TLocalVar (Identifier.make_ident "i", Type.Integer (Type.Signed, Type.Width64)),
                             TLocalVar (Identifier.make_ident "n", Type.Integer (Type.Signed, Type.Width64))) in
       (match LiquidInfer.formula_of cmp with
        | None -> assert_failure "a comparison must translate"
        | Some (_, vars) ->
           eq 2 (List.length vars);
           assert_bool "both variables are captured"
             (List.mem "i" vars && List.mem "n" vars)))
  ]

let () = run_test_tt_main suite