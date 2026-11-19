(*
   Typed KernelIR for the GPU map/zip subset (PLAN_gpu_kernels.md G02).

   This module is the semantic boundary between the checked Austral frontend
   and every emitter. It deliberately has no dependency on the compiler: the
   subset pass (Gpu_subset) builds these values from the monomorphic typed AST,
   and renderers (WGSL, later MLIR) consume them.

   Canonical serialization is defined here: fixed field order, decimal-string
   u32 values, explicit arrays, no source locations. There is no hash-map
   iteration and no process-local identifier in the bytes.
*)

exception Ir_error of string

let fail fmt = Printf.ksprintf (fun s -> raise (Ir_error s)) fmt

(* -------------------------------------------------------------------------- *)
(* Values                                                                      *)
(* -------------------------------------------------------------------------- *)

(** A u32 literal. Stored as a canonical decimal string so no OCaml native int
    range or sign behaviour leaks into the artifact. *)
type u32 = string

let max_u32 = Z.of_string "4294967295"

let u32_of_string (s: string): u32 =
  let z =
    try Z.of_string s
    with _ -> fail "u32 literal %S is not an integer" s
  in
  if Z.sign z < 0 || Z.compare z max_u32 > 0 then
    fail "u32 literal %S is out of range [0,4294967295]" s;
  Z.to_string z

let u32_to_z (s: u32): Z.t = Z.of_string s

(* -------------------------------------------------------------------------- *)
(* Numeric profile                                                             *)
(* -------------------------------------------------------------------------- *)

(** The numeric contract a kernel is compiled against. It is part of the
    semantic IR (and therefore of [ir_sha256]) because the same expressions mean
    different things at different widths: a u32 modular add and an f32 add are
    not the same instruction. v1 remains the default. *)
type numeric_profile = U32ModularV1 | WgslF32V1

let profile_name = function
  | U32ModularV1 -> "u32-modular-v1"
  | WgslF32V1 -> "wgsl-f32-v1"

let profile_of_name = function
  | "u32-modular-v1" -> U32ModularV1
  | "wgsl-f32-v1" -> WgslF32V1
  | s -> fail "unknown GPU numeric profile %S" s

(** The storage element a profile maps over. Both are 4 bytes wide; the
    difference is interpretation, not layout. *)
type element = ElementU32 | ElementF32

let element_name = function ElementU32 -> "u32" | ElementF32 -> "f32"

let element_of_profile = function
  | U32ModularV1 -> ElementU32
  | WgslF32V1 -> ElementF32

(* -------------------------------------------------------------------------- *)
(* IR                                                                          *)
(* -------------------------------------------------------------------------- *)

type intrinsic =
  | Add
  | Subtract
  | Multiply
  | BitwiseAnd
  | BitwiseOr
  | BitwiseXor
  | BitwiseNot
  (* wgsl-f32-v1 only. Kept distinct from the modular u32 ops so a profile
     mix-up is a type/validation error instead of a silent reinterpretation. *)
  | F32Add
  | F32Subtract
  | F32Multiply

type comparison = Eq | NotEqual | LessThan | LessThanOrEqual | GreaterThan | GreaterThanOrEqual

type boolean_op = Conjunction | Disjunction

type unary_bool = Negation

type expr =
  | Param of int
  | Local of int
  | U32 of u32
  (** A binary32 literal, stored as the shortest decimal string that round-trips
      to the exact f32 value (see [f32_of_string]). Rendering it verbatim is
      exact: both Naga and the Rust reference parse it to the same bits. *)
  | F32 of string
  | Bool of bool
  | Intrinsic of intrinsic * expr list
  | Compare of comparison * expr * expr
  | BooleanOp of boolean_op * expr * expr
  | Not of expr
  | Conditional of expr * expr * expr
  | Call of int * expr list

type stmt =
  | Let of int * expr
  | Seq of stmt list
  | If of expr * stmt * stmt
  | Return of expr

type value_type = TyU32 | TyBool | TyF32

