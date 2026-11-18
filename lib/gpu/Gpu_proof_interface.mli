(*
   PC04 source → KernelIR reflection for the clamp fragment.

   `of_typed` reads the checked monomorphic typed AST and `of_ir` reads the
   actual KernelIR; `witness` requires them to describe the same comparison,
   literal, branch value and parameter. `render` emits the MM0 fragment
   assembled after `word.mm0`; `check_artifact` re-checks a recorded artifact
   against a fresh reflection, so a digest edit cannot make a mismatched
   correspondence pass.
*)

exception Refused of string

type reflection = {
  entry: string;
  param_name: string;
  param_index: int;
  comparison: Common.comparison_operator;
  literal: string;
  branch: string;
  operator: string;  (* "umeet" for `>`, "ujoin" for `<` *)
}

val describe : reflection -> string
(** The canonical `-- reflect:` line recorded on the artifact. *)

val of_typed : entry:string -> Stages.Mtast.mono_module -> reflection
(** Reflect the entry's typed body. Refuses a body that is not the admitted
    `if <param> <cmp> <literal> then return <literal> else return <param>`. *)

val of_ir : entry:string -> Gpu_ir.kernel -> reflection
(** Reflect the actual KernelIR body. Refuses a different shape. *)

val witness : reflection -> reflection -> reflection
(** Require the source and IR reflections to agree; returns the source
    reflection with the IR parameter index. *)

val render :
  ?source_sha256:string -> ?ir_sha256:string -> reflection -> string * string
(** `(mm0, mm1)` for the reflected fragment. *)

val check_artifact : reflection -> string -> unit
(** Refuse an artifact whose recorded reflection or `ir_clamp` definition
    disagrees with the fresh reflection. *)
