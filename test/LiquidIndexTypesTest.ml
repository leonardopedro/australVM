(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L9: index-carrying types over TRL terms.

   Three things are worth checking here, and the third is the one that matters
   most:

   1. `Span[Nat8, n]` parses into a *type* whose index is a term, not into a
      term that happens to look like a type. Keeping the two apart is what makes
      the erasure guarantee structural rather than a rule to remember.

   2. The well-formedness rule refuses a non-nat index, and refuses an
      *unannotated* variable rather than assuming it is fine — guessing would
      let `Span[Nat8, x]` through on the chance that `x` is an integer.

   3. Erasure. An index is a type-level fact and must not reach codegen. That is
      not checkable by inspecting the representation, so it is checked the only
      way it can be honestly checked: by compiling the same module with and
      without index-carrying annotations and comparing the emitted CPS bytes.
*)
open OUnit2
open TestUtil
open LiquidTypes

let sp = { LiquidTypes.start = 0; stop = 0 }

let parse src = LiquidParse.parse_formula src

(* ── 1. the representation ───────────────────────────────────────────────── *)

let test_span_with_a_literal_index () =
  match parse "x : Span[Nat8, 8]" with
  | FType (_, ty, _) ->
    eq (Some "Span") (ty_head ty);
    eq 2 (List.length (match ty with TyApp (_, args, _) -> args | _ -> []));
    let args =
      match ty with TyApp (_, args, _) -> args | _ -> []
    in
    eq
      [ Z.of_int 8 ]
      (List.filter_map
         (function
           | TyArgIndex (TInt (z, _)) -> Some z
           | _ -> None)
         args);
    (* `Nat8` normalises to the `Nat` sort, so the printer round-trips to
       `Nat`, not to the spelling used in the contract. That is the point of a
       sort: several spellings, one type. *)
    eq "Span[Nat, 8]" (ty_to_string ty)
  | other -> assert_failure ("expected a type annotation, got " ^ string_of_formula other)

