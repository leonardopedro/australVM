(*
   A record field read must lower to exactly one load.

   The typing pass types `p.lo` as a dereference of the slot accessor, so the
   C renderer may keep `*(&(p.lo))`. In the CPS IR the accessor is
   `__slot_get`, which already loads the field, so an enclosing `Deref` loads
   the field's *value* as an address — a segfault in the JIT for any field that
   is not a pointer (the end-to-end `WidgetHandle` run crashed this way right
   after `__new` minted its pair).

   These tests pin the emitted CPS for real Austral source: one `__slot_get`
   per field read, and no `Deref` at all. A lowering that reintroduced the
   redundant dereference would show up as `*(__slot_get(...))`, and a test that
   counted nothing would be caught by the two-read fixture.

   The *offset* is pinned as well, for the reads through a borrow. The typer
   types the accessor's base as the borrow temp's type, and a re-borrow nests
   (`read(hot: &[Hot, R])` sees `&&Hot`), so an unwrap that stops after one
   layer leaves a reference, which is not a `MonoNamedType`: the offset lookup
   is skipped and falls back to 0, and `hot->hi` silently reads `lo`'s word.
   A count-only assertion cannot see that: the call is still there, at the
   wrong offset. Hence the offset fixtures.
*)

open OUnit2

let source_module functions =
  let aum =
    "module body Fields is\n"
    ^ "    record Pair: Free is\n"
    ^ "        lo: Int64;\n"
    ^ "        hi: Int64;\n"
    ^ "    end;\n\n"
    ^ String.concat "\n" (List.map (fun body -> "    " ^ body) functions)
    ^ "\nend module body.\n"
  in
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.BodyModuleSource { body_filename = "Fields.aum"; body_code = aum }
  in
  match Compiler.prepare_module env source with
  | (_, p) -> (p.Compiler.prepared_env, p.Compiler.prepared_mono)
  | exception Error.Austral_error e ->
     assert_failure
       ("field-read fixture failed the frontend:\n"
        ^ Error.render_error_to_plain e)

(* Every node, counted rather than short-circuited: a function with two field
   reads must report two, so the count is not a constant that happens to match
   the one-read case. *)
let rec counts_expr (e: CpsGen.cps_expr) : int * int =
  let add (g1, d1) (g2, d2) = (g1 + g2, d1 + d2) in
  match e with
  | CpsGen.IntLit _ | CpsGen.BoolLit _ | CpsGen.FloatLit _
  | CpsGen.FloatLit32 _ | CpsGen.StringLit _ | CpsGen.Var _ -> (0, 0)
  | CpsGen.App ("__slot_get", args) ->
     List.fold_left (fun acc a -> add acc (counts_expr a)) (1, 0) args
  | CpsGen.App (_, args) ->
     List.fold_left (fun acc a -> add acc (counts_expr a)) (0, 0) args
  | CpsGen.Deref inner ->
     let (g, d) = counts_expr inner in
     (g, d + 1)
  | CpsGen.CmpLt (a, b) | CpsGen.CmpGt (a, b) | CpsGen.CmpLte (a, b)
  | CpsGen.CmpGte (a, b) | CpsGen.CmpEq (a, b) | CpsGen.CmpNeq (a, b)
  | CpsGen.And (a, b) | CpsGen.Or (a, b) | CpsGen.Add (a, b)
  | CpsGen.Sub (a, b) | CpsGen.Mul (a, b) -> add (counts_expr a) (counts_expr b)
  | CpsGen.Not inner -> counts_expr inner

let rec counts_stmt (s: CpsGen.cps_stmt) : int * int =
  let add (g1, d1) (g2, d2) = (g1 + g2, d1 + d2) in
  match s with
  | CpsGen.Skip -> (0, 0)
  | CpsGen.Let (_, e, body) -> add (counts_expr e) (counts_stmt body)
  | CpsGen.Assign (_, e) | CpsGen.Discard e | CpsGen.Return e -> counts_expr e
  | CpsGen.Store (a, b) -> add (counts_expr a) (counts_expr b)
  | CpsGen.If (c, a, b) ->
     add (counts_expr c) (add (counts_stmt a) (counts_stmt b))
  | CpsGen.While (c, body) -> add (counts_expr c) (counts_stmt body)
  | CpsGen.Match (e, cases, default) ->
     List.fold_left
       (fun acc (_, body) -> add acc (counts_stmt body))
       (add (counts_expr e) (counts_stmt default)) cases
  | CpsGen.Block (a, b) -> add (counts_stmt a) (counts_stmt b)

(* The offsets a function's field reads actually use. `find_record_slot_offset`
   falls back to 0 on a miss, so a count-only assertion cannot see a read of the
   wrong slot: a two-field record read through a borrow would still emit two
   loads, both at offset 0. Collecting the literals is what makes the offset a
   measured value. *)
