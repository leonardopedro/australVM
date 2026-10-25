(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(* The linearity ledger (docs/memsafe.md §3).

   LinearityCheck already refuses illegal programs. What it cannot give an
   auditor is a *handful of facts* a frozen verifier can re-check: its decision
   procedure is 700 lines of OCaml inside the compiler, which is the untrusted
   party in the memsafe design. So every point where the checker commits to a
   decision - "this value is acquired", "this is the one consumption", "this
   borrow starts/ends" - also records a row here.

   Two properties make the trace checkable rather than decorative:

   - Rows are tagged with the branch path they were recorded on. The checker
     walks *both* arms of an `if` from the same table, so a flat event stream
     would show a variable consumed twice; per-path timelines do not. Resolving
     rows to one timeline per maximal path is what turns the tree the checker
     really walks into the lines an automaton can accept.
   - Only linear-ish types are recorded, so the timelines quantify over exactly
     the resources whose conservation is the claim.

   Recording is off unless `--emit-linearity-ledger` was passed, so ordinary
   compilation does no work here and cannot be affected by a bug in this
   module. *)

type event =
  | Acquire
  | Consume
  | Reacquire
  | BorrowRead
  | BorrowWrite
  | Reborrow
  | Unborrow
  | Drop

let event_token (e: event): string =
  match e with
  | Acquire -> "acquire"
  | Consume -> "consume"
  | Reacquire -> "reacquire"
  | BorrowRead -> "borrow_read"
  | BorrowWrite -> "borrow_write"
  | Reborrow -> "reborrow"
  | Unborrow -> "unborrow"
  | Drop -> "drop"

(* The constructor names of docs/memsafe.mm0. Kept here so the emitter never
   invents a name the theory does not declare (scripts/verify-invariants
   cross-checks the artifact against the theory, same as it does for the
   Lean-side certificates). *)
let event_rule (e: event): string =
  match e with
  | Acquire -> "e_acq"
  | Consume -> "e_cons"
  | Reacquire -> "e_reacq"
  | BorrowRead -> "e_bor"
  | BorrowWrite -> "e_bw"
  | Reborrow -> "e_reb"
  | Unborrow -> "e_unbor"
  | Drop -> "e_drop"

type row = {
    var: string;
    ty: string;
    depth: int;
    event: event;
    path: (int * int) list;  (* innermost branch first: [(id, arm), ...] *)
  }

type timeline = {
    t_var: string;
    t_ty: string;
    t_path: (int * int) list;
    t_events: event list;
  }

(* ── recording state ───────────────────────────────────────────────────── *)

let enabled = ref false
let path: (int * int) list ref = ref []
let branch_ids = ref 0
let current: (string * string * row list ref) option ref = ref None
(* Completed functions, most recent first. Reversed on export. *)
let completed: (string * string * row list) list ref = ref []

let set_enabled (b: bool): unit =
  enabled := b;
  if not b then begin
    path := [];
    current := None;
    completed := []
  end

let is_enabled (): bool = !enabled

let clear (): unit =
  path := [];
  branch_ids := 0;
  current := None;
  completed := []

let begin_function ~(name: string) ~(kind: string): unit =
  if not !enabled then () else
  (match !current with
   | Some (n, k, rows) -> completed := (n, k, List.rev !rows) :: !completed
   | None -> ());
  path := [];
  current := Some (name, kind, ref [])

let end_function (): unit =
  if not !enabled then () else
  match !current with
  | Some (n, k, rows) ->
     completed := (n, k, List.rev !rows) :: !completed;
     current := None
  | None -> ()

let next_branch (): int =
  let id = !branch_ids in
  branch_ids := id + 1;
  id

let with_arm (id: int) (arm: int) (f: unit -> 'a): 'a =
  if not !enabled then f ()
  else begin
    let saved = !path in
    path := (id, arm) :: saved;
    Fun.protect ~finally:(fun () -> path := saved) f
  end

let record ~(var: string) ~(ty: string) ~(depth: int) (event: event): unit =
  if not !enabled then () else
  match !current with
  | Some (_, _, rows) ->
     rows := { var; ty; depth; event; path = !path } :: !rows
  | None -> ()

(* ── path arithmetic ───────────────────────────────────────────────────── *)

(* `path` lists are innermost-first, so scope comparison (an event is on an
   execution path when its branch path is an outer prefix of it) works on the
   reversed lists. *)
let scope (outer: (int * int) list) (inner: (int * int) list): bool =
  let rec go a b =
    match a, b with
    | [], _ -> true
    | _ :: _, [] -> false
    | x :: a', y :: b' -> x = y && go a' b'
  in
  go (List.rev outer) (List.rev inner)

let path_label (p: (int * int) list): string =
  match p with
  | [] -> "entry"
  | _ ->
     String.concat "/"
       (List.rev_map (fun (id, arm) -> Printf.sprintf "b%d.a%d" id arm) p)

(* ── resolution ────────────────────────────────────────────────────────── *)

let timelines (rows: row list): timeline list =
  let add_unique (p: (int * int) list) (acc: (int * int) list list) =
    if List.exists (fun q -> q = p) acc then acc else acc @ [p]
  in
  let paths = List.fold_left (fun acc r -> add_unique r.path acc) [] rows in
  let maximal =
    List.filter
      (fun p -> not (List.exists (fun q -> q <> p && scope p q) paths))
      paths
  in
  let maximal =
    List.sort (fun a b -> String.compare (path_label a) (path_label b)) maximal
  in
  (* Variable order: first appearance in the function, so a timeline list is
     diffable across runs. *)
  let vars =
    List.fold_left
      (fun acc r -> if List.mem r.var acc then acc else acc @ [r.var])
      [] rows
  in
  List.concat_map
    (fun p ->
      List.filter_map
        (fun v ->
          let scoped =
            List.filter (fun r -> r.var = v && scope r.path p) rows
          in
          match scoped with
          | [] -> None
          | first :: _ ->
             Some {
                 t_var = v;
                 t_ty = first.ty;
                 t_path = p;
                 t_events = List.map (fun r -> r.event) scoped;
               })
        vars)
    maximal

(* ── JSON export ───────────────────────────────────────────────────────── *)

let timeline_json (t: timeline): Yojson.Safe.t =
  `Assoc [
      ("var", `String t.t_var);
      ("ty", `String t.t_ty);
      ("path", `String (path_label t.t_path));
      ("events", `List (List.map (fun e -> `String (event_token e)) t.t_events));
    ]

