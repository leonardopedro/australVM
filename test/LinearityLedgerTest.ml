(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(* Unit tests for the linearity ledger (lib/LinearityLedger.ml).

   The ledger's whole value is that a *tree* of checker decisions becomes a
   set of *lines* an automaton can accept, so the interesting cases are all
   about branch paths:

     - an event recorded before an `if` belongs to both arms' timelines;
     - the arms themselves do not appear in each other's timelines;
     - only maximal paths are emitted (a variable consumed before the `if`
       must not also produce an unbalanced "entry" claim);
     - a variable that exists only inside one arm appears only there.

   The wire contracts the certificate pipeline depends on are pinned too:
   the event tokens ([event_token], read by scripts/ledger2mm0) and the
   theory constructors ([event_rule], declared by docs/memsafe.mm0). *)

open OUnit2
open LinearityLedger

let row ?(ty = "Handle") ?(depth = 0) ?(path = []) var event =
  { var; ty; depth; event; path }

let open_ = LinearityLedger.Acquire
let cons = LinearityLedger.Consume

let test_path_label _ =
  assert_equal "entry" (LinearityLedger.path_label []);
  (* Outermost branch first, whatever the internal (innermost-first) order. *)
  assert_equal "b0.a0" (LinearityLedger.path_label [(0, 0)]);
  assert_equal "b0.a1/b2.a0" (LinearityLedger.path_label [(2, 0); (0, 1)])

let test_single_path_timeline _ =
  let rows = [row "h" open_; row "h" cons] in
  match LinearityLedger.timelines rows with
  | [t] ->
     assert_equal "h" t.t_var;
     assert_equal "entry" (LinearityLedger.path_label t.t_path);
     assert_equal [open_; cons] t.t_events
  | other -> assert_failure (Printf.sprintf "expected one timeline, got %d" (List.length other))

let test_arms_are_separate_paths _ =
  (* One `if` (branch 0), same variable consumed in both arms. Each arm is a
     complete execution path, so each gets its own balanced timeline. *)
  let rows =
    [ row "h" open_;
      row ~path:[(0, 0)] "h" cons;
      row ~path:[(0, 1)] "h" cons ]
  in
  match LinearityLedger.timelines rows with
  | [a; b] ->
     assert_equal "b0.a0" (LinearityLedger.path_label a.t_path);
     assert_equal [open_; cons] a.t_events;
     assert_equal "b0.a1" (LinearityLedger.path_label b.t_path);
     assert_equal [open_; cons] b.t_events
  | other ->
     assert_failure
       (Printf.sprintf "expected two arm timelines, got %d" (List.length other))

let test_scope_prefix_is_inherited _ =
  (* Events before the `if` are on both paths; events inside one arm are not
     on the other. *)
  let rows =
    [ row "h" open_;
      row ~path:[(0, 0)] "h" LinearityLedger.BorrowRead;
      row ~path:[(0, 1)] "h" cons;
      row ~path:[(0, 0)] "h" LinearityLedger.Unborrow;
      row ~path:[(0, 0)] "h" cons ]
  in
  match LinearityLedger.timelines rows with
  | [a; b] ->
     assert_equal [open_; LinearityLedger.BorrowRead; LinearityLedger.Unborrow; cons] a.t_events;
     assert_equal [open_; cons] b.t_events
  | other ->
     assert_failure (Printf.sprintf "expected two timelines, got %d" (List.length other))

let test_only_maximal_paths _ =
  (* `a` never enters the branch: its rows sit on the entry path only. The
     entry path is a strict prefix of the arms, so it is not maximal and no
     timeline is emitted for it — otherwise `a` would look unconsumed. *)
  let rows =
    [ row "a" open_;
      row "a" cons;
      row "h" open_;
      row ~path:[(0, 0)] "h" cons;
      row ~path:[(0, 1)] "h" cons ]
  in
  let timelines = LinearityLedger.timelines rows in
  let labels =
    List.map (fun t -> (t.t_var, LinearityLedger.path_label t.t_path)) timelines in
  (* Path-major then first-appearance, the order {!timelines} documents. *)
  assert_equal [("a", "b0.a0"); ("h", "b0.a0"); ("a", "b0.a1"); ("h", "b0.a1")] labels

let test_nested_branch_keeps_outer_prefix _ =
  (* Branch 1 nested inside arm 0 of branch 0: a row on [(0,0)] is on both
     nested paths, a row on [(0,1)] is on neither. *)
  let rows =
    [ row "h" open_;
      row ~path:[(1, 0); (0, 0)] "h" cons;
      row ~path:[(1, 1); (0, 0)] "h" cons ]
  in
  match LinearityLedger.timelines rows with
  | [a; b] ->
     assert_equal "b0.a0/b1.a0" (LinearityLedger.path_label a.t_path);
     assert_equal [open_; cons] a.t_events;
     assert_equal "b0.a0/b1.a1" (LinearityLedger.path_label b.t_path);
     assert_equal [open_; cons] b.t_events
  | other ->
     assert_failure (Printf.sprintf "expected two nested timelines, got %d" (List.length other))

let test_json_shape _ =
  let rows =
    [ row "h" open_;
      row ~path:[(3, 0)] "h" cons;
      row ~path:[(3, 1)] "h" cons ]
  in
  let json = LinearityLedger.to_json_string ~name:"Test" ~functions:[("main", "function", rows)] in
  let parsed = Yojson.Safe.from_string json in
  let open Yojson.Safe.Util in
  let module_ = parsed |> member "modules" |> to_list |> List.hd in
  assert_equal "Test" (module_ |> member "name" |> to_string);
  let fn = module_ |> member "functions" |> to_list |> List.hd in
  assert_equal "main" (fn |> member "name" |> to_string);
  assert_equal "function" (fn |> member "kind" |> to_string);
  let timelines = fn |> member "timelines" |> to_list in
  assert_equal 2 (List.length timelines);
  let arm = List.nth timelines 1 in
  assert_equal "b3.a1" (arm |> member "path" |> to_string);
  assert_equal ["acquire"; "consume"]
    (arm |> member "events" |> to_list |> List.map to_string)

let test_borrow_depth_counts_nesting _ =
  (* Borrow depth is *checker* state (LinearityCheck): it decides both what a
     nested scope restores and when `Unborrow` reaches the ledger. Two nested
     borrows of one variable: only the outermost end owns the value again. *)
  let v = Identifier.make_ident in
  LinearityCheck.reset_borrow_depth ();
  LinearityCheck.borrow_push (v "h");
  LinearityCheck.borrow_push (v "h");
  assert_equal false (LinearityCheck.borrow_pop (v "h"))
    ~msg:"a nested borrow's end must not release the value";
  assert_equal true (LinearityCheck.borrow_pop (v "h"))
    ~msg:"the outermost borrow's end releases the value";
  assert_equal true (LinearityCheck.borrow_pop (v "h"))
    ~msg:"an unbalanced pop still reports outermost, so an anomaly is judged not hidden";
  (* Independent variables are independent counters. *)
  LinearityCheck.borrow_push (v "a");
  LinearityCheck.borrow_push (v "a");
  LinearityCheck.borrow_push (v "b");
  assert_equal false (LinearityCheck.borrow_pop (v "a"));
  assert_equal true (LinearityCheck.borrow_pop (v "b"))
    ~msg:"a single-level borrow on another variable ends immediately";
  assert_equal true (LinearityCheck.borrow_pop (v "a"));
  (* Reset leaves nothing behind: the next function starts at zero. *)
  LinearityCheck.borrow_push (v "c");
  LinearityCheck.reset_borrow_depth ();
  assert_equal true (LinearityCheck.borrow_pop (v "c"))
    ~msg:"reset drops counts, so functions cannot inherit each other's nesting"

let test_event_tokens_are_the_wire_format _ =
  (* scripts/ledger2mm0 reads these tokens; lib/LinearityLedger.event_token
     writes them. Renaming one without the other must fail here rather than
     surface as an "unknown event" from the emitter. *)
  let expected =
    [ (LinearityLedger.Acquire, "acquire");
      (LinearityLedger.Consume, "consume");
      (LinearityLedger.Reacquire, "reacquire");
      (LinearityLedger.BorrowRead, "borrow_read");
      (LinearityLedger.BorrowWrite, "borrow_write");
      (LinearityLedger.Reborrow, "reborrow");
      (LinearityLedger.Unborrow, "unborrow");
      (LinearityLedger.Drop, "drop") ]
  in
  List.iter
    (fun (event, token) -> assert_equal token (LinearityLedger.event_token event))
    expected

let test_event_rules_are_the_theory_constructors _ =
  (* docs/memsafe.mm0 declares these; a mismatch would make every proof
     reference an undeclared name. *)
  let expected =
    [ (LinearityLedger.Acquire, "e_acq");
      (LinearityLedger.Consume, "e_cons");
      (LinearityLedger.Reacquire, "e_reacq");
      (LinearityLedger.BorrowRead, "e_bor");
      (LinearityLedger.BorrowWrite, "e_bw");
      (LinearityLedger.Reborrow, "e_reb");
      (LinearityLedger.Unborrow, "e_unbor");
      (LinearityLedger.Drop, "e_drop") ]
  in
  List.iter
    (fun (event, rule) -> assert_equal rule (LinearityLedger.event_rule event))
    expected

let suite =
  "LinearityLedger" >::: [
      "path label" >:: test_path_label;
      "single path timeline" >:: test_single_path_timeline;
      "arms are separate paths" >:: test_arms_are_separate_paths;
      "scope prefix is inherited" >:: test_scope_prefix_is_inherited;
      "only maximal paths" >:: test_only_maximal_paths;
      "nested branch keeps outer prefix" >:: test_nested_branch_keeps_outer_prefix;
      "json shape" >:: test_json_shape;
      "event tokens are the wire format" >:: test_event_tokens_are_the_wire_format;
      "borrow depth counts nesting" >:: test_borrow_depth_counts_nesting;
      "event rules are the theory constructors" >:: test_event_rules_are_the_theory_constructors;
    ]

let () =
  run_test_tt_main suite
