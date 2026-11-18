(*
   G13: narrow memory-plan obligations for the v1 identity map wrapper.

   The obligations are about the *actual* bundle: the parser recomputes the
   semantic IR digest and the shader digest, checks the runtime requirement
   record, and refuses a bundle whose memory plan is not the admitted identity
   plan. The emitted WhyML is the argument for the arithmetic/index goals; the
   MM0 output states the same obligations but carries no proof: PC03's
   composition is instantiated for the plan in `mm0/proof_chain/gpu_plan.mm0`,
   and its residual premises are listed in `docs/proof/GPU_PLAN.md`.
*)

exception Error of string

type source = {
  path: string;
  sha256: string;
}

type t = {
  entry: string;
  input_arity: int;
  workgroup_size: int;
  max_elements: int;
  ir_sha256: string;
  wgsl_sha256: string;
  bundle_sha256: string;
  plan_sha256: string;
  sources: source list;
}

(** Read a published `kernel.json` + `kernel.wgsl` directory. The bundle's
    digests are checked against the bytes actually present; a memory plan that
    is not map1/map2 with the identity output index and pre-load guard is
    refused. *)
val of_bundle : string -> t

(** WhyML goals for the emitted wrapper. Contains only definitions and goals;
    there is no `axiom` or `theorem` to trust. *)
val render_whyml : t -> string

(** MM0 statements. The vocabulary is self-declared and the file carries
    `-- certificate: [gap]`: it is a statement artifact, not a certificate. *)
val render_mm0 : t -> string

(** Comments for the companion `.mm1`; no proof steps are generated until the
    shared proof chain can discharge the statements. *)
val render_mm1 : t -> string

val manifest_json : t -> Yojson.Safe.t

(** Substring helper used by the shape tests (and by the bundle shader check). *)
val contains : string -> string -> bool
