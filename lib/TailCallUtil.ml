(*
    Tail Call Utility Functions
    Part of SafestOS extension to Austral
*)

open Stages.Mtast
open MonoType

(* Check if an expression is a function call that can be a tail call *)
let is_function_call (expr: mexpr): bool =
  match expr with
  | MConcreteFuncall _ | MGenericFuncall _ 
  | MConcreteMethodCall _ | MGenericMethodCall _ ->
      true
  | _ -> false

(* Check if a statement ends with a tail call or return *)
let rec ends_with_tail (stmt: mstmt): bool =
  match stmt with
  | MReturn _ -> true
  | MBlock (_, s2) -> ends_with_tail s2
  | MIf (_, tb, fb) -> ends_with_tail tb && ends_with_tail fb
  | MCase (_, _, whens, _) ->
      List.for_all (fun (MTypedWhen (_, _, body)) -> ends_with_tail body) whens
  | _ -> false

(* Get the return type of a function *)
let get_function_return_type (params: mvalue_parameter list) (rt: mono_ty): mono_ty =
  rt

(* Check if a function has the cell step signature *)
let is_cell_step_function (name: identifier) (params: mvalue_parameter list) (rt: mono_ty): bool =
  (* cell_step: void (*)(void* state) or similar *)
  (* We need: takes state pointer, returns nothing (noreturn) *)
  let name_str = ident_string name in
  name_str = "cell_step" && List.length params = 1

(* Generate function signature for cell wrapper *)
let gen_cell_function_signature (desc: string) (params: mvalue_parameter list) (rt: mono_ty): string =
  let param_str = List.map (function
    | MValueParameter (n, t) -> MonoTypeUtil.show_mono_ty t ^ " " ^ ident_string n
  ) params
  |> String.concat ", "
  in
  let rt_str = MonoTypeUtil.show_mono_ty rt in
  desc ^ "(" ^ param_str ^ ") -> " ^ rt_str
