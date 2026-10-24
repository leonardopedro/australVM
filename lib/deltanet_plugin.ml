(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The DeltaNets UNF compiler pass (the Austral → DeltaNets translation as a
   compiler gate). Every top-level `TConstant` with a closed arithmetic
   initializer is serialized to the Austral subset the kernel's
   `uk_austral_unf` symbol accepts, reduced through the kernel's
   interaction-net unique-normal-form reducer (via the OCaml↔Rust bridge),
   and checked against this pass's own independent OCaml evaluator for the
   same expression — by the **conversion check** (`uk_austral_defeq`): the
   initializer and the compiler's evaluated literal must reduce to the same
   net, certificate (isomorphism witness) included. Equal nets ⇒ equal
   values, so this subsumes the plain value comparison, which is kept as the
   fallback when the kernel's defeq answer is unusable. A disagreement means
   the compiler's view of a constant and the kernel's canonical normal form
   diverge — a compiler-consistency failure — and the module is rejected,
   mirroring how the Why3 gate rejects modules importing ungranted symbols.

   This is the "call the kernel from the compiler" direction of the S36
   cycle (see `unfer/docs/WHYML_CYCLE.md`): the kernel supplies the
   unique-normal-form reducer, the compiler enforces agreement with it.

   Opt-in: the pass is a no-op unless `UNFER_DELTANET=1` is set (the JIT
   module-manifest plumbing stands in for the grants env of the Why3 gate).
   When the kernel/bridge is unavailable the pass is also a no-op, so the
   compiler keeps working outside the unfer checkout.
*)

open Compiler_plugin
open Stages.Tast
open Common

let enabled_var = "UNFER_DELTANET"

let enabled () =
  match Sys.getenv_opt enabled_var with
  | Some "1" -> true
  | _ -> false

(* ── serialize a closed typed expression to the Austral subset ─────── *)

let rec to_austral (e : texpr) : string option =
  match e with
  | TIntConstant s -> Some s
  | TFloatConstant s -> Some s
  | TBoolConstant b -> Some (if b then "true" else "false")
  | TConjunction (a, b) -> binop "and" a b
  | TDisjunction (a, b) -> binop "or" a b
  | TNegation a -> (
      match to_austral a with
      | Some s -> Some ("(not " ^ s ^ ")")
      | None -> None)
  | TComparison (op, a, b) -> (
      (* The kernel's Austral subset has no `<>` and no non-strict comparison.
         `logos/src/austral_codegen/parser.rs` rewrites `<=` to `<` and `>=` to
         `>` on the way in, so `3 <= 3` compiled here as `true` and reduced on the
         kernel side as `3 < 3` = `false` — a silent disagreement in exactly the
         place this gate exists to catch. `<>` has no token at all, so
         `uk_austral_unf` returned UK-4804 and a valid module was rejected with a
         message blaming the kernel.

         Only `=`, `<`, `>` survive unchanged, and `not (a = b)` is how the
         kernel spells inequality. *)
      let op_s =
        match op with
        | Equal -> Some "="
        | LessThan -> Some "<"
        | GreaterThan -> Some ">"
        | NotEqual -> None
        | LessThanOrEqual | GreaterThanOrEqual -> None
      in
      match op_s with
      | Some op_s -> binop op_s a b
      | None -> (
          match (op, to_austral a, to_austral b) with
          | NotEqual, Some x, Some y -> Some ("(not (" ^ x ^ " = " ^ y ^ "))")
          | LessThanOrEqual, Some x, Some y ->
              Some ("(not (" ^ y ^ " < " ^ x ^ "))")
          | GreaterThanOrEqual, Some x, Some y ->
              Some ("(not (" ^ x ^ " > " ^ y ^ "))")
          | _ -> None))
  | _ -> None

and binop op a b =
  match (to_austral a, to_austral b) with
  | Some x, Some y -> Some ("(" ^ x ^ " " ^ op ^ " " ^ y ^ ")")
  | _ -> None

(* ── an independent OCaml evaluator for the same subset ─────────────── *)

type value = VInt of int64 | VFloat of float | VBool of bool

let rec eval (e : texpr) : value option =
  match e with
  | TIntConstant s -> ( try Some (VInt (Int64.of_string s)) with _ -> None)
  | TFloatConstant s -> ( try Some (VFloat (float_of_string s)) with _ -> None)
  | TBoolConstant b -> Some (VBool b)
  | TConjunction (a, b) -> bool2 ( && ) a b
  | TDisjunction (a, b) -> bool2 ( || ) a b
  | TNegation a -> (
      match eval a with Some (VBool x) -> Some (VBool (not x)) | _ -> None)
  | TComparison (op, a, b) -> cmp op a b
  | _ -> None

