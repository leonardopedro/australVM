(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open Stages.Tast

type verdict =
  | VerdictOk
  | VerdictReject of string

type pass = {
  name : string;
  check :
    module_name:string ->
    foreign_externals:string list ->
    constants:(Identifier.identifier * texpr) list ->
    verdict;
}

let registry : pass list ref = ref []

let register ~name check =
  registry := { name; check } :: !registry

let reset () =
  registry := []

let list_registered () =
  List.rev_map (fun p -> p.name) !registry

(** The foreign externals a typed module imports: `TForeignFunction` decls
    whose external symbol name is a kernel symbol (`uk_*` / `uz_*`). This is
    exactly the set the JIT registers and the module manifest grants — the
    compiler-side mirror of the `GrantSet.kernel` namespace. *)
let foreign_externals_of (TypedModule (_, decls)) : string list =
  List.filter_map
    (fun d ->
      match d with
      | TForeignFunction (_, _, _, _, _, external_name, _) ->
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

let run ~module_name ~foreign_externals ~constants =
  List.fold_left
    (fun acc p ->
      match acc with
      | VerdictReject _ -> acc
      | VerdictOk -> p.check ~module_name ~foreign_externals ~constants)
    VerdictOk (List.rev !registry)

let run_on_typed (m : Stages.Tast.typed_module) : verdict =
  let (TypedModule (module_name, _)) = m in
  let module_name = Identifier.mod_name_string module_name in
  run ~module_name ~foreign_externals:(foreign_externals_of m)
    ~constants:(constants_of m)
