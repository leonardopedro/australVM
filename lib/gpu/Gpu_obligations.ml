(*
   G13: memory-plan obligations for the v1 GPU identity wrapper.

   The input is a *published bundle*, not a retyped plan: the parser rebuilds
   the KernelIR, recomputes its canonical digest, re-hashes `kernel.wgsl`, and
   checks the runtime requirements record (bindings, roles, workgroup size and
   the 1048576-element job bound). A bundle with a different plan is refused,
   not translated into a neighbouring obligation.

   The WhyML file states the arithmetic consequences of the actual plan as
   goals. Why3's `int` is mathematical; every index goal carries the u32 bound
   so the emitted wrapper's index is proved representable instead of an
   unbounded integer being silently narrowed. The runtime facts (all input
   extents are N, the output is a fresh buffer, and distinct invocations have
   distinct `global_invocation_id.x`) are listed as premises in the manifest
   and the documentation; they are not axioms in this file.
*)

exception Error of string

let fail fmt = Printf.ksprintf (fun s -> raise (Error s)) fmt

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

let read_file path =
  let ic = open_in_bin path in
  Fun.protect
    ~finally:(fun () -> close_in ic)
    (fun () ->
      let n = in_channel_length ic in
      really_input_string ic n)

let json_member key json = Yojson.Safe.Util.member key json

let json_string key json =
  match json_member key json with
  | `String s -> s
  | _ -> fail "bundle field %S is not a string" key

let json_int key json =
  match json_member key json with
  | `Int n -> n
  | _ -> fail "bundle field %S is not an integer" key

let json_bool key json =
  match json_member key json with
  | `Bool b -> b
  | _ -> fail "bundle field %S is not a boolean" key

let json_list key json =
  match json_member key json with
  | `List xs -> xs
  | _ -> fail "bundle field %S is not an array" key

(* -------------------------------------------------------------------------- *)
(* KernelIR decoding                                                          *)
(* -------------------------------------------------------------------------- *)

let intrinsic_of_string = function
  | "add" -> Gpu_ir.Add
  | "subtract" -> Gpu_ir.Subtract
  | "multiply" -> Gpu_ir.Multiply
  | "bitwiseAnd" -> Gpu_ir.BitwiseAnd
  | "bitwiseOr" -> Gpu_ir.BitwiseOr
  | "bitwiseXor" -> Gpu_ir.BitwiseXor
  | "bitwiseNot" -> Gpu_ir.BitwiseNot
  | s -> fail "unknown intrinsic %S" s

let comparison_of_string = function
  | "eq" -> Gpu_ir.Eq
  | "ne" -> Gpu_ir.NotEqual
  | "lt" -> Gpu_ir.LessThan
  | "le" -> Gpu_ir.LessThanOrEqual
  | "gt" -> Gpu_ir.GreaterThan
  | "ge" -> Gpu_ir.GreaterThanOrEqual
  | s -> fail "unknown comparison %S" s

let boolean_op_of_string = function
  | "and" -> Gpu_ir.Conjunction
  | "or" -> Gpu_ir.Disjunction
  | s -> fail "unknown boolean operator %S" s

let rec expr_of_json json =
  match json_string "kind" json with
  | "param" -> Gpu_ir.Param (json_int "index" json)
  | "local" -> Gpu_ir.Local (json_int "index" json)
  | "u32" -> Gpu_ir.U32 (Gpu_ir.u32_of_string (json_string "value" json))
  | "bool" -> Gpu_ir.Bool (json_bool "value" json)
  | "intrinsic" ->
     Gpu_ir.Intrinsic
       (intrinsic_of_string (json_string "op" json),
        List.map expr_of_json (json_list "args" json))
  | "compare" ->
     Gpu_ir.Compare
       (comparison_of_string (json_string "op" json),
        expr_of_json (json_member "left" json),
        expr_of_json (json_member "right" json))
  | "boolean" ->
     Gpu_ir.BooleanOp
       (boolean_op_of_string (json_string "op" json),
        expr_of_json (json_member "left" json),
        expr_of_json (json_member "right" json))
  | "not" -> Gpu_ir.Not (expr_of_json (json_member "value" json))
  | "conditional" ->
     Gpu_ir.Conditional
       (expr_of_json (json_member "condition" json),
        expr_of_json (json_member "then" json),
        expr_of_json (json_member "else" json))
  | "call" ->
     Gpu_ir.Call
       (json_int "helper" json,
        List.map expr_of_json (json_list "args" json))
  | k -> fail "unknown KernelIR expression kind %S" k