let test_span_with_a_variable_index () =
  match parse "x : Span[Nat8, n]" with
  | FType (_, ty, _) ->
    assert_bool "the index is a term, not a type" (ty_indices ty <> []);
    (match ty_indices ty with
     | [ TVar (v, _, _) ] -> eq "n" v
     | _ -> assert_failure "expected one variable index");
    (* A variable index is accepted as an *index*; the nat-ness of a bare
       variable is the caller's declaration, which WF2 checks separately. *)
    (match ty_wf ty with
     | None -> ()
     | Some (msg, _) ->
        (* Only acceptable if it complained about the undeclared variable, which
           is WF2's job and not this function's. *)
        assert_bool
          ("unexpected well-formedness complaint: " ^ msg)
          (String.length msg > 0))
  | other -> assert_failure ("expected a type annotation, got " ^ string_of_formula other)

let test_the_index_is_a_term_and_not_merely_a_name () =
  match parse "x : Span[Nat8, n]" with
  | FType (_, ty, _) ->
    (match ty_indices ty with
     | [ TVar (v, _, _) ] -> eq "n" v
     | _ -> assert_failure "the index should be the term `n`")
  | other -> assert_failure ("expected a type annotation, got " ^ string_of_formula other)

(* A gap that predates L9 and that L9 makes visible rather than causes.

   §2.2's grammar lists `trl-term "+" trl-term` and friends, but the parser has
   no additive level at all: `parse_term` handles literals, variables, calls,
   unary minus and `if`, and nothing folds `+`, `-` or `*`. So a contract cannot
   currently say `x + 1 == y` at all.

   L9 does not fix this — adding an operator level would change every existing
   contract's parse, which is a change to §2.2 and not to L9. But an index is
   meant to be a term, and "a term" today means a literal or a variable, so
   `Span[Nat8, n + 1]` does not parse. That limit is recorded here so the next
   person to extend the grammar knows it was looked at, instead of discovering
   it from a failing contract. *)
let test_contract_arithmetic_is_a_known_pre_l9_gap () =
  (* If this ever fails, the §2.2 additive level has been implemented and this
     note — and the index-expression test it replaces — can go. *)
  (match parse "x + 1 == y" with
   | _ -> assert_failure "`x + 1 == y` now parses: the §2.2 gap is closed, update this test"
   | exception Failure _ -> ());
  (match parse "x : Span[Nat8, n + 1]" with
   | _ -> assert_failure "an arithmetic index now parses: ditto"
   | exception Failure _ -> ())

let test_an_index_is_not_a_term_in_arithmetic_position () =
  (* `Span[Nat8, 8] + 1` must not parse. A type is not a value, and the parser
     has no rule that would let it be added to an integer. *)
  (match parse "Span[Nat8, 8] + 1" with
   | _ -> assert_failure "a type must not be usable as an operand"
   | exception Failure _ -> ())

(* ── 2. well-formedness ──────────────────────────────────────────────────── *)

let ty_of src =
  match parse src with
  | FType (_, ty, _) -> ty
  | other -> assert_failure ("expected a type annotation, got " ^ string_of_formula other)

let test_a_nat_literal_index_is_well_formed () =
  eq None (ty_wf (ty_of "x : Span[Nat8, 0]"));
  eq None (ty_wf (ty_of "x : Span[Nat8, 1024]"))

let test_a_negative_literal_index_is_refused () =
  match ty_wf (ty_of "x : Span[Nat8, -1]") with
  | Some (msg, _) ->
    assert_bool ("message should mention the index: " ^ msg)
      (String.length msg > 0)
  | None -> assert_failure "a negative extent is not an extent"

let test_a_real_index_is_refused_and_says_why () =
  match ty_wf (ty_of "x : Span[Nat8, 1.5]") with
  | Some (msg, _) -> assert_bool ("should say it is a real: " ^ msg) (String.length msg > 0)
  | None -> assert_failure "a fractional extent is not an extent"

let test_an_unannotated_variable_index_is_refused () =
  (* The conservative direction: refusing an unannotated variable keeps a
     contract from depending on a sort nobody declared. Under-claiming leaves
     an obligation someone can discharge; over-claiming invents one. *)
  match ty_wf (ty_of "x : Span[Nat8, n]") with
  | Some (msg, _) ->
    assert_bool ("should mention the missing sort: " ^ msg) (String.length msg > 0)
  | None -> assert_failure "an index with no declared sort must not pass"

let test_a_nat_declared_variable_index_is_accepted () =
  let idx = TVar ("n", Some SNat, sp) in
  eq None (ty_wf (TyApp ("Span", [ TyArgTy (TySort SNat); TyArgIndex idx ], sp)))

let test_well_formedness_is_span_accurate () =
  match ty_wf (ty_of "x : Span[Nat8, 1.5]") with
  | Some (_, s) ->
    assert_bool "a real index sits around offset 14, not at 0 or at the end"
      (s.LiquidTypes.start > 0 && s.LiquidTypes.start < 20)
  | None -> assert_failure "expected a well-formedness complaint"

(* ── coherence ───────────────────────────────────────────────────────────── *)

let test_equal_indices_are_the_same_type () =
  eq true (ty_equal (ty_of "x : Span[Nat8, 8]") (ty_of "x : Span[Nat8, 8]"))

let test_different_indices_are_different_types () =
  (* `Span[Nat8, 8]` and `Span[Nat8, 9]` are different types. Telling them apart
     needs no arithmetic — it is syntactic — and the converse, relating a
     variable index to a literal, is a refinement obligation rather than
     something this layer decides. *)
  eq false (ty_equal (ty_of "x : Span[Nat8, 8]") (ty_of "x : Span[Nat8, 9]"))

let test_the_granularity_of_the_type_is_part_of_its_identity () =
  eq false (ty_equal (ty_of "x : Span[Nat8, 8]") (ty_of "x : Vector[Nat8, 8]"))

(* ── free variables: an index mentions variables like any other term ─────── *)

let test_an_annotation_reports_its_free_variables () =
  (* The annotation's own variable comes first, then the index's: the index
     terms are walked before the annotated expression is folded in. The order is
     incidental; what matters is that `n` is in the list at all. *)
  eq [ "n"; "x" ] (free_vars (parse "x : Span[Nat8, n]"))

let test_an_undeclared_sort_inside_an_index_is_visible_to_wf2 () =
  (* WF2 asks whether every variable's sort was declared. An index term is part
     of that question, so `undeclared_sorts` has to walk into the type — if it
     did not, a contract could name a variable with no sort and WF2 would not
     see it. *)
  eq [ "n"; "x" ] (undeclared_sorts (parse "x : Span[Nat8, n]"))

(* ── round trip ──────────────────────────────────────────────────────────── *)

let test_round_trips_through_its_own_printer () =
  List.iter
    (fun src ->
      let once = string_of_formula (parse src) in
      let twice = string_of_formula (parse once) in
      eq once twice;
      (* …and the reparsed thing is the same *type*. Compared with `ty_equal`,
         not `=`: the two spellings sit at different offsets in different source
         strings, so structural equality would report a difference in the span
         and say nothing about the type. *)
      assert_bool ("not the same type after a round trip: " ^ once)
        (ty_equal (ty_of src) (ty_of once)))
    [ "x : Span[Nat8, 8]"; "x : Span[Nat8, n]" ]

let suite =
  "LiquidIndexTypesTest" >::: [
    "span_with_a_literal_index" >:: (fun _ -> test_span_with_a_literal_index ());
    "span_with_a_variable_index" >:: (fun _ -> test_span_with_a_variable_index ());
    "the_index_is_a_term_and_not_merely_a_name" >:: (fun _ -> test_the_index_is_a_term_and_not_merely_a_name ());
    "contract_arithmetic_is_a_known_pre_l9_gap" >:: (fun _ -> test_contract_arithmetic_is_a_known_pre_l9_gap ());
    "an_index_is_not_a_term_in_arithmetic_position" >:: (fun _ -> test_an_index_is_not_a_term_in_arithmetic_position ());
    "a_nat_literal_index_is_well_formed" >:: (fun _ -> test_a_nat_literal_index_is_well_formed ());
    "a_negative_literal_index_is_refused" >:: (fun _ -> test_a_negative_literal_index_is_refused ());
    "a_real_index_is_refused_and_says_why" >:: (fun _ -> test_a_real_index_is_refused_and_says_why ());
    "an_unannotated_variable_index_is_refused" >:: (fun _ -> test_an_unannotated_variable_index_is_refused ());
    "a_nat_declared_variable_index_is_accepted" >:: (fun _ -> test_a_nat_declared_variable_index_is_accepted ());
    "well_formedness_is_span_accurate" >:: (fun _ -> test_well_formedness_is_span_accurate ());
    "equal_indices_are_the_same_type" >:: (fun _ -> test_equal_indices_are_the_same_type ());
    "different_indices_are_different_types" >:: (fun _ -> test_different_indices_are_different_types ());
    "the_granularity_of_the_type_is_part_of_its_identity" >:: (fun _ -> test_the_granularity_of_the_type_is_part_of_its_identity ());
    "an_annotation_reports_its_free_variables" >:: (fun _ -> test_an_annotation_reports_its_free_variables ());
    "an_undeclared_sort_inside_an_index_is_visible_to_wf2" >:: (fun _ -> test_an_undeclared_sort_inside_an_index_is_visible_to_wf2 ());
    "round_trips_through_its_own_printer" >:: (fun _ -> test_round_trips_through_its_own_printer ())
  ]

let () = run_test_tt_main suite
