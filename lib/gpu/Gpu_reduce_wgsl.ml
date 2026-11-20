(*
   G22b/G22c: staged WGSL for the fixed `map-sum-u32` reduction.

   The scalar mapper is the admitted KernelIR body (the Austral function), so
   the reduction's arithmetic still comes from the source; this module only
   adds the fixed collective structure around it. Two layouts share that
   structure:

   - `Lane1` is the original schedule: one element per lane, a 64-element
     group. `map_sum` (pass 0) reads the caller's input and applies the
     mapper; `add_partials` (every later pass) reads the previous pass's
     partials.
   - `Lane4` is the tuning variant: each lane maps four elements at
     `i = group*256 + j*64 + lane`, so a group covers 256 elements and the
     number of passes shrinks (1 048 576 needs three instead of four).

   Both keep the guard-before-load rule (a missing element contributes the
   identity and is never mapped), one barrier after the load and after each
   of the strides 32,16,8,4,2,1, and exactly one write per group from lane 0.

   `reduce.json` carries the program identity (operation/profile included),
   both entry digests, the mapper IR and the source closure. The launch plan
   (per-run pass counts and extents) is *not* in the artifact: the host derives
   it from validated input with `Gpu_reduce.schedule`/`schedule_x4`.
*)

let schema = "austral.gpu.reduction.v1"
let backend = "wgsl"
let mapper_entry = "map_sum"
let partial_entry = "add_partials"
let mapper_function = "mapper"
let tile = Gpu_reduce.tile

type variant = Lane1 | Lane4

let operation = Gpu_reduce.operation
let profile = Gpu_reduce.profile

let operation_for = function
  | Lane1 -> Gpu_reduce.operation
  | Lane4 -> Gpu_reduce.operation_x4

let profile_for = function
  | Lane1 -> Gpu_reduce.profile
  | Lane4 -> Gpu_reduce.profile_x4

let elements_per_group = function
  | Lane1 -> Gpu_reduce.tile
  | Lane4 -> Gpu_reduce.group_elements

let bindings =
  "@group(0) @binding(0) var<storage, read> input0: array<u32>;\n\
   @group(0) @binding(1) var<storage, read_write> output0: array<u32>;\n\
   var<workgroup> scratch: array<u32, 64>;\n\n"

(* The mapper is the admitted KernelIR, rendered exactly as Gpu_wgsl renders a
   pointwise entry, but as a callable function instead of a binding wrapper. *)
let mapper_source (kernel: Gpu_ir.kernel): string =
  Gpu_ir.check_kernel kernel;
  let helper_rets =
    List.map
      (fun (h: Gpu_ir.helper) -> (h.Gpu_ir.id, h.Gpu_ir.ret))
      kernel.Gpu_ir.helpers
  in
  let env =
    { Gpu_wgsl.params = [Gpu_ir.TyU32];
      locals = [];
      helper_rets }
  in
  let (body, _) = Gpu_wgsl.render_stmt env 1 kernel.Gpu_ir.body in
  let helpers =
    String.concat "" (List.map Gpu_wgsl.render_helper kernel.Gpu_ir.helpers)
  in
  helpers
  ^ Printf.sprintf "fn %s(p0: u32) -> u32 {\n%s}\n\n" mapper_function body

let strides = [32; 16; 8; 4; 2; 1]

let reduction_tail (buffer: Buffer.t) =
  List.iter
    (fun stride ->
      Buffer.add_string buffer
        (Printf.sprintf
           "    if (lane < %du) {\n\
            \        scratch[lane] = scratch[lane] + scratch[lane + %du];\n\
            \    }\n\
            \    workgroupBarrier();\n"
           stride stride))
    strides;
  Buffer.add_string buffer
    "    if (lane == 0u) {\n\
     \        output0[wid.x] = scratch[0u];\n\
     \    }\n\
     }\n"

(* One element per lane: guarded load, identity for a tail lane. *)
let collective_lane1 ~(source: string) ~(name: string): string =
  let buffer = Buffer.create 512 in
  Buffer.add_string buffer
    (Printf.sprintf
       "@compute @workgroup_size(%d)\n\
        fn %s(@builtin(local_invocation_id) lid: vec3<u32>,\n\
        \        @builtin(workgroup_id) wid: vec3<u32>) {\n\
        \    let lane = lid.x;\n\
        \    let i = wid.x * %du + lane;\n\
        \    if (i >= arrayLength(&input0)) {\n\
        \        scratch[lane] = %du;\n\
        \    } else {\n\
        \        scratch[lane] = %s;\n\
        \    }\n\
        \    workgroupBarrier();\n"
       tile name tile Gpu_reduce.identity source);
  reduction_tail buffer;
  Buffer.contents buffer

(* Four elements per lane: four guarded loads, accumulated into one slot. A
   lane's missing elements each contribute the identity, so a short tail needs
   no padding and never maps a missing index. *)