let rec stmt_of_json json =
  match json_string "kind" json with
  | "let" -> Gpu_ir.Let (json_int "local" json, expr_of_json (json_member "value" json))
  | "seq" -> Gpu_ir.Seq (List.map stmt_of_json (json_list "items" json))
  | "if" ->
     Gpu_ir.If
       (expr_of_json (json_member "condition" json),
        stmt_of_json (json_member "then" json),
        stmt_of_json (json_member "else" json))
  | "return" -> Gpu_ir.Return (expr_of_json (json_member "value" json))
  | k -> fail "unknown KernelIR statement kind %S" k

let value_type_of_string = function
  | "u32" -> Gpu_ir.TyU32
  | "bool" -> Gpu_ir.TyBool
  | s -> fail "unknown KernelIR value type %S" s

let helper_of_json json =
  { Gpu_ir.id = json_int "id" json;
    params = List.map
        (fun j -> match j with
           | `String s -> value_type_of_string s
           | _ -> fail "KernelIR helper param is not a string")
        (json_list "params" json);
    ret = value_type_of_string (json_string "ret" json);
    body = stmt_of_json (json_member "body" json) }

let memory_of_json json =
  let map_kind =
    match json_string "map" json with
    | "map1" -> Gpu_ir.Map1
    | "map2" -> Gpu_ir.Map2
    | s -> fail "unsupported v1 memory map %S" s
  in
  let index =
    match json_string "output_index" json with
    | "identity" -> true
    | s -> fail "unsupported v1 output index %S (expected identity)" s
  in
  { Gpu_ir.map_kind;
    identity_output_index = index;
    guard_before_load = json_bool "guard_before_load" json;
    workgroup_size = json_int "workgroup_size" json }

let kernel_of_json json =
  { Gpu_ir.entry_name = json_string "entry" json;
    input_arity = json_int "input_arity" json;
    helpers = List.map helper_of_json (json_list "helpers" json);
    body = stmt_of_json (json_member "body" json);
    memory = memory_of_json (json_member "memory" json) }

(* -------------------------------------------------------------------------- *)
(* Bundle checks                                                              *)
(* -------------------------------------------------------------------------- *)

let contains haystack needle =
  let n = String.length needle and h = String.length haystack in
  let rec at i =
    i + n <= h && (String.sub haystack i n = needle || at (i + 1))
  in
  n = 0 || at 0

let check_requirements ~(arity: int) ~(workgroup_size: int) json =
  let max_elements = json_int "max_elements" json in
  if max_elements <> 1048576 then
    fail "v1 job bound must be 1048576, bundle says %d" max_elements;
  if json_int "workgroup_size" json <> workgroup_size then
    fail "requirements workgroup_size disagrees with the KernelIR plan";
  let bindings = json_list "bindings" json in
  if List.length bindings <> arity + 1 then
    fail "requirements has %d bindings for arity %d" (List.length bindings) arity;
  let binding i =
    match List.nth_opt bindings i with
    | Some b -> b
    | None -> fail "missing requirements binding %d" i
  in
  for i = 0 to arity - 1 do
    let b = binding i in
    if json_int "group" b <> 0 || json_int "binding" b <> i
       || json_string "role" b <> "input"
       || json_string "access" b <> "read"
       || json_string "element" b <> "u32"
       || json_int "stride_bytes" b <> 4 then
      fail "requirements input binding %d is not the v1 read-only u32 binding" i
  done;
  let out = binding arity in
  if json_int "group" out <> 0 || json_int "binding" out <> arity
     || json_string "role" out <> "output"
     || json_string "access" out <> "read_write"
     || json_string "element" out <> "u32"
     || json_int "stride_bytes" out <> 4 then
    fail "requirements output binding is not the v1 fresh read-write u32 binding";
  max_elements

let check_shader ~(workgroup_size: int) (wgsl: string) =
  let wrapper =
    Printf.sprintf
      "@compute @workgroup_size(%d)\n\
       fn main(@builtin(global_invocation_id) gid: vec3<u32>) {\n\
       \    let i = gid.x;\n\
       \    if (i >= arrayLength(&input0)) { return; }\n\
       \    output0[i] = scalar_entry("
      workgroup_size
  in
  if not (contains wgsl wrapper) then
    fail "kernel.wgsl does not contain the admitted identity map wrapper"

let of_bundle dir =
  let json_path = Filename.concat dir "kernel.json" in
  let wgsl_path = Filename.concat dir "kernel.wgsl" in
  if not (Sys.file_exists json_path) then fail "no kernel.json in %s" dir;
  if not (Sys.file_exists wgsl_path) then fail "no kernel.wgsl in %s" dir;
  let json_bytes = read_file json_path in
  let wgsl = read_file wgsl_path in
  let json =
    try Yojson.Safe.from_string json_bytes
    with Yojson.Json_error msg -> fail "kernel.json is not valid JSON: %s" msg
  in
  if json_string "schema" json <> Gpu_bundle.schema then
    fail "kernel.json schema is not %s" Gpu_bundle.schema;
  if json_string "backend" json <> Gpu_bundle.backend then
    fail "kernel.json backend is not %s" Gpu_bundle.backend;
  if json_string "numeric_profile" json <> Gpu_bundle.numeric_profile then
    fail "kernel.json numeric profile is not %s" Gpu_bundle.numeric_profile;

  let kernel =
    try kernel_of_json (json_member "kernel_ir" json)
    with Gpu_ir.Ir_error msg -> fail "KernelIR refused: %s" msg
  in
  (try Gpu_ir.check_kernel kernel
   with Gpu_ir.Ir_error msg -> fail "KernelIR refused: %s" msg);
  if json_string "entry" json <> kernel.Gpu_ir.entry_name then
    fail "bundle entry disagrees with kernel_ir.entry";
  if json_int "input_arity" json <> kernel.Gpu_ir.input_arity then
    fail "bundle input_arity disagrees with kernel_ir.input_arity";
  let expected_arity = match kernel.Gpu_ir.memory.Gpu_ir.map_kind with
    | Gpu_ir.Map1 -> 1
    | Gpu_ir.Map2 -> 2
  in
  if kernel.Gpu_ir.input_arity <> expected_arity then
    fail "memory map does not match input arity";

  let ir = Gpu_ir.semantic_sha256 kernel in
  let declared_ir = json_string "ir_sha256" json in
  if declared_ir <> ir then
    fail "ir_sha256 does not match the canonical KernelIR (bundle %s, computed %s)"
      declared_ir ir;
  let wgsl_sha = Gpu_ir.sha256_hex wgsl in
  let declared_wgsl = json_string "wgsl_sha256" json in
  if declared_wgsl <> wgsl_sha then
    fail "wgsl_sha256 does not match kernel.wgsl (bundle %s, computed %s)"
      declared_wgsl wgsl_sha;

  let max_elements =
    check_requirements ~arity:kernel.Gpu_ir.input_arity
      ~workgroup_size:kernel.Gpu_ir.memory.Gpu_ir.workgroup_size
      (json_member "requirements" json)
  in
  check_shader ~workgroup_size:kernel.Gpu_ir.memory.Gpu_ir.workgroup_size wgsl;

  let sources =
    List.map
      (fun s ->
        { path = json_string "path" s; sha256 = json_string "sha256" s })
      (json_list "sources" json)
  in
  { entry = kernel.Gpu_ir.entry_name;
    input_arity = kernel.Gpu_ir.input_arity;
    workgroup_size = kernel.Gpu_ir.memory.Gpu_ir.workgroup_size;
    max_elements;
    ir_sha256 = ir;
    wgsl_sha256 = wgsl_sha;
    bundle_sha256 = Gpu_ir.sha256_hex json_bytes;
    plan_sha256 =
      Gpu_ir.sha256_hex (Yojson.Safe.to_string
                           (Gpu_ir.json_of_memory kernel.Gpu_ir.memory));
    sources }

(* -------------------------------------------------------------------------- *)
(* WhyML                                                                      *)
(* -------------------------------------------------------------------------- *)

let max_groups (t: t) =
  (t.max_elements + t.workgroup_size - 1) / t.workgroup_size

let whyml_header (t: t) =
  let lines = [
    "(* Generated by Gpu_obligations from the actual v1 GPU bundle.";
    Printf.sprintf "   entry=%s arity=%d workgroup_size=%d max_elements=%d"
      t.entry t.input_arity t.workgroup_size t.max_elements;
    Printf.sprintf "   bundle_sha256=%s" t.bundle_sha256;
    Printf.sprintf "   ir_sha256=%s" t.ir_sha256;
    Printf.sprintf "   wgsl_sha256=%s" t.wgsl_sha256;
    Printf.sprintf "   plan_sha256=%s" t.plan_sha256;
    "   Why3 int is mathematical. The index goals bound their values by u32_max,";
    "   so no unbounded integer is extracted into a native int. The runtime";
    "   premises (equal input extents, fresh output, distinct invocation ids)";
    "   are documentation, not axioms: they are listed in the manifest. *)";
  ] in
  String.concat "\n" lines ^ "\n"

let render_whyml (t: t) =
  let lines = [
    "module GpuMemoryPlan";
    "  use int.Int";
    "  use int.EuclideanDivision";
    "";
    Printf.sprintf "  constant max_elements : int = %d" t.max_elements;
    Printf.sprintf "  constant workgroup_size : int = %d" t.workgroup_size;
    Printf.sprintf "  constant max_groups : int = %d" (max_groups t);
    "  constant u32_max : int = 4294967295";
    "";
    "  predicate in_range (n:int) = 0 <= n <= max_elements";
    "  function groups (n:int) : int = div (n + workgroup_size - 1) workgroup_size";
    "  function flat (gid:int) : int = gid";
    "  function out_index (gid:int) : int = flat gid";
    "";
    "  predicate active (n gid:int) = in_range n /\\ 0 <= gid < n";
    "";
    "  goal groups_nonneg :";
    "    forall n:int. in_range n -> groups n >= 0";
    "";
    "  goal groups_bound :";
    "    forall n:int. in_range n ->";
    "      groups n * workgroup_size <= max_elements + workgroup_size - 1";
    "";
    "  goal launch_representable :";
    "    forall n:int. in_range n -> groups n * workgroup_size <= u32_max";
    "";
    "  goal index_representable :";
    "    forall gid n:int. in_range n ->";
    "      0 <= gid < groups n * workgroup_size -> 0 <= flat gid <= u32_max";
    "";
    "  goal guard_implies_bound :";
    "    forall i len n:int. in_range n -> len = n -> 0 <= i < len -> i < n";
    "";
    "  goal flatten_injective :";
    "    forall wg1 lid1 wg2 lid2:int.";
    "      0 <= lid1 < workgroup_size -> 0 <= lid2 < workgroup_size ->";
    "      wg1 * workgroup_size + lid1 = wg2 * workgroup_size + lid2 ->";
    "      wg1 = wg2 /\\ lid1 = lid2";
    "";
    "  goal distinct_active_writes :";
    "    forall g1 g2 n:int.";
    "      active n g1 -> active n g2 -> g1 <> g2 -> out_index g1 <> out_index g2";
    "";
    "  goal write_index_unique :";
    "    forall g1 g2 n:int.";
    "      active n g1 -> active n g2 -> out_index g1 = out_index g2 -> g1 = g2";
    "";
    "end";
  ] in
  whyml_header t ^ String.concat "\n" lines ^ "\n"

(* -------------------------------------------------------------------------- *)
(* MM0 statements                                                             *)
(* -------------------------------------------------------------------------- *)

let render_mm0 (t: t) =
  let lines = [
    "-- gpu_memory.mm0 — G13 memory-plan obligations for one actual v1 bundle.";
    "--";
    "-- These are *statements*, not a certificate: PC03's composition is";
    "-- instantiated for this plan in `mm0/proof_chain/gpu_plan.mm0`, but its";
    "-- residual premises (`docs/proof/GPU_PLAN.md`) are not discharged, so no";
    "-- `theorem` is declared here. Why3";
    "-- evidence for the arithmetic/index goals is emitted beside this file as";
    "-- `gpu_memory.mlw`; Why3 success is a separate route and does not";
    "-- discharge an MM0-required claim. `scripts/gpu-obligations` records the";
    "-- exact split and never reports this file as proved.";
    "--";
    Printf.sprintf "-- entry=%s arity=%d workgroup_size=%d max_elements=%d"
      t.entry t.input_arity t.workgroup_size t.max_elements;
    Printf.sprintf "-- bundle_sha256=%s" t.bundle_sha256;
    Printf.sprintf "-- ir_sha256=%s" t.ir_sha256;
    Printf.sprintf "-- wgsl_sha256=%s" t.wgsl_sha256;
    Printf.sprintf "-- plan_sha256=%s" t.plan_sha256;
    "-- certificate: [gap]";
    "-- proves: 0 theorems";
    "-- theory: self-declared vocabulary";
    "";
    "provable sort Prop;";
    "delimiter $ ( ) $;";
    "";
    "sort nat;";
    "term n0: nat; prefix n0: $0$ prec max;";
    "";
    "term in_range (n: nat): Prop;";
    "term groups (n: nat): nat;";
    "term flat (gid: nat): nat;";
    "term out_index (gid: nat): nat;";
    "term guard_holds (i len: nat): Prop;";
    "term active (n gid: nat): Prop;";
    "term distinct_index (a b: nat): Prop;";
    "";
    "-- obligation 1: the generated flat index is representable.";
    Printf.sprintf "--   in_range n => 0 <= gid < groups n * %d => flat gid <= 4294967295"
      t.workgroup_size;
    "-- obligation 2: the guard implies the element bound.";
    "--   in_range n => len = n => guard_holds i len => i < n";
    "-- obligation 3: active distinct invocations write distinct indices.";
    "--   active n g1 => active n g2 => g1 <> g2 => out_index g1 <> out_index g2";
    "-- obligation 4: inputs have sufficient extents (runtime-checked, trusted";
    "--   at bind time: every input binding length equals n).";
    "-- obligation 5: the fresh output cannot alias an input (runtime allocation";
    "--   fact, trusted; the WGSL wrapper has no alias construction).";
  ] in
  String.concat "\n" lines ^ "\n"

let render_mm1 (t: t) =
  let lines = [
    "-- gpu_memory.mm1 — companion to gpu_memory.mm0.";
    "--";
    "-- No proof steps are generated: G13's MM0 discharge needs the residual";
    "-- obligations in `docs/proof/GPU_PLAN.md` closed against this artifact.";
    "-- The statements above therefore remain `[gap]`; the WhyML";
    "-- file is the separately checked evidence. Do not add an `axiom` here to";
    "-- make the artifact look proved.";
    Printf.sprintf "-- bundle_sha256=%s" t.bundle_sha256;
  ] in
  String.concat "\n" lines ^ "\n"

(* -------------------------------------------------------------------------- *)
(* Manifest                                                                   *)
(* -------------------------------------------------------------------------- *)

let manifest_json (t: t) =
  `Assoc [
    "schema", `String "austral.gpu.memory-obligations.v1";
    "entry", `String t.entry;
    "input_arity", `Int t.input_arity;
    "workgroup_size", `Int t.workgroup_size;
    "max_elements", `Int t.max_elements;
    "bundle_sha256", `String t.bundle_sha256;
    "ir_sha256", `String t.ir_sha256;
    "wgsl_sha256", `String t.wgsl_sha256;
    "plan_sha256", `String t.plan_sha256;
    "sources", `List (List.map (fun s ->
        `Assoc [ "path", `String s.path; "sha256", `String s.sha256 ]) t.sources);
    "runtime_premises", `List [
      `String "every input binding has extent n and the output binding has extent n";
      `String "the output is a fresh allocation and does not alias an input";
      `String "distinct invocations have distinct global_invocation_id.x values";
    ];
    "why3", `Assoc [ "status", `String "not-run"; "prover", `String "alt-ergo" ];
    "mm0", `Assoc [ "status", `String "gap"; "certificate", `String "[gap]" ];
  ]