let rec offsets_expr (e: CpsGen.cps_expr) : int64 list =
  let concat xs = List.concat xs in
  match e with
  | CpsGen.App (("__slot_get" | "__ptr_slot_get"), [ base; CpsGen.IntLit off ]) ->
     off :: offsets_expr base
  | CpsGen.IntLit _ | CpsGen.BoolLit _ | CpsGen.FloatLit _
  | CpsGen.FloatLit32 _ | CpsGen.StringLit _ | CpsGen.Var _ -> []
  | CpsGen.App (_, args) -> concat (List.map offsets_expr args)
  | CpsGen.Deref inner | CpsGen.Not inner -> offsets_expr inner
  | CpsGen.CmpLt (a, b) | CpsGen.CmpGt (a, b) | CpsGen.CmpLte (a, b)
  | CpsGen.CmpGte (a, b) | CpsGen.CmpEq (a, b) | CpsGen.CmpNeq (a, b)
  | CpsGen.And (a, b) | CpsGen.Or (a, b) | CpsGen.Add (a, b)
  | CpsGen.Sub (a, b) | CpsGen.Mul (a, b) -> offsets_expr a @ offsets_expr b

let rec offsets_stmt (s: CpsGen.cps_stmt) : int64 list =
  match s with
  | CpsGen.Skip -> []
  | CpsGen.Let (_, e, body) -> offsets_expr e @ offsets_stmt body
  | CpsGen.Assign (_, e) | CpsGen.Discard e | CpsGen.Return e -> offsets_expr e
  | CpsGen.Store (a, b) -> offsets_expr a @ offsets_expr b
  | CpsGen.If (c, a, b) -> offsets_expr c @ offsets_stmt a @ offsets_stmt b
  | CpsGen.While (c, body) -> offsets_expr c @ offsets_stmt body
  | CpsGen.Match (e, cases, default) ->
     offsets_expr e
     @ List.concat (List.map (fun (_, body) -> offsets_stmt body) cases)
     @ offsets_stmt default
  | CpsGen.Block (a, b) -> offsets_stmt a @ offsets_stmt b

(* Exactly one CPS function under `prefix`, so an assertion names a single
   lowering: two matches would otherwise let a fixture pass by inspecting the
   wrong instance. *)
let instance ~prefix functions =
  let (env, mono) = source_module functions in
  ignore env;
  let (funcs, _) = Compiler_cps.compile_module_cps mono in
  let matches =
    List.filter
      (fun (f: CpsGen.function_def) ->
        let name = f.CpsGen.name in
        String.length name >= String.length prefix
        && String.sub name 0 (String.length prefix) = prefix)
      funcs
  in
  match matches with
  | [ f ] -> f
  | [] ->
     assert_failure
       ("no CPS function whose name starts with " ^ prefix ^ "; have: "
        ^ String.concat ", " (List.map (fun (f: CpsGen.function_def) -> f.CpsGen.name) funcs))
  | _ ->
     assert_failure
       (Printf.sprintf "%d CPS functions start with %s; the fixture is ambiguous"
          (List.length matches) prefix)

let lowered_offsets ~prefix functions =
  List.sort compare (offsets_stmt (instance ~prefix functions).CpsGen.body)

