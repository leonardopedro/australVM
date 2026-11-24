(*
   Versioned MLIR artifact metadata (PLAN_gpu_kernels.md G20b).

   A published MLIR bundle is four files whose identities are chained, not
   four files in one directory:

     kernel.kernelir   the canonical KernelIR bytes (the Gpu_ir identity)
     kernel.mlir       Gpu_mlir.render of exactly those bytes
     kernel.ptx        the pinned toolchain's lowering of exactly that module
     bundle.json       the digests, the toolchain identity and the ABI

   Each derivative names its predecessor, so a mixed set is detectable without
   recompiling: `verify_dir` re-hashes every file and compares, and it also
   requires the emitter's own `// kernelir-sha256:` header to equal the
   recorded IR digest — a tampered `.kernelir` whose digest in `bundle.json`
   was recomputed therefore still fails, because the module it was rendered
   from says otherwise.

   This module never runs a tool. Verification and lowering are the compiler's
   (`--target-type=mlir`) or `scripts/gpu/mlir-probe`'s (G19a) job; `validate`
   in `Gpu_mlir` is structural. What is recorded here is what an artifact set
   claims, and what the checker refuses.
*)

let schema = "austral.gpu.mlir.v1"

let sha256_hex = Gpu_ir.sha256_hex

(** The three toolchain facts a lowering record carries. The pipeline string is
    a single argv element at the call site; here it is recorded verbatim so a
    reader can reproduce it. *)
let target = "sm_70"

(** The chip is part of the lowering identity: PTX targets one virtual ISA, and
    a module built for a newer chip cannot be loaded on an older device. The
    default stays `sm_70` so every record written before G21a keeps its
    meaning; `--gpu-chip` names another one and the record carries it. *)
let valid_chip (chip: string): bool =
  let n = String.length chip in
  if n < 5 || String.sub chip 0 3 <> "sm_" then false
  else begin
    let body = String.sub chip 3 (n - 3) in
    let body_len = String.length body in
    let digits =
      if body.[body_len - 1] >= 'a' && body.[body_len - 1] <= 'f' then
        String.sub body 0 (body_len - 1)
      else body
    in
    let len = String.length digits in
    len >= 2 && len <= 3
    && String.for_all (fun c -> c >= '0' && c <= '9') digits
  end

let pipeline_for (chip: string): string =
  "--gpu-lower-to-nvvm-pipeline=cubin-format=isa cubin-chip=" ^ chip

let pipeline = pipeline_for target

(* ------------------------------------------------------------------ *)
(* Reading a lowered module                                             *)
(* ------------------------------------------------------------------ *)