let collective_lane4 ~(source: int -> string) ~(name: string): string =
  let buffer = Buffer.create 768 in
  Buffer.add_string buffer
    (Printf.sprintf
       "@compute @workgroup_size(%d)\n\
        fn %s(@builtin(local_invocation_id) lid: vec3<u32>,\n\
        \        @builtin(workgroup_id) wid: vec3<u32>) {\n\
        \    let lane = lid.x;\n\
        \    let i0 = wid.x * %du + lane;\n\
        \    let i1 = i0 + %du;\n\
        \    let i2 = i0 + %du;\n\
        \    let i3 = i0 + %du;\n\
        \    var acc: u32 = %du;\n"
       tile name Gpu_reduce.group_elements tile (tile * 2) (tile * 3)
       Gpu_reduce.identity);
  for j = 0 to 3 do
    Buffer.add_string buffer
      (Printf.sprintf
         "    if (i%d < arrayLength(&input0)) {\n\
          \        acc = acc + %s;\n\
          \    }\n"
         j (source j))
  done;
  Buffer.add_string buffer
    "    scratch[lane] = acc;\n\
     \    workgroupBarrier();\n";
  reduction_tail buffer;
  Buffer.contents buffer

let collective ~(variant: variant) ~(source: int -> string) ~(name: string): string =
  match variant with
  | Lane1 -> collective_lane1 ~source:(source 0) ~name
  | Lane4 -> collective_lane4 ~source ~name

let entry_texts ?(variant = Lane1) (kernel: Gpu_ir.kernel):
  (string * string * string) =
  let mapper_source_text = function
    | 0 when variant = Lane1 -> "mapper(input0[i])"
    | j -> Printf.sprintf "mapper(input0[i%d])" j
  in
  let partial_source_text = function
    | 0 when variant = Lane1 -> "input0[i]"
    | j -> Printf.sprintf "input0[i%d]" j
  in
  let mapper =
    collective ~variant ~source:mapper_source_text ~name:mapper_entry
  in
  let partial =
    collective ~variant ~source:partial_source_text ~name:partial_entry
  in
  let preamble =
    "// Generated from typed KernelIR by the Austral GPU backend (map-sum-u32). \
     Do not edit.\n"
  in
  let mapper_full = preamble ^ bindings ^ mapper_source kernel ^ mapper in
  let partial_full = preamble ^ bindings ^ mapper_source kernel ^ partial in
  (preamble ^ bindings ^ mapper_source kernel ^ mapper ^ "\n" ^ partial,
   mapper_full, partial_full)

let render ?(variant = Lane1) (kernel: Gpu_ir.kernel): string =
  let (wgsl, _, _) = entry_texts ~variant kernel in
  wgsl

let program_text ?(variant = Lane1) (kernel: Gpu_ir.kernel): string =
  Printf.sprintf
    "operation=%s\nprofile=%s\nentries=%s,%s\ngroup=%d\nscratch-bytes=%d\n\
     mapper-ir=%s\n"
    (operation_for variant) (profile_for variant) mapper_entry partial_entry
    tile Gpu_reduce.scratch_bytes (Gpu_ir.semantic_sha256 kernel)

let program_id ?(variant = Lane1) (kernel: Gpu_ir.kernel): string =
  Gpu_ir.sha256_hex (program_text ~variant kernel)

let metadata ?(variant = Lane1)
    ?(evidence = Gpu_bundle.default_evidence ())
    ~(sources: Gpu_bundle.source list) (kernel: Gpu_ir.kernel): string =
  let (wgsl, mapper_full, partial_full) = entry_texts ~variant kernel in
  let sources =
    List.sort
      (fun (a: Gpu_bundle.source) (b: Gpu_bundle.source) -> compare a.path b.path)
      (sources @ Gpu_bundle.builtin_sources ())
  in
  let entries =
    [ `Assoc [ "name", `String mapper_entry;
               "role", `String "mapper-input";
               "digest", `String (Gpu_ir.sha256_hex mapper_full) ];
      `Assoc [ "name", `String partial_entry;
               "role", `String "partial-input";
               "digest", `String (Gpu_ir.sha256_hex partial_full) ] ]
  in
  let json =
    `Assoc
      [ "schema", `String schema;
        "backend", `String backend;
        "operation", `String (operation_for variant);
        "reduction_profile", `String (profile_for variant);
        "group_size", `Int tile;
        "scratch_bytes", `Int Gpu_reduce.scratch_bytes;
        "max_inputs", `Int Gpu_reduce.max_inputs;
        "identity", `Int Gpu_reduce.identity;
        "operator", `String "wrapping-add-u32";
        "entries", `List entries;
        "program_sha256", `String (program_id ~variant kernel);
        "program_text", `String (program_text ~variant kernel);
        "sources", `List (List.map Gpu_bundle.json_of_source sources);
        "kernel_ir", Gpu_ir.to_yojson kernel;
        "ir_sha256", `String (Gpu_ir.semantic_sha256 kernel);
        "wgsl_sha256", `String (Gpu_ir.sha256_hex wgsl);
        "evidence", Gpu_bundle.json_of_evidence evidence ]
  in
  Yojson.Safe.to_string json ^ "\n"