let module_json ~(name: string)
    (functions: (string * string * row list) list): Yojson.Safe.t =
  `Assoc [
      ("name", `String name);
      ("functions",
       `List
         (List.map
            (fun (fn, kind, rows) ->
              `Assoc [
                  ("name", `String fn);
                  ("kind", `String kind);
                  ("timelines",
                   `List (List.map timeline_json (timelines rows)));
                ])
            functions));
    ]

let to_json_string ~(name: string)
    ~(functions: (string * string * row list) list): string =
  (* The same wrapper a ledger file has: one module, so a test or self-test
     can compare this string against a written file directly. *)
  Yojson.Safe.pretty_to_string
    (`Assoc [("modules", `List [module_json ~name functions])])

let write_json ~(path: string) ~(name: string): unit =
  let existing: Yojson.Safe.t list =
    if Sys.file_exists path then
      try
        match Yojson.Safe.from_file path with
        | `Assoc fields ->
           (match List.assoc_opt "modules" fields with
            | Some (`List items) -> items
            | _ -> [])
        | _ -> []
      with Yojson.Json_error _ -> []
    else
      []
  in
  let ours = module_json ~name (List.rev !completed) in
  let others =
    List.filter
      (fun j ->
        match Yojson.Safe.Util.member "name" j with
        | `String n -> n <> name
        | _ -> false)
      existing
  in
  let modules =
    List.sort
      (fun a b ->
        let n j =
          match Yojson.Safe.Util.member "name" j with
          | `String s -> s
          | _ -> ""
        in
        String.compare (n a) (n b))
      (ours :: others)
  in
  let oc = open_out path in
  output_string oc (Yojson.Safe.pretty_to_string (`Assoc [("modules", `List modules)]));
  close_out oc
