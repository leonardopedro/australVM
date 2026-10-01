(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open Stages.Tast

type verdict =
  | VerdictOk
  | VerdictReject of string

(** A pass that sees only the gate-level facts: the module name, the kernel
    foreign externals it imports, and its top-level constants. This is the
    WHYML_CYCLE §3 signature (extended with [constants], which the deltanet
    UNF gate needs). Kept as-is so the existing Why3/deltanet/NPU plugins
    continue to compile unchanged. *)
type gate_pass =
  module_name:string ->
  foreign_externals:string list ->
  constants:(Identifier.identifier * texpr) list ->
  verdict

(** A pass that sees the whole typed module. The gate signature cannot observe
    per-declaration pragmas, which is what the liquid contract pass and its
    acceptance test need, so PLAN_liquid_types.md L1 introduces this second
    pass kind alongside [gate_pass] rather than widening the gate. *)
type typed_pass = Stages.Tast.typed_module -> verdict

(** One ordered registry holding both kinds, kept in *registration order* so
    that [names] reports the order passes were installed in. [run_on_typed]
    walks it in that order and the first [VerdictReject] wins. *)
type entry =
  | Gate of string * gate_pass
  | Typed of string * typed_pass

let registry : entry list ref = ref []

let entry_name = function
  | Gate (name, _) -> name
  | Typed (name, _) -> name

(** Register, replacing an existing entry of the same name *in place* so that
    registration is idempotent in both membership and order: re-registering a
    pass cannot accumulate duplicates, and cannot silently move it to the end
    of the run order. A new name is appended. *)
let add (e : entry) : unit =
  let name = entry_name e in
  let rec replace = function
    | [] ->
       [ e ]
    | (e' : entry) :: rest ->
       if entry_name e' = name then
         e :: rest
       else
         e' :: replace rest
  in
  registry := replace !registry

let register ~name check = add (Gate (name, check))

let register_typed ~name check = add (Typed (name, check))

let reset () =
  registry := []

let names () =
  List.map (fun (e : entry) -> entry_name e) !registry

let list_registered () =
  List.filter_map
    (fun (e : entry) ->
      match e with
      | Gate (name, _) -> Some name
      | Typed _ -> None)
    !registry

let unregister name =
  registry := List.filter (fun (e : entry) -> entry_name e <> name) !registry

(** The foreign externals a typed module imports: `TForeignFunction` decls
    whose external symbol name is a kernel symbol (`uk_*` / `uz_*`). This is
    exactly the set the JIT registers and the module manifest grants — the
    compiler-side mirror of the `GrantSet.kernel` namespace. *)
let foreign_externals_of (TypedModule (_, decls)) : string list =
  List.filter_map
    (fun d ->
      match d with
      | TForeignFunction (_, _, _, _, _, external_name, _, _) ->
          let s = String.trim external_name in
          let is_kernel =
            (String.length s >= 3 && String.sub s 0 3 = "uk_")
            || (String.length s >= 3 && String.sub s 0 3 = "uz_")
          in
          if is_kernel then Some s else None
      | _ ->
          None)
    decls

(** The top-level constant declarations of a typed module, as
    `(name, initializer)` pairs. Passes that need the module's compile-time
    arithmetic (e.g. the deltanet UNF consistency gate) consume this — the
    same surface the JIT's module manifest exposes. *)
let constants_of (TypedModule (_, decls)) : (Identifier.identifier * texpr) list =
  List.filter_map
    (fun d ->
      match d with
      | TConstant (_, _, name, _, init, _) -> Some (name, init)
      | _ -> None)
    decls

(** The pragmas carried by each function-like declaration of a typed module.
    This is the surface the gate signature cannot reach, and the reason
    [typed_pass] exists: PLAN_liquid_types.md L1 threads declaration pragmas
    through to the typed AST so the liquid pass (and its acceptance test) can
    read them. *)
let decl_pragmas_of (TypedModule (_, decls)) : (Identifier.identifier * Common.pragma list) list =
  List.filter_map
    (fun d ->
      match d with
      | TFunction (_, _, name, _, _, _, _, _, pragmas) -> Some (name, pragmas)
      | TForeignFunction (_, _, name, _, _, _, _, pragmas) -> Some (name, pragmas)
      | _ -> None)
    decls

(** Run only the gate passes, against facts supplied by the caller. *)
let run ~module_name ~foreign_externals ~constants =
  List.fold_left
    (fun acc (e : entry) ->
      match acc with
      | VerdictReject _ -> acc
      | VerdictOk ->
          (match e with
           | Gate (_, check) -> check ~module_name ~foreign_externals ~constants
           | Typed _ -> acc))
    VerdictOk !registry

(** Run every registered pass — gate and typed alike — over a typed module, in
    registration order. The first [VerdictReject] wins and short-circuits. *)
let run_on_typed (m : Stages.Tast.typed_module) : verdict =
  let (TypedModule (module_name, _)) = m in
  let module_name = Identifier.mod_name_string module_name in
  let gate = run ~module_name ~foreign_externals:(foreign_externals_of m)
      ~constants:(constants_of m) in
  match gate with
  | VerdictReject _ -> gate
  | VerdictOk ->
      List.fold_left
        (fun acc (e : entry) ->
          match acc with
          | VerdictReject _ -> acc
          | VerdictOk ->
              (match e with
               | Gate _ -> acc
               | Typed (_, check) -> check m))
        VerdictOk !registry
