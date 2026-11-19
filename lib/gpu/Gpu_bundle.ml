(*
   Deterministic GPU bundle metadata (PLAN_gpu_kernels.md G02/G06).

   `kernel.json` is the device-independent description of a compiled kernel.
   The semantic identity is the canonical KernelIR without source locations;
   the WGSL bytes have their own digest. Nothing here authenticates a bundle:
   the digests detect stale or mixed artifacts (KERNEL_V1 §4).
*)

type source = {
  path: string;
  sha256: string;
}

type evidence = {
  source: string;
  (* Which GPU source admission produced the kernel. Recorded here because it
     is a property of how the entry was admitted, not of the emitted IR. *)
  source_profile: string;
  shader_validation: string;
  device_kind: string;
  execution: string;
  proof: string;
}

type t = {
  entry: string;
  input_arity: int;
  compiler: string;
  sources: source list;
  kernel: Gpu_ir.kernel;
  ir_sha256: string;
  wgsl: string;
  wgsl_sha256: string;
  requirements: Yojson.Safe.t;
  evidence: evidence;
}

let schema = "austral.gpu.kernel.v1"
let backend = "wgsl"

let numeric_profile (k: Gpu_ir.kernel): string =
  Gpu_ir.profile_name k.Gpu_ir.numeric_profile

(** The numeric contract travels with the bundle instead of living only in
    prose, so the checker can refuse a kernel whose requirements disagree with
    its IR. The tolerance is the one the profile admits, in ULP of the CPU
    binary32 reference, per admitted operation. *)
let numeric_contract (k: Gpu_ir.kernel): Yojson.Safe.t =
  match k.Gpu_ir.numeric_profile with
  | Gpu_ir.U32ModularV1 ->
     `Assoc [ "profile", `String (Gpu_ir.profile_name Gpu_ir.U32ModularV1);
              "element", `String "u32";
              "operations", `List [ `String "add"; `String "subtract";
                                   `String "multiply"; `String "bitwiseAnd";
                                   `String "bitwiseOr"; `String "bitwiseXor";
                                   `String "bitwiseNot" ];
              "rounding", `String "modulo-2^32";
              "contraction", `String "not-applicable";
              "domain", `String "all-2^32";
              "subnormals", `String "not-applicable";
              "overflow", `String "defined-wraparound";
              "tolerance_ulp_per_operation", `Int 0 ]
  | Gpu_ir.WgslF32V1 ->
     `Assoc [ "profile", `String (Gpu_ir.profile_name Gpu_ir.WgslF32V1);
              "element", `String "f32";
              "operations", `List [ `String "add"; `String "subtract";
                                   `String "multiply" ];
              "rounding", `String "binary32-round-to-nearest-even";
              (* The emitted expression order is fixed and no `fma` is written,
                 but a backend may still fuse a multiply followed by an add:
                 measured on Intel UHD 630, 0.1*2.5+0.2 returns 0x3ee66667
                 where two roundings return 0x3ee66666. The profile permits
                 that and budgets for it (1 ULP per admitted operation)
                 rather than claiming a rounding the platform does not
                 guarantee. *)
              "contraction", `String "permitted";
              "domain", `String "finite";
              "subnormals", `String "outside-profile";
              "overflow", `String "outside-profile";
              "tolerance_ulp_per_operation", `Int 1 ]

let compiler_identity (): string =
  (* The actual compiler version is authoritative. A git revision is included
     only when the build supplied one; otherwise it is explicitly unknown
     rather than guessed. *)
  let build =
    match Sys.getenv_opt "AUSTRAL_BUILD_ID" with
    | Some id when String.trim id <> "" -> id
    | _ -> "unknown"
  in
  Printf.sprintf "austral-%s build=%s" Version.version_string build

let default_evidence (): evidence =
  { source = "checked";
    source_profile = "v1";
    shader_validation = "not-run";
    device_kind = "not-probed";
    execution = "not-run";
    proof = "not-run" }

let digest_source ~(path: string) (contents: string): source =
  { path; sha256 = Gpu_ir.sha256_hex contents }

(** The canonical builtin source is part of the admitted closure: a change to
    Pervasive's Nat32 instance definitions must change the bundle's provenance
    digests even though it never appears in the user's file list. *)
let builtin_sources (): source list =
  [ digest_source ~path:"builtin/Austral.Pervasive.aui" BuiltInModules.pervasive_interface_source;
    digest_source ~path:"builtin/Austral.Pervasive.aum" BuiltInModules.pervasive_body_source ]

let requirements_json (k: Gpu_ir.kernel): Yojson.Safe.t =
  let element = Gpu_ir.element_name (Gpu_ir.element_of_profile k.numeric_profile) in
  let bindings =
    List.init k.input_arity (fun i ->
      `Assoc [ "group", `Int 0;
               "binding", `Int i;
               "role", `String "input";
               "access", `String "read";
               "element", `String element;
               "stride_bytes", `Int 4 ])
    @ [ `Assoc [ "group", `Int 0;
                 "binding", `Int k.input_arity;
                 "role", `String "output";
                 "access", `String "read_write";
                 "element", `String element;
                 "stride_bytes", `Int 4 ] ]
  in
  `Assoc [ "bindings", `List bindings;
           "workgroup_size", `Int k.memory.workgroup_size;
           "max_elements", `Int 1048576;
           "required_features", `List [];
           "backend_features", `List [];
           "numeric", numeric_contract k ]

let make ?(evidence = default_evidence ()) ?(wgsl = "") ~(sources: source list)
    (kernel: Gpu_ir.kernel): t =
  Gpu_ir.check_kernel kernel;
  let sources =
    List.sort (fun (a: source) (b: source) -> compare a.path b.path)
      (sources @ builtin_sources ())
  in
  { entry = kernel.entry_name;
    input_arity = kernel.input_arity;
    compiler = compiler_identity ();
    sources;
    kernel;
    ir_sha256 = Gpu_ir.semantic_sha256 kernel;
    wgsl;
    wgsl_sha256 = Gpu_ir.sha256_hex wgsl;
    requirements = requirements_json kernel;
    evidence }

let json_of_source (s: source) =
  `Assoc [ "path", `String s.path; "sha256", `String s.sha256 ]

let json_of_evidence (e: evidence) =
  `Assoc [ "source", `String e.source;
           "source_profile", `String e.source_profile;
           "shader_validation", `String e.shader_validation;
           "device_kind", `String e.device_kind;
           "execution", `String e.execution;
           "proof", `String e.proof ]

let to_yojson (b: t): Yojson.Safe.t =
  `Assoc [
    "schema", `String schema;
    "backend", `String backend;
    "numeric_profile", `String (numeric_profile b.kernel);
    "entry", `String b.entry;
    "input_arity", `Int b.input_arity;
    "compiler", `String b.compiler;
    "sources", `List (List.map json_of_source b.sources);
    "kernel_ir", Gpu_ir.to_yojson b.kernel;
    "ir_sha256", `String b.ir_sha256;
    "wgsl_sha256", `String b.wgsl_sha256;
    "requirements", b.requirements;
    "evidence", json_of_evidence b.evidence;
    "proof_manifest", `Null;
  ]

let render_kernel_json (b: t): string =
  Yojson.Safe.to_string (to_yojson b) ^ "\n"

let bundle_digest (b: t): string =
  Gpu_ir.sha256_hex (render_kernel_json b ^ b.wgsl)