(** MLIR prints a `gpu.object`'s assembly as a quoted string with `\NN` hex
    escapes (newlines become `\0A`). This is the inverse, nothing more: it is
    how the PTX bytes are recovered from the pipeline's textual output. *)
let unescape_assembly (text: string): string =
  let buf = Buffer.create (String.length text) in
  let n = String.length text in
  let i = ref 0 in
  while !i < n do
    if text.[!i] = '\\' && !i + 2 < n then begin
      (match int_of_string_opt ("0x" ^ String.sub text (!i + 1) 2) with
       | Some 0 -> ()                                   (* NUL padding *)
       | Some code -> Buffer.add_char buf (Char.chr code)
       | None -> Buffer.add_string buf (String.sub text !i 3));
      i := !i + 3
    end else begin
      Buffer.add_char buf text.[!i];
      i := !i + 1
    end
  done;
  Buffer.contents buf

(** Substring search without a regexp dependency. *)
let contains (haystack: string) (needle: string): bool =
  let hn = String.length haystack and nn = String.length needle in
  let rec go i =
    if i + nn > hn then false
    else if String.sub haystack i nn = needle then true
    else go (i + 1)
  in
  nn > 0 && go 0

(** The assembly text of the first `gpu.object`/`gpu.binary` in a lowered
    module, or `None` when the pipeline produced no such artifact. *)
let extract_assembly (lowered: string): string option =
  let needle = "assembly = \"" in
  let n = String.length needle in
  let rec find i =
    if i >= String.length lowered then None
    else if String.length lowered - i >= n && String.sub lowered i n = needle then
      Some (i + n)
    else find (i + 1)
  in
  match find 0 with
  | None -> None
  | Some start ->
     let buf = Buffer.create 4096 in
     let rec scan i =
       if i >= String.length lowered then Buffer.contents buf
       else
         match lowered.[i] with
         | '\\' when i + 2 < String.length lowered ->
            (* Keep the escape; `unescape_assembly` decodes it. *)
            Buffer.add_string buf (String.sub lowered i 3);
            scan (i + 3)
         | '"' -> Buffer.contents buf
         | c -> Buffer.add_char buf c; scan (i + 1)
     in
     Some (unescape_assembly (scan start))

type ptx_facts = {
  ptx_entry: string;
  ptx_target: string;
  ptx_address_size: string;
  ptx_params: int;
  ptx_param_types: string list;
  ptx_address_space: string;
}

(** What the lowered text itself says. All of it is read from the bytes; a
    missing field reads as `"?"` rather than as a default that would pass. *)
let ptx_facts (ptx: string): ptx_facts =
  let lines = String.split_on_char '\n' ptx in
  let starts_with p s = String.length s >= String.length p && String.sub s 0 (String.length p) = p in
  let trimmed l = String.trim l in
  let field prefix extract =
    let rec go = function
      | [] -> "?"
      | l :: rest ->
         let l = trimmed l in
         if starts_with prefix l then
           let rest' = String.sub l (String.length prefix) (String.length l - String.length prefix) in
           extract (trimmed rest')
         else go rest
    in
    go lines
  in
  (* A declaration's name ends at whitespace or at the opening parenthesis,
     whichever comes first (`.visible .entry main(`). *)
  let word s =
    let rec stop i =
      if i >= String.length s then i
      else match s.[i] with ' ' | '(' | ',' -> i | _ -> stop (i + 1)
    in
    String.sub s 0 (stop 0)
  in
  let params =
    List.filter_map
      (fun l ->
        let l = trimmed l in
        if starts_with ".param ." l then
          let rest = String.sub l 8 (String.length l - 8) in
          Some (word (trimmed rest))
        else None)
      lines
  in
  let has s = contains ptx s in
  { ptx_entry = field ".visible .entry " word;
    ptx_target = field ".target " word;
    ptx_address_size = field ".address_size " word;
    ptx_params = List.length params;
    ptx_param_types = List.sort_uniq compare params;
    ptx_address_space =
      if has "cvta.to.global" then "generic -> global (cvta.to.global)"
      else "no conversion seen" }

(** The ABI the recorded kernel requires: one `memref<extent x i32>` per
    operand, each lowering to five 64-bit words (alloc ptr, aligned ptr,
    offset, size, stride), plus the same five for the output. `ptxas` is not
    involved, so these are PTX `.param` declarations, not a cubin header. *)
let expected_params ~(arity: int): int = 5 * (arity + 1)

let param_layout = "one memref -> 5 x .u64: alloc ptr, aligned ptr, offset, size, stride"

(** Every way the lowered ABI disagrees with the recorded kernel. An empty list
    is agreement, not proof of a correct launch. *)
let abi_mismatches ~(arity: int) ~(chip: string) (f: ptx_facts): string list =
  let problems = ref [] in
  let add s = problems := s :: !problems in
  if f.ptx_entry <> "main" then
    add (Printf.sprintf "entry is %s, expected main" f.ptx_entry);
  if f.ptx_target <> chip then
    add (Printf.sprintf "target is %s, expected %s" f.ptx_target chip);
  if f.ptx_address_size <> "64" then
    add (Printf.sprintf "address size is %s, expected 64" f.ptx_address_size);
  if f.ptx_params <> expected_params ~arity then
    add (Printf.sprintf "%d parameter(s), expected %d for %d memref operand(s)"
           f.ptx_params (expected_params ~arity) (arity + 1));
  if f.ptx_param_types <> ["u64"] then
    add (Printf.sprintf "parameter types %s, expected [u64]"
           (String.concat "," f.ptx_param_types));
  if not (contains f.ptx_address_space "cvta.to.global") then
    add "no generic -> global address-space conversion";
  List.rev !problems

(* ------------------------------------------------------------------ *)
(* The record                                                          *)
(* ------------------------------------------------------------------ *)

type lowering = {
  target: string;
  pipeline: string;
  toolchain: string;
  ptxas: string;
}

type t = {
  entry: string;
  input_arity: int;
  extent: int;
  (* The block size the module declares (`gpu.known_block_size`), which is the
     workgroup the memory plan chose. A launch may use a smaller block and a
     grid; it may not silently use a larger one, so the runner reads this
     rather than picking a block from the extent. *)
  workgroup: int;
  numeric_profile: string;
  sources: Gpu_bundle.source list;
  numeric_contract: Yojson.Safe.t;
  ir_sha256: string;
  mlir_sha256: string;
  ptx_sha256: string;
  lowering: lowering;
  abi: ptx_facts;
}

let file_kernelir = "kernel.kernelir"
let file_mlir = "kernel.mlir"
let file_ptx = "kernel.ptx"
let file_json = "bundle.json"

let derivative_json ~stage ~file ~sha ~predecessor ~producer =
  `Assoc ([ "stage", `String stage;
            "file", `String file;
            "sha256", `String sha;
            "produced_by", `String producer ]
          @ (match predecessor with
             | None -> []
             | Some p -> [ "predecessor", `String p ]))

