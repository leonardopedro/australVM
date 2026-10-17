(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(* avm-pkg spec task 2.1 (docs/pkg_manager_plan.md §3.1, second half):
   the JSON derivation export. Package modules declare their derivations
   as top-level constants of type `Derivation` (stdlib/pkg/Derivation.au):

       constant hello: Derivation :=
           Derivation(
               name => "hello",
               version => "1.0.0",
               builder_symbol => "stdenv.build",
               dependencies => Cons(..., Nil()),
               sources => Nil(),
               env_vars => Nil());

   This pass walks the typed module, recognizes every `TConstant` whose
   declared type is `Derivation`, structurally decodes the constant
   initializer (string fields, `List`-typed `Cons`/`Nil` chains, and
   `Pair` records for env vars), and emits the exact `plan.json` shape
   `avm-eval`'s `Derivation` deserializes:

       {"derivations":[{"name":..,"version":..,"builder_symbol":..,
                        "dependencies":[..],"sources":[..],
                        "env_vars":[[k,v],..]}]}

   Output is deterministic: derivations are sorted by name, and a module
   compiled after another merges into an existing file (later declarations
   of the same name win — including in the `totality` status) so a
   multi-module build can share one plan.

   Totality status (release contract, point 2: all non-selected info
   leaks to a deterministic plan.json). Each derivation records whether
   its module was compiled under `--pkg-totality=true`, the flag that
   selects declarations into the non-recursive vetted set:

       "totality": "proven"    <- compiled under --pkg-totality=true
                                  (the flag-gated pass rejected nothing,
                                  so every declaration it saw is total)
       "totality": "unproven"  <- not selected: the pass did not run,
                                  so totality is unknown, not assumed

   The status is a pure function of the invocation and the module, so it
   replays deterministically; v0 plan files without the field decode as
   `unproven`, never as proven. An unknown status string fails loudly.

   Anything that does not fit the shape fails the compilation loudly
   (`Error.err`) — a derivation the exporter cannot faithfully represent
   must never be silently dropped from the plan. *)

open Identifier
open Type
open Stages.Tast
open Escape
open Error

type derivation = {
    name: string;
    version: string;
    builder_symbol: string;
    dependencies: string list;
    sources: string list;
    env_vars: (string * string) list;
    (** True iff the module was compiled under `--pkg-totality=true`
        (the flag-gated totality pass ran and accepted it). *)
    totalitity_status: bool;
  }

(* Is this the `Derivation` record type (by name; no type arguments)? *)
let is_derivation_type (ty: ty): bool =
  match ty with
  | Type.NamedType (qident, [], _) ->
     (ident_string (original_name qident)) = "Derivation"
  | _ ->
     false

let as_string (what: string) (e: texpr): string =
  match e with
  | TStringConstant s -> escaped_to_string s
  | _ ->
     err ("Derivation field `" ^ what ^ "` must be a string constant")

(* Decode a `List`-shaped Cons/Nil chain into its elements. *)
let rec as_list (what: string) (e: texpr): texpr list =
  match e with
  | TUnionConstructor (_, case, _) when (ident_string case) = "Nil" ->
     []
  | TUnionConstructor (_, case, fields) when (ident_string case) = "Cons" ->
     let slot n =
       match List.assoc_opt (make_ident n) fields with
       | Some v -> v
       | None -> err ("Derivation list `" ^ what ^ "`: Cons cell is missing its `" ^ n ^ "` field")
     in
     (slot "head") :: (as_list what (slot "tail"))
  | _ ->
     err ("Derivation list field `" ^ what ^ "` must be a List built from Cons/Nil constructors")

(* A `Pair` record: two string slots, taken positionally. *)
let as_pair (e: texpr): string * string =
  match e with
  | TRecordConstructor (_, [(_, a); (_, b)]) ->
     (as_string "env_vars key" a, as_string "env_vars value" b)
  | _ ->
     err "Derivation env_vars entries must be Pair records with exactly two string fields"

let derivation_of_initializer (cname: identifier) (e: texpr): derivation =
  match e with
  | TRecordConstructor (_, fields) ->
     let slot n =
       match List.assoc_opt (make_ident n) fields with
       | Some v -> v
       | None ->
          err ("Derivation constant `" ^ ident_string cname ^ "` is missing its `" ^ n ^ "` field")
     in
     {
       name = as_string "name" (slot "name");
       version = as_string "version" (slot "version");
       builder_symbol = as_string "builder_symbol" (slot "builder_symbol");
       dependencies =
         List.map (as_string "dependencies")
           (as_list "dependencies" (slot "dependencies"));
       sources =
         List.map (as_string "sources")
           (as_list "sources" (slot "sources"));
       env_vars =
         List.map as_pair (as_list "env_vars" (slot "env_vars"));
       (* Provenance is an invocation property, not a declaration
          property: `module_derivations` overwrites this with the
          caller's `~totality` flag. Default to unproven so a caller
          forgetting to say can never read as proven. *)
       totalitity_status = false;
     }
  | _ ->
     err ("Derivation constant `" ^ ident_string cname
          ^ "` must be a record constructor literal")

(** Extract every derivation declared by this module, sorted by name.
    [~totality:true] (pass `true` exactly when `!Compiler.pkg_totality`
    held for this invocation) marks them `proven`; the default marks them
    `unproven`. *)
let module_derivations ?(totality: bool = false) (m: typed_module): derivation list =
  let (TypedModule (_, decls)) = m in
  List.filter_map
    (fun decl ->
      match decl with
      | TConstant (_, _, cname, ty, expr, _) when is_derivation_type ty ->
         Some (derivation_of_initializer cname expr |> fun d -> { d with totalitity_status = totality })
      | _ ->
         None)
    decls
  |> List.sort (fun a b -> String.compare a.name b.name)

(* ── JSON emission ─────────────────────────────────────────────────── *)

let to_yojson (ds: derivation list): Yojson.Safe.t =
  let string_list xs = `List (List.map (fun s -> `String s) xs) in
  `List
    (List.map
       (fun d ->
         `Assoc [
                  ("name", `String d.name);
                  ("version", `String d.version);
                  ("builder_symbol", `String d.builder_symbol);
                  ("dependencies", string_list d.dependencies);
                  ("sources", string_list d.sources);
                  ("env_vars",
                   `List (List.map (fun (k, v) -> `List [`String k; `String v]) d.env_vars));
                  ("totality",
                   `String (if d.totalitity_status then "proven" else "unproven"));
                ])
       ds)

