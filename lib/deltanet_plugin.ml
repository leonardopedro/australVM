(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The DeltaNets UNF compiler pass (the Austral → DeltaNets translation as a
   compiler gate). Every top-level `TConstant` with a closed arithmetic
   initializer is serialized to the Austral subset the kernel's
   `uk_austral_unf` symbol accepts, reduced through the kernel's
   interaction-net unique-normal-form reducer (via the OCaml↔Rust bridge),
   and compared against this pass's own independent OCaml evaluator for the
   same expression. A disagreement means the compiler's view of a constant
   and the kernel's canonical normal form diverge — a compiler-consistency
   failure — and the module is rejected, mirroring how the Why3 gate rejects
   modules importing ungranted symbols.

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
  | TComparison (op, a, b) ->
      let op_s =
        match op with
        | Equal -> "="
        | NotEqual -> "<>"
        | LessThan -> "<"
        | LessThanOrEqual -> "<="
        | GreaterThan -> ">"
        | GreaterThanOrEqual -> ">="
      in
      binop op_s a b
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
                      src kernel_value)))
  | _ -> VerdictOk (* not in the translatable subset: no-op *)

(** The pass as registered: checks every top-level constant. *)
let check ~module_name ~foreign_externals:_ ~constants : verdict =
  if not (enabled ()) then VerdictOk
  else List.fold_left (fun acc c -> match acc with VerdictReject _ -> acc | VerdictOk -> check_constant ~module_name c) VerdictOk constants

(** Register the pass (idempotent). Called from `Vm_plugin.boot`. *)
let install () =
  if List.mem "deltanet_unf" (list_registered ()) then ()
  else register ~name:"deltanet_unf" check
