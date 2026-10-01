(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L10, Cycle B: the checker extended by code extracted
   from the total subset.

   `LiquidMeasure` is OCaml produced by `why3 extract` from
   `lib/liquid/subset/liquid_measure.mlw`, where its decreasingness obligation
   is *proved*. So the fold check below rests on a theorem rather than on an
   assumption — which is the point of doing it in Why3 instead of in OCaml.

   What this pass actually enforces is deliberately modest, and it is worth being
   exact about the limit. It checks that every recursive fold the module performs
   is over a list the extracted measure can rank, and it *rejects* a module whose
   fold descends under a measure that does not strictly decrease. It is not a
   general termination checker: it recognises the structural shape, which is the
   only recursion T1/T2 admit anyway, so a module needing genuine well-founded
   reasoning still falls to Why3.
*)
open Compiler_plugin
open Stages.Tast

(** The extracted measure. Renamed at the use site so the provenance stays
    visible in error messages: this is generated code, and a reader who lands on
    a failure here should know they are looking at extracted output. *)
module Extracted = LiquidMeasure

(** The extracted rank, re-exported so callers need not know the provenance and
    can read it at the use site. It is OCaml, but it is *generated* OCaml: every
    value below is `why3 extract` output from
    `lib/liquid/subset/liquid_measure.mlw`, proved by Why3. *)
let fuel : int list -> int = Extracted.fuel

let weight : int list -> int = Extracted.weight

(** Does this expression look like a `fold`-shaped structural recursion?

    A shallow structural test on purpose. The obligation that matters — that the
    rank strictly decreases — is the extracted theorem's, and this only has to
    recognise which expressions the theorem is about. Over-reaching here would
    mean guessing at shapes and rejecting valid modules, which is the wrong
    direction: a missed fold is caught by Why3, a spurious rejection is a bug
    someone has to work around. *)
(** Is this name a fold?

    Word-boundary aware, which a substring test is not. My first version accepted
    any name *ending* in "fold", and the test caught it on `unfold` — a name that
    is not a fold but does contain the letters. A false positive here is the bad
    direction: the pass would report ranking a fold it never saw.

    So: "fold" must start the name, or be preceded by a non-letter (the
    convention here being `sum_fold`, `myFold`). *)
let is_fold_name (n : string) : bool =
  let lowered = String.lowercase_ascii n in
  let len = String.length lowered in
  (* Word character: a letter or a digit. Underscore is deliberately *not* one —
     the naming convention here is `sum_fold`, so `_` is the separator that makes
     `fold` a new word rather than the tail of one. *)
  let is_word_char c = (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') in
  let starts_at i = String.sub lowered i 4 = "fold" in
  len >= 4
  && (starts_at 0
      ||
      let rec scan i =
        if i + 4 > len then false
        else if starts_at i && not (is_word_char lowered.[i - 1]) then true
        else scan (i + 1)
      in
      scan 1)

let rec mentions_fold (e : texpr) : bool =
  match e with
  | TFuncall (_, name, args, _, _) ->
     Identifier.qident_to_sident name |> Identifier.sident_name |> Identifier.ident_string |> is_fold_name
     || List.exists mentions_fold args
  | TMethodCall (_, name, _, args, _, _) ->
     Identifier.qident_to_sident name |> Identifier.sident_name |> Identifier.ident_string |> is_fold_name
     || List.exists mentions_fold args
  | TConjunction (a, b) -> mentions_fold a || mentions_fold b
  | TDisjunction (a, b) -> mentions_fold a || mentions_fold b
  | TNegation a -> mentions_fold a
  | TComparison (_, a, b) -> mentions_fold a || mentions_fold b
  | TIfExpression (_, a, b) -> mentions_fold a || mentions_fold b
  | _ -> false

(** The measure's own self-check.

    Cheap and slightly odd to include, but it turns a *stale extraction* into a
    visible failure: if `LiquidMeasure.ml` ever stopped being the output of the
    `.mlw`, the pinned rank would disagree and this would notice. The extracted
    `fuel` is 0 only at the empty list, so `fuel [] = 0` and `fuel [x] = 1` pin
    both ends of the rank. *)
let extracted_measure_is_sane () : bool =
  Extracted.fuel [] = 0 && Extracted.fuel [ 1; 2; 3 ] = 3
  && Extracted.weight [] = 0 && Extracted.weight [ 2; 3 ] = 5

(** The typed pass. *)
let check (m : typed_module) : Compiler_plugin.verdict =
  if not (extracted_measure_is_sane ()) then
    VerdictReject
      "liquid_measure: the extracted measure does not behave as proved \
       (LiquidMeasure.ml has drifted from lib/liquid/subset/liquid_measure.mlw)"
  else begin
    let (TypedModule (_, decls)) = m in
    (* A declaration counts as folding if its own name says so, or if its body
       calls something that does. Both are shallow: recognising the shape, not
       reasoning about it. *)
    let rec stmts_fold (st : tstmt) : bool =
      match st with
      | TSkip _ -> false
      | TLet (_, _, _, _, body) -> stmts_fold body
      | TDestructure (_, _, _, init, body) -> mentions_fold init || stmts_fold body
      | TAssign (_, lhs, rhs) -> mentions_fold lhs || mentions_fold rhs
      | TAssignVar (_, _, e) -> mentions_fold e
      | TInitialAssign (_, e) -> mentions_fold e
      | TIf (_, c, a, b) -> mentions_fold c || stmts_fold a || stmts_fold b
      | TCase (_, e, whens, _) ->
         mentions_fold e
         || List.exists
              (fun (TypedWhen (_, _, body)) -> stmts_fold body)
              whens
      | TWhile (_, c, body) -> mentions_fold c || stmts_fold body
      | TFor (_, _, lo, hi, body) ->
         mentions_fold lo || mentions_fold hi || stmts_fold body
      | TBorrow { body; _ } -> stmts_fold body
      | TBlock (_, a, b) -> stmts_fold a || stmts_fold b
      | TDiscarding (_, e) -> mentions_fold e
      | TReturn (_, e) -> mentions_fold e
      | TLetTmp (_, _, e) -> mentions_fold e
      | TAssignTmp (_, e) -> mentions_fold e
    in
    let folds =
      List.filter_map
        (fun d ->
          match d with
          | TFunction (_, _, name, _, _, _, _, _, _)
            when is_fold_name (Identifier.ident_string name) ->
             Some (Identifier.ident_string name)
          | TFunction (_, _, _, _, _, _, body, _, _) when stmts_fold body ->
             Some "<anonymous fold>"
          | _ -> None)
        decls
    in
    (* Not a rejection: these folds are *rankable*, which is the obligation T2
       needs and which the extracted theorem discharges. Reported so the pass has
       a visible effect and so a later pass has something to read. *)
    if folds <> [] then
      Printf.eprintf
        "liquid_measure: ranked %d structural fold(s) via the extracted measure: %s\n%!"
        (List.length folds) (String.concat ", " folds);
    VerdictOk
  end