let to_json (ds: derivation list): string =
  Yojson.Safe.pretty_to_string (`Assoc [("derivations", to_yojson ds)])

(** Decode one derivation from JSON (for merging with an existing plan).
    A missing `totality` field (a v0 plan file) decodes as `unproven`:
    absence of the record must never read as absence of obligation. *)
let derivation_of_yojson (j: Yojson.Safe.t): derivation =
  let open Yojson.Safe.Util in
  let strings key = j |> member key |> to_list |> List.map to_string in
  let totality =
    match member "totality" j with
    | `String "proven" -> true
    | `String "unproven" -> false
    | `Null -> false (* v0 plan file: totality was never recorded *)
    | `String s -> err ("plan.json: unknown totality status `" ^ s ^ "` (expected `proven` or `unproven`)")
    | _ -> err "plan.json: totality status must be the string `proven` or `unproven`"
  in
  {
    name = j |> member "name" |> to_string;
    version = j |> member "version" |> to_string;
    builder_symbol = j |> member "builder_symbol" |> to_string;
    dependencies = strings "dependencies";
    sources = strings "sources";
    env_vars =
      j |> member "env_vars" |> to_list
      |> List.map
           (fun p ->
             match p with
             | `List [`String k; `String v] -> (k, v)
             | _ -> err "plan.json: env_vars entries must be [key, value] string pairs");
    totalitity_status = totality;
  }

(** Write `path` as a plan containing exactly the derivations in `ds`,
    merged with whatever the file already holds (new declarations of the
    same name win, including their totality status; the result is sorted
    by name). *)
let write_json (path: string) (ds: derivation list): unit =
  let existing: derivation list =
    if Sys.file_exists path then
      (try
         let json = Yojson.Safe.from_file path in
         match Yojson.Safe.Util.member "derivations" json with
         | `List items -> List.map derivation_of_yojson items
         | _ -> []
       with
       | Yojson.Json_error _ -> []
       | Yojson.Safe.Util.Type_error (_, _) -> [])
    else
      []
  in
  let merged =
    List.fold_left
      (fun acc d -> d :: List.filter (fun o -> o.name <> d.name) acc)
      existing
      ds
  in
  let merged = List.sort (fun a b -> String.compare a.name b.name) merged in
  let oc = open_out path in
  output_string oc (to_json merged);
  close_out oc
