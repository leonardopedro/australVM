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

   `wgsl-f32-v1` resolves the canonical `TrappingArithmetic(Float32)` add /
   subtract / multiply methods by the same instance-declaration identity.
   `trappingDivide` is registered as *rejected*, so the refusal names the
   operation rather than claiming the method is unknown.
*)

open EnvTypes

let canonical_nat32 (ty: Type.ty): bool =
  match ty with
  | Type.Integer (Type.Unsigned, Type.Width32) -> true
  | _ -> false

let canonical_float32 (ty: Type.ty): bool =
  match ty with Type.SingleFloat -> true | _ -> false

let method_intrinsics = [
  ("modularAdd", Gpu_ir.Add);
  ("modularSubtract", Gpu_ir.Subtract);
  ("modularMultiply", Gpu_ir.Multiply);
  ("bitwiseAnd", Gpu_ir.BitwiseAnd);
  ("bitwiseOr", Gpu_ir.BitwiseOr);
  ("bitwiseXor", Gpu_ir.BitwiseXor);
  ("bitwiseNot", Gpu_ir.BitwiseNot);
]

(* wgsl-f32-v1 admits add/subtract/multiply only. Division and the trapping
   contract are deliberately absent: the method resolves here as a *rejected*
   name so the subset can say why, instead of reporting an unknown method. *)
let float_method_intrinsics = [
  ("trappingAdd", Gpu_ir.F32Add);
  ("trappingSubtract", Gpu_ir.F32Subtract);
  ("trappingMultiply", Gpu_ir.F32Multiply);
]

let float_rejected_methods = ["trappingDivide"]

let methods_of_instance env id names =
  List.filter_map
    (fun (method_name, intrinsic) ->
      let name = Identifier.make_ident method_name in
      match Env.get_instance_method_from_instance_id_and_method_name
              env id name with
      | Some (InsMethRec { id = method_id; _ }) -> Some (method_id, intrinsic)
      | None -> None)
    names

type t = {
  admitted: (Id.ins_meth_id * Gpu_ir.intrinsic) list;
  rejected: Id.ins_meth_id list;
}

let table_of_env (env: Env.env): t =
  match Env.get_module_by_name env BuiltIn.pervasive_module_name with
  | None -> { admitted = []; rejected = [] }
  | Some (ModRec { id = mod_id; _ }) ->
     let instances = Env.module_instances env mod_id in
     let admitted =
       List.concat_map
         (fun (decl: decl) ->
           match decl with
           | Instance { id; argument; _ } when canonical_nat32 argument ->
              methods_of_instance env id method_intrinsics
           | Instance { id; argument; _ } when canonical_float32 argument ->
              methods_of_instance env id float_method_intrinsics
           | _ -> [])
         instances
     in
     let rejected =
       List.concat_map
         (fun (decl: decl) ->
           match decl with
           | Instance { id; argument; _ } when canonical_float32 argument ->
              List.filter_map
                (fun method_name ->
                  match Env.get_instance_method_from_instance_id_and_method_name
                          env id (Identifier.make_ident method_name) with
                  | Some (InsMethRec { id = method_id; _ }) -> Some method_id
                  | None -> None)
                float_rejected_methods
           | _ -> [])
         instances
     in
     { admitted; rejected }

let classify (table: t) (id: Id.ins_meth_id): Gpu_ir.intrinsic option =
  List.assoc_opt id table.admitted

let is_rejected (table: t) (id: Id.ins_meth_id): bool =
  List.mem id table.rejected

let known_names: string list = List.map fst method_intrinsics
let known_float_names: string list = List.map fst float_method_intrinsics
