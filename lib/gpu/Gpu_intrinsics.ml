(*
   Canonical Nat32 intrinsic identity (PLAN_gpu_kernels.md G04).

   The GPU subset admits exactly the standard `ModularArithmetic(Nat32)` and
   `BitwiseOperations(Nat32)` instances. Identity comes from the compiled
   canonical `Austral.Pervasive` module in the environment — the instance
   declaration and its method records — never from a method's textual name in
   user source. A user-defined `modularAdd`, an aliasing import that resolves
   elsewhere, or a module that merely claims the Pervasive name does not
   produce these IDs.

   The builtin bodies are `@embed` arithmetic. Admitting the *instance method*
   is the audited leaf exception; arbitrary user `@embed` stays refused by
   Gpu_subset.
*)

open EnvTypes

let canonical_nat32 (ty: Type.ty): bool =
  match ty with
  | Type.Integer (Type.Unsigned, Type.Width32) -> true
  | _ -> false

let method_intrinsics = [
  ("modularAdd", Gpu_ir.Add);
  ("modularSubtract", Gpu_ir.Subtract);
  ("modularMultiply", Gpu_ir.Multiply);
  ("bitwiseAnd", Gpu_ir.BitwiseAnd);
  ("bitwiseOr", Gpu_ir.BitwiseOr);
  ("bitwiseXor", Gpu_ir.BitwiseXor);
  ("bitwiseNot", Gpu_ir.BitwiseNot);
]

let table_of_env (env: Env.env): (Id.ins_meth_id * Gpu_ir.intrinsic) list =
  match Env.get_module_by_name env BuiltIn.pervasive_module_name with
  | None -> []
  | Some (ModRec { id = mod_id; _ }) ->
     let instances = Env.module_instances env mod_id in
     List.concat_map
       (fun (decl: decl) ->
         match decl with
         | Instance { id; argument; _ } when canonical_nat32 argument ->
            List.filter_map
              (fun (method_name, intrinsic) ->
                let name = Identifier.make_ident method_name in
                match Env.get_instance_method_from_instance_id_and_method_name
                        env id name with
                | Some (InsMethRec { id = method_id; _ }) -> Some (method_id, intrinsic)
                | None -> None)
              method_intrinsics
         | _ -> [])
       instances

type t = (Id.ins_meth_id * Gpu_ir.intrinsic) list

let classify (table: t) (id: Id.ins_meth_id): Gpu_ir.intrinsic option =
  List.assoc_opt id table

let known_names: string list = List.map fst method_intrinsics
