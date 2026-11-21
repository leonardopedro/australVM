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

let suite =
  "CpsFieldRead" >::: [
      "a field read emits one load" >:: test_one_field_read_is_one_load;
      "two field reads emit two loads" >:: test_two_field_reads_are_two_loads;
      "a local record's field read emits one load"
      >:: test_local_record_field_read_is_one_load;
    ]

let () = run_test_tt_main suite