let to_yojson (b: t): Yojson.Safe.t =
  `Assoc
    [ "schema", `String schema;
      "entry", `String b.entry;
      "input_arity", `Int b.input_arity;
      "extent", `Int b.extent;
      "workgroup", `Int b.workgroup;
      "numeric_profile", `String b.numeric_profile;
      "sources", `List (List.map (fun (s: Gpu_bundle.source) ->
                             `Assoc [ "path", `String s.Gpu_bundle.path;
                                      "sha256", `String s.Gpu_bundle.sha256 ])
                        b.sources);
      "derivatives",
      `List
        [ derivative_json ~stage:"kernelir" ~file:file_kernelir
            ~sha:b.ir_sha256 ~predecessor:None
            ~producer:"Gpu_ir.canonical_bytes";
          derivative_json ~stage:"mlir" ~file:file_mlir
            ~sha:b.mlir_sha256 ~predecessor:(Some "kernelir")
            ~producer:"Gpu_mlir.render (G20a)";
          derivative_json ~stage:"ptx" ~file:file_ptx
            ~sha:b.ptx_sha256 ~predecessor:(Some "mlir")
            ~producer:b.lowering.pipeline ];
      "lowering",
      `Assoc [ "target", `String b.lowering.target;
               "pipeline", `String b.lowering.pipeline;
               "toolchain", `String b.lowering.toolchain;
               "ptxas", `String b.lowering.ptxas ];
      "abi",
      `Assoc [ "entry", `String b.abi.ptx_entry;
               "params", `Int b.abi.ptx_params;
               "param_types", `List (List.map (fun t -> `String t) b.abi.ptx_param_types);
               "param_layout", `String param_layout;
               "address_space", `String b.abi.ptx_address_space ];
      "numeric_contract", b.numeric_contract ]

let render (b: t): string = Yojson.Safe.pretty_to_string (to_yojson b) ^ "\n"

let make ~(entry: string) ~(extent: int) ~(sources: Gpu_bundle.source list)
    ~(kernel: Gpu_ir.kernel) ~(mlir: string) ~(ptx: string)
    ~(lowering: lowering): t =
  let ir_sha256 = Gpu_ir.semantic_sha256 kernel in
  { entry; input_arity = kernel.Gpu_ir.input_arity; extent;
    workgroup = kernel.Gpu_ir.memory.Gpu_ir.workgroup_size;
    numeric_profile = Gpu_ir.profile_name kernel.Gpu_ir.numeric_profile;
    sources; numeric_contract = Gpu_bundle.numeric_contract kernel; ir_sha256;
    mlir_sha256 = sha256_hex mlir; ptx_sha256 = sha256_hex ptx;
    lowering; abi = ptx_facts ptx }