and bool2 f a b =
  match (eval a, eval b) with
  | Some (VBool x), Some (VBool y) -> Some (VBool (f x y))
  | _ -> None

and cmp op a b =
  let cmp_num x y =
    match op with
    | Equal -> Some (VBool (x = y))
    | NotEqual -> Some (VBool (x <> y))
    | LessThan -> Some (VBool (x < y))
    | LessThanOrEqual -> Some (VBool (x <= y))
    | GreaterThan -> Some (VBool (x > y))
    | GreaterThanOrEqual -> Some (VBool (x >= y))
  in
  match (eval a, eval b) with
  | Some (VInt x), Some (VInt y) -> cmp_num (Int64.to_float x) (Int64.to_float y)
  | Some (VFloat x), Some (VFloat y) -> cmp_num x y
  | Some (VInt x), Some (VFloat y) -> cmp_num (Int64.to_float x) y
  | Some (VFloat x), Some (VInt y) -> cmp_num x (Int64.to_float y)
  | Some (VBool x), Some (VBool y) -> (
      match op with
      | Equal -> Some (VBool (x = y))
      | NotEqual -> Some (VBool (x <> y))
      | _ -> None)
  | _ -> None

(* ── compare the kernel's UNF value against the OCaml evaluation ────── *)

(** The literal-source form of an *evaluated* constant: what the compiler
    believes the initializer reduces to, written as source the kernel can
    re-reduce. Round-trip checked against the kernel's Austral subset:
    `(3 > 2)` ≡ `true`, `(1.5 + 2.25)` ≡ `3.75`, `(0 - 5)` ≡ `-5`, and
    `(2.0 + 2.0)` ≡ `4.` all reduce to identical unique normal forms
    (`logos iso`), so a conversion-check failure here is a genuine
    compiler↔kernel inconsistency, not a formatting artifact. *)
let literal_source (v : value) : string =
  match v with
  | VInt i -> Int64.to_string i
  | VBool b -> string_of_bool b
  | VFloat f -> string_of_float f

(** The `equivalent` field of an `AustralDefeqReport` JSON. [None] when the
    report is unparseable or lacks the field — the caller then falls back
    to the plain value comparison rather than trusting a report it cannot
    read. *)
let report_equivalent (json : string) : bool option =
  try
    match Yojson.Safe.Util.member "equivalent" (Yojson.Safe.from_string json) with
    | `Bool b -> Some b
    | _ -> None
  with _ -> None

(** The kernel report's `value` field: `Some s` when the term was closed
    (no unknowns) and numeric, `None` otherwise. *)
let report_value (json : string) : string option =
  try
    let report = Yojson.Safe.from_string json in
    match Yojson.Safe.Util.member "value" report with
    | `String s -> Some s
    | _ -> None
  with _ -> None

let value_to_number (v : value) : float =
  match v with VInt i -> Int64.to_float i | VFloat f -> f | VBool _ -> nan

let kernel_number (s : string) : float =
  match float_of_string_opt s with Some f -> f | None -> nan

(* The kernel's report carries the reduced value as text. `true` / `false` are
   booleans; anything else is a number or an open term. *)
let kernel_bool (s : string) : bool option =
  match String.lowercase_ascii (String.trim s) with
  | "true" -> Some true
  | "false" -> Some false
  | _ -> None

(** Decision when the kernel returned no report. `None` from
    `austral_unf_json` means EITHER the kernel/bridge is genuinely
    unavailable (documented no-op: the compiler keeps working outside the
    unfer checkout) OR the translate entry point ran and failed — and every
    failure of `austral_unf_translate` is recorded on the bridge's
    last-error channel. Passing silently in the second case would hide a
    real kernel↔compiler consistency failure — the exact thing this gate
    exists to catch — so a recorded error becomes a visible rejection. *)
let verdict_on_missing_report ~module_name (id : Identifier.identifier)
    (kernel_err : string option) : verdict =
  match kernel_err with
  | Some msg ->
      VerdictReject
        (Printf.sprintf
           "module %s constant `%s`: deltanet UNF gate could not check via \
            the kernel (bridge error: %s); not passing silently"
           module_name (Identifier.ident_string id) msg)
  | None -> VerdictOk (* kernel/bridge unavailable: no-op *)

(** The legacy value comparison: booleans against booleans, everything
    else through [value_to_number]/[kernel_number]. Kept as the fallback
    for when the kernel answers the UNF question but the conversion answer
    is unusable (a malformed defeq report, or a bridge without the defeq
    surface). *)
let compare_values ~module_name (id : Identifier.identifier) (declared : value)
    (kernel_value : string) (src : string) : verdict =
  match (declared, kernel_bool kernel_value) with
  | VBool b, Some kb when b = kb -> VerdictOk
  | VBool b, Some kb when b <> kb -> (
      VerdictReject
        (Printf.sprintf
           "module %s constant `%s` is a boolean, the compiler \
             evaluates it to %b but the kernel's `uk_austral_unf` \
             reduces the emitted program to %b"
           module_name (Identifier.ident_string id) b kb))
  | _ ->
      let dv = value_to_number declared in
      let kv = kernel_number kernel_value in
      if Float.is_nan dv || Float.is_nan kv then VerdictOk
      else if dv = kv then VerdictOk
      else
        VerdictReject
          (Printf.sprintf
             "module %s constant `%s` disagrees with the kernel's \
              deltanet unique normal form: compiler evaluates to %s, \
              kernel's `uk_austral_unf` reduces `%s` to %s"
             module_name (Identifier.ident_string id)
             (match declared with
              | VInt i -> Int64.to_string i
              | VFloat f -> string_of_float f
              | VBool b -> string_of_bool b)
             src kernel_value)

let check_constant ~module_name (id, init) : verdict =
  match (to_austral init, eval init) with
  | Some src, Some declared ->
      (match CamlCompiler_rust_bridge.austral_unf_json src with
       | None ->
           verdict_on_missing_report ~module_name id
             (CamlCompiler_rust_bridge.last_jit_error ())
       | Some json -> (
           match report_value json with
           | None -> VerdictOk (* open term / non-numeric: nothing to check *)
           | Some kernel_value ->
               (* Conversion check first: the initializer and the compiler's
                  own evaluated literal must reduce to the same net —
                  witness included. Equal nets ⇒ equal values, so when the
                  kernel answers this question it subsumes the value
                  comparison, and it is *exact* where the float comparison
                  was lossy (Int64 magnitudes above 2^53 round through
                  float_of_string, and NaN never equals itself). *)
               (match
                  CamlCompiler_rust_bridge.austral_defeq_json src
                    (literal_source declared)
                with
               | Some djson -> (
                   match report_equivalent djson with
                   | Some true -> VerdictOk
                   | Some false ->
                       VerdictReject
                         (Printf.sprintf
                            "module %s constant `%s`: the compiler evaluates \
                             the initializer `%s` to %s, but the kernel's \
                             conversion check (`uk_austral_defeq`) says it \
                             does not reduce to the same unique normal form \
                             as that literal — no isomorphism witness \
                             (compiler↔kernel consistency failure)"
                            module_name (Identifier.ident_string id) src
                            (literal_source declared))
                   | None ->
                       compare_values ~module_name id declared kernel_value src)
               | None -> (
                   (* Same doctrine as [verdict_on_missing_report]: a
                      recorded bridge error is a visible rejection, never a
                      silent pass; no error means a bridge without the defeq
                      surface, so the legacy value comparison still runs. *)
                   match CamlCompiler_rust_bridge.last_jit_error () with
                   | Some msg ->
                       VerdictReject
                         (Printf.sprintf
                            "module %s constant `%s`: deltanet conversion \
                             check could not run via the kernel (bridge \
                             error: %s); not passing silently"
                            module_name (Identifier.ident_string id) msg)
                   | None ->
                       compare_values ~module_name id declared kernel_value
                         src))))
  | _ -> VerdictOk (* not in the translatable subset: no-op *)

(** The pass as registered: checks every top-level constant. *)
let check ~module_name ~foreign_externals:_ ~constants : verdict =
  if not (enabled ()) then VerdictOk
  else List.fold_left (fun acc c -> match acc with VerdictReject _ -> acc | VerdictOk -> check_constant ~module_name c) VerdictOk constants

(** Register the pass (idempotent). Called from `Vm_plugin.boot`. *)
let install () =
  if List.mem "deltanet_unf" (list_registered ()) then ()
  else register ~name:"deltanet_unf" check