(* The same offsets in emission order rather than sorted. `Sub` emits its left
   operand first, so `p->hi - p->lo` must read 8 and then 0: the sorted fixture
   cannot tell that from a lowering that gave the two fields each other's
   offset, which is exactly the bug a borrow's unwrap caused. *)
let lowered_offsets_in_order ~prefix functions =
  offsets_stmt (instance ~prefix functions).CpsGen.body

let assert_offsets ~prefix ~offsets functions =
  let actual = lowered_offsets ~prefix functions in
  if actual <> offsets then
    assert_failure
      (Printf.sprintf
         "%s reads field offset(s) [%s], expected [%s] — a borrow's pointee \
          type is what `find_record_slot_offset` must be given, and its miss \
          path is offset 0"
         prefix
         (String.concat "; " (List.map Int64.to_string actual))
         (String.concat "; " (List.map Int64.to_string offsets)))

let assert_offsets_in_order ~prefix ~offsets functions =
  let actual = lowered_offsets_in_order ~prefix functions in
  if actual <> offsets then
    assert_failure
      (Printf.sprintf
         "%s reads field offset(s) [%s] in emission order, expected [%s] — \
          the order is the expression's, so a field given the other's offset \
          is visible here even when the set of offsets is right"
         prefix
         (String.concat "; " (List.map Int64.to_string actual))
         (String.concat "; " (List.map Int64.to_string offsets)))

let lowered_counts ~entry functions =
  let (env, mono) = source_module functions in
  ignore env;
  let (funcs, _) = Compiler_cps.compile_module_cps mono in
  let fn =
    match List.find_opt (fun (f: CpsGen.function_def) -> f.CpsGen.name = entry) funcs with
    | Some f -> f
    | None -> assert_failure ("no CPS function named " ^ entry)
  in
  counts_stmt fn.CpsGen.body

let assert_field_reads ~entry ~reads functions =
  let (slot_gets, derefs) = lowered_counts ~entry functions in
  if slot_gets <> reads then
    assert_failure
      (Printf.sprintf
         "%s lowers %d field read(s) to %d `__slot_get` call(s), expected %d"
         entry reads slot_gets reads);
  if derefs <> 0 then
    assert_failure
      (Printf.sprintf
         "%s lowers a field read through %d extra `Deref` node(s): \
          `__slot_get` already loads, so the enclosing dereference loads the \
          field's value as an address (a JIT segfault)"
         entry derefs)

let test_one_field_read_is_one_load _ =
  assert_field_reads ~entry:"readLo"
    ~reads:1
    [ "function readLo(p: Pair): Int64 is\n\
      \    return p.lo;\n\
       end;" ]

let test_two_field_reads_are_two_loads _ =
  assert_field_reads ~entry:"sum"
    ~reads:2
    [ "function sum(p: Pair): Int64 is\n\
      \    return p.hi - p.lo;\n\
       end;" ]

(* A read through a borrowed record takes the same path (the accessor's base is
   a `Var` either way); this keeps the fixture honest about the shape the
   `WidgetHandle` module uses. *)
let test_local_record_field_read_is_one_load _ =
  assert_field_reads ~entry:"readLocal"
    ~reads:1
    [ "function readLocal(): Int64 is\n\
      \    let p: Pair := Pair(lo => 7, hi => 9);\n\
      \    return p.lo;\n\
       end;" ]

(* A borrow of a record inside a generically-region-polymorphic function is
   the shape `HotHandle.read(&[Hot, R])` uses. The function is monomorphised
   before the CPS pass, so the fixture calls it: an unreferenced generic has no
   CPS function to inspect. *)
(* A monomorphised instance is named `monomorph_(Id.MonoId n)`, not after the
   source function, so the prefix is the pass's, and the fixture keeps exactly
   one such instance — a second one would make the assertion ambiguous rather
   than silently pick a function. *)
let test_generic_borrow_reads_the_second_field_at_its_own_offset _ =
  assert_offsets ~prefix:"monomorph_" ~offsets:[ 8L ]
    [ "generic [R: Region]\n\
      \    function readHi(p: &[Pair, R]): Int64 is\n\
      \        return p->hi;\n\
      \    end;";
      "function caller(): Int64 is\n\
      \    let p: Pair := Pair(lo => 1, hi => 2);\n\
      \    return readHi(&p);\n\
       end;" ]

let test_generic_borrow_reads_both_fields_at_their_own_offsets _ =
  assert_offsets ~prefix:"monomorph_" ~offsets:[ 0L; 8L ]
    [ "generic [R: Region]\n\
      \    function readBoth(p: &[Pair, R]): Int64 is\n\
      \        return p->hi - p->lo;\n\
      \    end;";
      "function caller(): Int64 is\n\
      \    let p: Pair := Pair(lo => 1, hi => 2);\n\
      \    return readBoth(&p);\n\
       end;" ]

(* Both fields in one expression, in the order they are read. This is the
   end-to-end shape of the offset-0 bug: `p->hi - p->lo` through a nested
   borrow lowered to `0 - 0`, and the sorted fixture above would have accepted
   `8; 0` and `0; 8` alike. *)
let test_generic_borrow_two_fields_keep_their_offsets _ =
  assert_offsets_in_order ~prefix:"monomorph_" ~offsets:[ 8L; 0L ]
    [ "generic [R: Region]\n\
      \    function readSpread(p: &[Pair, R]): Int64 is\n\
      \        return p->hi - p->lo;\n\
      \    end;";
      "function caller(): Int64 is\n\
      \    let p: Pair := Pair(lo => 1, hi => 2);\n\
      \    return readSpread(&p);\n\
       end;" ]

let suite =
  "CpsFieldRead" >::: [
      "a field read emits one load" >:: test_one_field_read_is_one_load;
      "two field reads emit two loads" >:: test_two_field_reads_are_two_loads;
      "a local record's field read emits one load"
      >:: test_local_record_field_read_is_one_load;
      "a generic borrow reads the second field at offset 8"
      >:: test_generic_borrow_reads_the_second_field_at_its_own_offset;
      "a generic borrow reads both fields at their own offsets"
      >:: test_generic_borrow_reads_both_fields_at_their_own_offsets;
      "a generic borrow keeps each field's offset in one expression"
      >:: test_generic_borrow_two_fields_keep_their_offsets;
    ]

let () = run_test_tt_main suite