(* ------------------------------------------------------------------ *)
(* Checking a published directory                                      *)
(* ------------------------------------------------------------------ *)

let read_file path =
  let ic = open_in_bin path in
  let n = in_channel_length ic in
  let s = really_input_string ic n in
  close_in ic;
  s

let member path json =
  try Yojson.Safe.Util.member path json with _ -> `Null

let to_string_opt = function `String s -> Some s | _ -> None
let to_int_opt = function `Int i -> Some i | _ -> None

(** The emitter writes its input identity into the module. That header is what
    ties a `.mlir` to a `.kernelir` independently of `bundle.json`. *)
let kernelir_header_digest (mlir: string): string option =
  let prefix = "// kernelir-sha256: " in
  let lines = String.split_on_char '\n' mlir in
  let rec go = function
    | [] -> None
    | l :: rest ->
       if String.length l >= String.length prefix
          && String.sub l 0 (String.length prefix) = prefix then
         Some (String.trim (String.sub l (String.length prefix)
                              (String.length l - String.length prefix)))
       else go rest
  in
  go lines

(** `Ok (record, warnings)` when the set is internally consistent; `Error
    reasons` when it is not. Warnings are facts worth printing but not
    refusals (today: none, kept for symmetry with the other checkers). *)
let verify_dir (dir: string): (t * string list, string list) result =
  let problems = ref [] in
  let note s = problems := s :: !problems in
  let json_path = Filename.concat dir file_json in
  if not (Sys.file_exists json_path) then
    Error [ Printf.sprintf "%s is missing" json_path ]
  else
    let json = try Yojson.Safe.from_string (read_file json_path)
               with Yojson.Json_error e ->
                 note (Printf.sprintf "%s does not parse: %s" file_json e); `Null in
    if json = `Null then Error (List.rev !problems)
    else begin
      let expect_file stage file recorded =
        let path = Filename.concat dir file in
        if not (Sys.file_exists path) then
          note (Printf.sprintf "the %s artifact %s is missing" stage file)
        else
          let actual = sha256_hex (read_file path) in
          if actual <> recorded then
            note (Printf.sprintf
                    "the %s artifact %s hashes to %s…, but %s records %s… — a \
                     derivative must not be replaced without its predecessor"
                    stage file (String.sub actual 0 12) file_json
                    (String.sub recorded 0 12))
      in
      let schema_v = member "schema" json |> to_string_opt in
      if schema_v <> Some schema then
        note (Printf.sprintf "schema is %s, expected %s"
                (match schema_v with Some s -> s | None -> "missing") schema);
      let derivatives = match member "derivatives" json with `List l -> l | _ -> [] in
      let recorded stage =
        List.fold_left
          (fun acc d ->
            if member "stage" d |> to_string_opt = Some stage then
              match member "file" d |> to_string_opt, member "sha256" d |> to_string_opt with
              | Some file, Some sha -> Some (file, sha)
              | _ -> acc
            else acc)
          None derivatives
      in
      let ir_recorded =
        match recorded "kernelir" with
        | None -> note "no kernelir derivative is recorded"; None
        | Some (file, sha) -> expect_file "kernelir" file sha; Some (file, sha)
      in
      let mlir_recorded =
        match recorded "mlir" with
        | None -> note "no mlir derivative is recorded"; None
        | Some (file, sha) -> expect_file "mlir" file sha; Some (file, sha)
      in
      (match recorded "ptx" with
       | None -> note "no ptx derivative is recorded"
       | Some (file, sha) -> expect_file "ptx" file sha);
      (* The chain: the module's own header must name the recorded IR. *)
      (match ir_recorded, mlir_recorded with
       | Some (_, ir_sha), Some (mlir_file, _) ->
          let mlir = read_file (Filename.concat dir mlir_file) in
          (match kernelir_header_digest mlir with
           | Some header when header = ir_sha -> ()
           | Some header ->
              note (Printf.sprintf
                      "%s was rendered from kernelir %s…, but the bundle records \
                       %s… — recomputing a digest does not repair the pair"
                      mlir_file (String.sub header 0 12) (String.sub ir_sha 0 12))
           | None ->
              note (Printf.sprintf "%s carries no kernelir-sha256 header" mlir_file))
       | _ -> ());
      (* The ABI must agree with the arithmetic the extent and arity imply. *)
      let arity = match member "input_arity" json |> to_int_opt with
        | Some a -> a | None -> note "input_arity is missing"; 0 in
      let ptx =
        match recorded "ptx" with
        | Some (file, _) ->
           let path = Filename.concat dir file in
           if Sys.file_exists path then Some (read_file path) else None
        | None -> None
      in
      let chip =
        match member "lowering" json |> fun l -> member "target" l with
        | `String s -> s
        | _ -> note "the lowering target is missing"; ""
      in
      (match ptx with
       | None -> ()
       | Some text ->
          let facts = ptx_facts text in
          List.iter note (abi_mismatches ~arity ~chip facts));
      if !problems <> [] then Error (List.rev !problems)
      else begin
        (* Rebuild the record from the files when everything agrees, so callers
           can print measured facts instead of the JSON's claims. *)
        let ir_sha = match ir_recorded with Some (_, s) -> s | None -> "" in
        let mlir_sha = match mlir_recorded with Some (_, s) -> s | None -> "" in
        let ptx_file, ptx_sha =
          match recorded "ptx" with Some (f, s) -> (f, s) | None -> ("", "")
        in
        let ptx_text = if ptx_file = "" then "" else read_file (Filename.concat dir ptx_file) in
        let facts = ptx_facts ptx_text in
        let sources =
          match member "sources" json with
          | `List l ->
             List.filter_map
               (fun s ->
                 match member "path" s |> to_string_opt, member "sha256" s |> to_string_opt with
                 | Some p, Some h -> Some { Gpu_bundle.path = p; sha256 = h }
                 | _ -> None)
               l
          | _ -> []
        in
        let lowering_json = member "lowering" json in
        let lowering =
          { target = (match member "target" lowering_json |> to_string_opt with Some s -> s | None -> "");
            pipeline = (match member "pipeline" lowering_json |> to_string_opt with Some s -> s | None -> "");
            toolchain = (match member "toolchain" lowering_json |> to_string_opt with Some s -> s | None -> "");
            ptxas = (match member "ptxas" lowering_json |> to_string_opt with Some s -> s | None -> "") }
        in
        let extent = match member "extent" json |> to_int_opt with Some e -> e | None -> 0 in
        let workgroup =
          match member "workgroup" json |> to_int_opt with
          | Some w when w >= 1 && w <= 1024 -> w
          | Some w ->
             note (Printf.sprintf
                     "the recorded workgroup is %d; no device accepts a block \
                      outside 1..1024" w);
             0
          | None -> note "workgroup is missing"; 0 in
        let declared =
          Printf.sprintf "gpu.known_block_size = array<i32: %d, 1, 1>" workgroup in
        (match mlir_recorded with
         | Some (mlir_file, _) when workgroup >= 1 ->
            let mlir_text = read_file (Filename.concat dir mlir_file) in
            if not (contains mlir_text declared) then
              note (Printf.sprintf
                      "%s declares no `%s`, so the recorded workgroup is not \
                       the one the module was built with" mlir_file declared)
         | _ -> ());
        let entry = match member "entry" json |> to_string_opt with Some e -> e | None -> "" in
        let profile = match member "numeric_profile" json |> to_string_opt with
          | Some p -> p | None -> "" in
        if lowering.pipeline <> pipeline_for lowering.target then
          Error [ Printf.sprintf
                    "the recorded pipeline is not the one its target names (%s)"
                    (pipeline_for lowering.target) ]
        else
          Ok ({ entry; input_arity = arity; extent; workgroup;
                numeric_profile = profile;
                sources;
                numeric_contract =
                  (match member "numeric_contract" json with
                   | `Null -> `Null
                   | other -> other);
                ir_sha256 = ir_sha; mlir_sha256 = mlir_sha; ptx_sha256 = ptx_sha;
                lowering; abi = facts }, [])
      end
    end
