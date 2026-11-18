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
let numeric_profile = "u32-modular-v1"

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
  let bindings =
    List.init k.input_arity (fun i ->
      `Assoc [ "group", `Int 0;
               "binding", `Int i;
               "role", `String "input";
               "access", `String "read";
               "element", `String "u32";
               "stride_bytes", `Int 4 ])
    @ [ `Assoc [ "group", `Int 0;
                 "binding", `Int k.input_arity;
                 "role", `String "output";
                 "access", `String "read_write";
                 "element", `String "u32";
                 "stride_bytes", `Int 4 ] ]
  in
  `Assoc [ "bindings", `List bindings;
           "workgroup_size", `Int k.memory.workgroup_size;
           "max_elements", `Int 1048576;
           "required_features", `List [];
           "backend_features", `List [] ]

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
           "shader_validation", `String e.shader_validation;
           "device_kind", `String e.device_kind;
           "execution", `String e.execution;
           "proof", `String e.proof ]

let to_yojson (b: t): Yojson.Safe.t =
  `Assoc [
    "schema", `String schema;
    "backend", `String backend;
    "numeric_profile", `String numeric_profile;
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