let json_of_value_type = function
  | TyU32 -> `String "u32"
  | TyBool -> `String "bool"
  | TyF32 -> `String "f32"

(** Canonical f32 literal text: the shortest `%g` rendering whose decimal
    parse reproduces the same binary32 bits. The string must be a float token
    in WGSL too, so an integral rendering gets a `.0` suffix (`3` would be an
    AbstractInt there). Non-finite input is refused: the profile admits only
    finite binary32 values. *)
let f32_of_string (s: string): string =
  let x =
    try float_of_string s
    with _ -> fail "f32 literal %S is not a float" s
  in
  if not (Float.is_finite x) then
    fail "f32 literal %S is not a finite binary32 value" s;
  let bits = Int32.bits_of_float x in
  (* `Int32.bits_of_float` saturates an out-of-range double to infinity instead
     of raising, so the f32 range is checked on the converted bits. *)
  let exponent = Int32.logand bits 0x7F80_0000l in
  if exponent = 0x7F80_0000l then
    fail "f32 literal %S overflows binary32" s;
  if exponent = 0l && Int32.logand bits 0x007F_FFFFl <> 0l then
    fail "f32 literal %S is subnormal, which is outside the admitted domain" s;
  let rec shortest p =
    if p > 9 then None
    else
      let candidate = Printf.sprintf "%.*g" p x in
      if Int32.bits_of_float (float_of_string candidate) = bits then
        Some candidate
      else shortest (p + 1)
  in
  match shortest 1 with
  | None -> fail "f32 literal %S has no round-tripping decimal form" s
  | Some c ->
     if String.contains c '.' || String.contains c 'e' || String.contains c 'E'
     then c
     else c ^ ".0"

let f32_bits_of_string (s: string): int32 =
  Int32.bits_of_float (float_of_string (f32_of_string s))

type helper = {
  id: int;
  params: value_type list;
  ret: value_type;
  body: stmt;
}

type map_kind = Map1 | Map2

type memory_plan = {
  map_kind: map_kind;
  identity_output_index: bool;
  guard_before_load: bool;
  workgroup_size: int;
}

type kernel = {
  entry_name: string;
  input_arity: int;
  helpers: helper list;
  body: stmt;
  memory: memory_plan;
  numeric_profile: numeric_profile;
}

let default_memory_plan (arity: int): memory_plan =
  if arity <> 1 && arity <> 2 then
    fail "v1 supports input arity 1 or 2, got %d" arity;
  { map_kind = (if arity = 1 then Map1 else Map2);
    identity_output_index = true;
    guard_before_load = true;
    workgroup_size = 64 }

(* -------------------------------------------------------------------------- *)
(* Structural validation                                                       *)
(* -------------------------------------------------------------------------- *)

let intrinsic_arity (i: intrinsic): int =
  match i with
  | Add | Subtract | Multiply | BitwiseAnd | BitwiseOr | BitwiseXor -> 2
  | F32Add | F32Subtract | F32Multiply -> 2
  | BitwiseNot -> 1

let is_float_intrinsic = function
  | F32Add | F32Subtract | F32Multiply -> true
  | _ -> false

let show_intrinsic = function
  | Add -> "add" | Subtract -> "subtract" | Multiply -> "multiply"
  | BitwiseAnd -> "bitwiseAnd" | BitwiseOr -> "bitwiseOr"
  | BitwiseXor -> "bitwiseXor" | BitwiseNot -> "bitwiseNot"
  | F32Add -> "f32Add" | F32Subtract -> "f32Subtract"
  | F32Multiply -> "f32Multiply"

(** [helpers] maps a helper id to its parameter count. *)
let rec check_expr ~(param_count: int) (locals: int list)
    (helpers: (int * int) list) (e: expr): unit =
  match e with
  | Param n ->
     if n < 0 || n >= param_count then
       fail "param index %d outside the declared arity %d" n param_count
  | Local n ->
     if not (List.mem n locals) then fail "reference to undefined local %d" n
  | U32 s -> ignore (u32_of_string s)
  | F32 s -> ignore (f32_of_string s)
  | Bool _ -> ()
  | Intrinsic (i, args) ->
     if List.length args <> intrinsic_arity i then
       fail "intrinsic %s takes %d argument(s), got %d"
         (show_intrinsic i) (intrinsic_arity i) (List.length args);
     List.iter (check_expr ~param_count locals helpers) args
  | Compare (_, a, b) ->
     check_expr ~param_count locals helpers a;
     check_expr ~param_count locals helpers b
  | BooleanOp (_, a, b) ->
     check_expr ~param_count locals helpers a;
     check_expr ~param_count locals helpers b
  | Not a -> check_expr ~param_count locals helpers a
  | Conditional (c, t, f) ->
     check_expr ~param_count locals helpers c;
     check_expr ~param_count locals helpers t;
     check_expr ~param_count locals helpers f
  | Call (f, args) ->
     (match List.assoc_opt f helpers with
      | None -> fail "call to undefined helper %d" f
      | Some expected ->
         if List.length args <> expected then
           fail "helper %d takes %d argument(s), got %d"
             f expected (List.length args));
     List.iter (check_expr ~param_count locals helpers) args

(** Walk a statement sequence, threading the locals it introduces. Returns the
    locals visible after the statement. *)
let rec check_stmt ~(param_count: int) (locals: int list)
    (helpers: (int * int) list) (s: stmt): int list =
  match s with
  | Let (n, e) ->
     check_expr ~param_count locals helpers e;
     if List.mem n locals then fail "local %d is bound twice" n;
     n :: locals
  | Seq stmts ->
     List.fold_left
       (fun loc s -> check_stmt ~param_count loc helpers s)
       locals stmts
  | If (c, t, f) ->
     check_expr ~param_count locals helpers c;
     ignore (check_stmt ~param_count locals helpers t);
     ignore (check_stmt ~param_count locals helpers f);
     locals
  | Return e ->
     check_expr ~param_count locals helpers e;
     locals

(* The profile decides which value language a kernel may use. Keeping this in
   [check_kernel] means a hand-built or re-read kernel cannot mix the two: a
   u32 modular add in an f32 kernel (or the reverse) is refused here, before
   any emitter sees it. *)
let rec check_profile ~(profile: numeric_profile) (e: expr): unit =
  match e with
  | U32 _ when profile = WgslF32V1 ->
     fail "a u32 literal is not part of %s" (profile_name profile)
  | F32 _ when profile = U32ModularV1 ->
     fail "an f32 literal requires the wgsl-f32-v1 profile"
  | Intrinsic (i, args) ->
     if is_float_intrinsic i && profile = U32ModularV1 then
       fail "%s requires the wgsl-f32-v1 profile" (show_intrinsic i);
     if not (is_float_intrinsic i) && profile = WgslF32V1 then
       fail "integer intrinsic %s is not part of %s"
         (show_intrinsic i) (profile_name profile);
     List.iter (check_profile ~profile) args
  | Compare (_, a, b) -> check_profile ~profile a; check_profile ~profile b
  | BooleanOp (_, a, b) -> check_profile ~profile a; check_profile ~profile b
  | Not a -> check_profile ~profile a
  | Conditional (c, t, f) ->
     check_profile ~profile c;
     check_profile ~profile t;
     check_profile ~profile f
  | Call (_, args) -> List.iter (check_profile ~profile) args
  | U32 _ | F32 _ | Param _ | Local _ | Bool _ -> ()

let rec check_stmt_profile ~(profile: numeric_profile) (s: stmt): unit =
  match s with
  | Let (_, e) -> check_profile ~profile e
  | Seq items -> List.iter (check_stmt_profile ~profile) items
  | If (c, t, f) ->
     check_profile ~profile c;
     check_stmt_profile ~profile t;
     check_stmt_profile ~profile f
  | Return e -> check_profile ~profile e

let check_kernel (k: kernel): unit =
  if k.input_arity <> 1 && k.input_arity <> 2 then
    fail "v1 supports input arity 1 or 2, got %d" k.input_arity;
  let helper_signatures =
    List.map (fun (h: helper) -> (h.id, List.length h.params)) k.helpers
  in
  let ids = List.map (fun (h: helper) -> h.id) k.helpers in
  let sorted = List.sort compare ids in
  let rec no_dups = function
    | a :: (b :: _ as rest) -> if a = b then fail "duplicate helper id %d" a else no_dups rest
    | _ -> ()
  in
  no_dups sorted;
  List.iter
    (fun (h: helper) ->
      check_stmt_profile ~profile:k.numeric_profile h.body;
      ignore (check_stmt ~param_count:(List.length h.params) [] helper_signatures h.body))
    k.helpers;
  check_stmt_profile ~profile:k.numeric_profile k.body;
  ignore (check_stmt ~param_count:k.input_arity [] helper_signatures k.body);
  if k.memory.workgroup_size <= 0 || k.memory.workgroup_size > 1024 then
    fail "workgroup size %d outside the v1 range" k.memory.workgroup_size;
  if not k.memory.identity_output_index || not k.memory.guard_before_load then
    fail "v1 requires the identity output index and the pre-load guard"

(* -------------------------------------------------------------------------- *)
(* Canonical JSON                                                              *)
(* -------------------------------------------------------------------------- *)

let json_of_intrinsic = function
  | Add -> `String "add"
  | Subtract -> `String "subtract"
  | Multiply -> `String "multiply"
  | BitwiseAnd -> `String "bitwiseAnd"
  | BitwiseOr -> `String "bitwiseOr"
  | BitwiseXor -> `String "bitwiseXor"
  | BitwiseNot -> `String "bitwiseNot"
  | F32Add -> `String "f32Add"
  | F32Subtract -> `String "f32Subtract"
  | F32Multiply -> `String "f32Multiply"

let json_of_comparison = function
  | Eq -> `String "eq"
  | NotEqual -> `String "ne"
  | LessThan -> `String "lt"
  | LessThanOrEqual -> `String "le"
  | GreaterThan -> `String "gt"
  | GreaterThanOrEqual -> `String "ge"

let json_of_boolean_op = function
  | Conjunction -> `String "and"
  | Disjunction -> `String "or"

let rec json_of_expr = function
  | Param n -> `Assoc [ "kind", `String "param"; "index", `Int n ]
  | Local n -> `Assoc [ "kind", `String "local"; "index", `Int n ]
  | U32 s -> `Assoc [ "kind", `String "u32"; "value", `String (u32_of_string s) ]
  | F32 s -> `Assoc [ "kind", `String "f32"; "value", `String (f32_of_string s) ]
  | Bool b -> `Assoc [ "kind", `String "bool"; "value", `Bool b ]
  | Intrinsic (i, args) ->
     `Assoc [ "kind", `String "intrinsic";
              "op", json_of_intrinsic i;
              "args", `List (List.map json_of_expr args) ]
  | Compare (c, a, b) ->
     `Assoc [ "kind", `String "compare";
              "op", json_of_comparison c;
              "left", json_of_expr a;
              "right", json_of_expr b ]
  | BooleanOp (o, a, b) ->
     `Assoc [ "kind", `String "boolean";
              "op", json_of_boolean_op o;
              "left", json_of_expr a;
              "right", json_of_expr b ]
  | Not a ->
     `Assoc [ "kind", `String "not"; "value", json_of_expr a ]
  | Conditional (c, t, f) ->
     `Assoc [ "kind", `String "conditional";
              "condition", json_of_expr c;
              "then", json_of_expr t;
              "else", json_of_expr f ]
  | Call (f, args) ->
     `Assoc [ "kind", `String "call";
              "helper", `Int f;
              "args", `List (List.map json_of_expr args) ]

let rec json_of_stmt = function
  | Let (n, e) -> `Assoc [ "kind", `String "let"; "local", `Int n; "value", json_of_expr e ]
  | Seq stmts -> `Assoc [ "kind", `String "seq"; "items", `List (List.map json_of_stmt stmts) ]
  | If (c, t, f) ->
     `Assoc [ "kind", `String "if";
              "condition", json_of_expr c;
              "then", json_of_stmt t;
              "else", json_of_stmt f ]
  | Return e -> `Assoc [ "kind", `String "return"; "value", json_of_expr e ]

let json_of_helper (h: helper) =
  `Assoc [ "id", `Int h.id;
           "params", `List (List.map json_of_value_type h.params);
           "ret", json_of_value_type h.ret;
           "body", json_of_stmt h.body ]

let json_of_memory (m: memory_plan) =
  `Assoc [ "map", `String (match m.map_kind with Map1 -> "map1" | Map2 -> "map2");
           "output_index", `String "identity";
           "guard_before_load", `Bool m.guard_before_load;
           "workgroup_size", `Int m.workgroup_size ]

(** The semantic IR object. Byte identity is defined by
    [Yojson.Safe.to_string (to_yojson k)] — compact JSON, declared field order. *)
let to_yojson (k: kernel): Yojson.Safe.t =
  `Assoc [
    "entry", `String k.entry_name;
    "input_arity", `Int k.input_arity;
    "helpers", `List (List.map json_of_helper k.helpers);
    "body", json_of_stmt k.body;
    "memory", json_of_memory k.memory;
    "numeric_profile", `String (profile_name k.numeric_profile);
  ]

let canonical_bytes (k: kernel): string =
  Yojson.Safe.to_string (to_yojson k)

let sha256_hex (s: string): string =
  Digestif.SHA256.(to_hex (digest_string s))

let semantic_sha256 (k: kernel): string =
  sha256_hex (canonical_bytes k)
