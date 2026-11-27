(*
   Phase 6: CPS Compiler Integration Layer
   Purpose: Convert Monomorphic AST (Mtast) to CPS IR binary format
   
   This module bridges the Austral compiler's monomorphized AST (Mtast)
   with the CpsGen module, creating binary IR that can be compiled
   by the Cranelift JIT backend.
*)

open Stages
open CpsGen
open Identifier
open Id
open MonoType

module Mt = Mtast

(******************************************************************************)
(* UTILITY FUNCTIONS *)
(******************************************************************************)

let qident_to_string q =
  Identifier.qident_debug_name q

(** Does this type denote something that can be stored through — the same set
    `get_expr_type` treats as a dereference target? *)
let is_pointer_like (ty: mono_ty): bool =
  match ty with
  | MonoPointer _ | MonoAddress _ | MonoReadRef _ | MonoWriteRef _ -> true
  | _ -> false

let ends_with s suffix =
  let len_s = String.length s in
  let len_suffix = String.length suffix in
  len_s >= len_suffix && String.sub s (len_s - len_suffix) len_suffix = suffix

let mono_ty_to_cps_type (ty: mono_ty): CpsGen.cps_type =
  match ty with
  | MonoInteger (_, (Width8 | Width16 | Width32 | Width64 | WidthByteSize | WidthIndex)) -> I64
  | MonoBoolean -> Bool
  | MonoUnit -> Unit
  | MonoDoubleFloat -> F64
  | MonoSingleFloat -> F64
  | MonoAddress _ -> I64
  | MonoPointer _ -> I64
  | MonoSpan _ -> I64
  | MonoSpanMut _ -> I64
  | MonoReadRef _ -> I64
  | MonoWriteRef _ -> I64
  | MonoFnPtr _ -> I64
  | _ -> I64

let type_size (_ty: mono_ty): int = 8 (* All types currently 8 bytes in JIT *)

let record_layouts = Hashtbl.create 16

let union_layouts = Hashtbl.create 16

let foreign_functions = Hashtbl.create 16

(* Secondary lookup keyed by the foreign function's source identifier (e.g.
   "kernelVersion"). A cross-module call resolves to the *interface*
   declaration's decl_id, which differs from the *body* declaration that
   actually carries the `External_Name`. Keying by name as well lets such
   imported foreign calls still lower to their external symbol (e.g.
   "uk_version") instead of a qualified Austral name the JIT cannot resolve. *)
let foreign_functions_by_name = Hashtbl.create 16

(* U11 wiring: is this foreign signature carried by plain host words on both
   sides? Everything else (Bool, floats, integers narrower than 64 bits) is
   marshalled by the generated adapter wrapper, so the call must name the
   adapter symbol, not the native function. Word-native means: every
   parameter is an Int64/Nat64/byte buffer address or pointer, and the result
   is an Int64/Nat64. Direct word calls keep their existing symbol.

   A byte buffer is the exception: such a call is always mediated, even when
   every word in it is 64 bits wide and nothing needs marshalling. The wrapper
   is where the buffer protocol's guards live (a length that is negative or
   beyond `i64::MAX`, a null pointer with a non-zero length), so a direct call
   would be the one buffer path that dereferences a pointer the protocol has
   not checked. *)
let is_word_native (ty: mono_ty): bool =
  match ty with
  | MonoInteger (_, Width64) -> true
  | MonoAddress _ | MonoPointer _ | MonoSpan _ | MonoSpanMut _ -> true
  | _ -> false

(* Declared before `foreign_needs_adapter`, which forces mediation for these. *)
let is_buffer_param (ty: mono_ty): bool =
  match ty with
  | MonoAddress _ | MonoPointer _ | MonoSpan _ | MonoSpanMut _ -> true
  | _ -> false

(* Adapter mediation is the Rust-interop contract (`rs_*` imports, the ones
   with a `Rust_Ownership` pragma). Legacy C imports keep their declared symbol:
   their ABI is C's, and there is no generated wrapper to route to. *)
let is_rust_import (name: string): bool =
  String.length name >= 3 && String.sub name 0 3 = "rs_"

let foreign_needs_adapter (params: Mt.mvalue_parameter list) (rt: mono_ty): bool =
  List.exists (function Mt.MValueParameter (_, ty) -> is_buffer_param ty) params
  || not (
       List.for_all (function Mt.MValueParameter (_, ty) -> is_word_native ty) params
       && is_word_native rt)

let adapter_symbol (name: string): string = name ^ "__adapter"

(* U11: the adapters this module's calls require, keyed by the symbol the CPS
   names. Recorded while lowering, emitted as canonical descriptor text that
   `umbrella/src/interface.rs` parses to generate the wrapper. *)
type adapter_requirement = {
  ar_symbol: string;
  ar_target: string;
  ar_params: mono_ty list;
  ar_result: mono_ty;
  (* U15: `(failure tag, success tag)` when the declared result is the
     built-in `Option[T]` and the compiler selected the checked wrapper. The
     block names the tags so the wrapper never guesses the union's case
     order. *)
  ar_checked: (int * int) option;
  (* U15b: when the checked union carries a payload in *both* cases
     ([Either[L, R]]), the descriptor also names the union, its case names and
     the failure payload's scalar type: `(union, failure case, success case,
     failure payload)`. The wrapper's provider then writes `[tag, payload]`,
     because a tagless word cannot say which case a value belongs to. Absent
     for the `Option` shape, so its block is unchanged. *)
  ar_checked_union: (string * string * string * string) option;
}

(* U15: the call-site side of a checked Option import. The descriptor names
   `<symbol>__checked`, which returns the union pointer the JIT already
   represents (`tag` at offset 0, payload at offset 8), so the module's
   `match` reads a real tag and a refusal or panic is `None`. *)
type checked_option = {
  co_checked_symbol: string;
  co_payload: mono_ty;
  co_failure_tag: int;
  co_success_tag: int;
  (* `None` for the `Option` shape (the failure case is empty); `Some left`
     for `Either[L, R]`, whose `Left` payload the wrapper must encode and
     whose tag the module's `match` reads. *)
  co_failure_payload: mono_ty option;
  co_union_name: string;
  co_failure_case: string;
  co_success_case: string;
  (* U15c: the checked wrapper allocates the union it returns, so the generated
     crate also exports `<symbol>__free`. The `match` lowering releases the
     union after copying the tag and slots it dispatches on, which is what
     keeps the module from leaking one allocation per checked call. *)
  co_free_symbol: string;
}

let checked_option_adapters = Hashtbl.create 4

(* U15c: checked-adapter results live in a local between the call and the
   `match` that consumes them (the monomorphizer hoists the scrutinee into a
   temporary). Keyed by the JIT variable name: the `match` lowering releases the
   union and drops the entry, so one linear value has one release. *)
let checked_owned_locals : (string, string) Hashtbl.t = Hashtbl.create 4

(* U15c: a checked union passed as an argument leaves the caller's table: the
   callee's parameter owns it, and a `match` on that parameter in the callee
   releases it. Keyed by the callee the lowering emits (`d<decl>`/`m<mono>`),
   each entry is `(parameter index, free symbol)`. *)
let checked_owned_fn_params : (string, (int * string) list) Hashtbl.t =
  Hashtbl.create 8

(* A parameter two call sites transfer different providers' unions into has no
   single release symbol. Mark it so neither site records a transfer (the
   caller keeps the value, so a later `match` can still release it) instead of
   oscillating the transfer fixpoint between the two symbols. *)
let checked_param_conflicts : (string, int list) Hashtbl.t = Hashtbl.create 4

let parameter_transfer_conflicted (key: string) (index: int): bool =
  match Hashtbl.find_opt checked_param_conflicts key with
  | Some indices -> List.mem index indices
  | None -> false

(* Set when a conversion pass discovered a transfer it had not recorded, so
   `compile_module_cps` converts once more: a parameter that only becomes an
   owner in this pass can pass the value on again. *)
let checked_transfers_changed = ref false

let callee_decl_key (id: decl_id): string = "d" ^ show_decl_id id
let callee_mono_key (id: mono_id): string = "m" ^ show_mono_id id

(* The callees whose bodies this compilation emits. A transfer into one of
   them is recorded for its parameter (the callee's `match` releases the
   union); a call to any other callee — an import from another module, whose
   body a different front-end run lowered — is released by the *caller* after
   the call returns, because only the caller holds the `<symbol>__free` its
   allocation needs. Reset at the start of every `compile_module_cps`. *)
let emitted_callee_keys : (string, unit) Hashtbl.t = Hashtbl.create 16

(* Names for the argument/result bindings a caller-side release introduces.
   Reset per module so the artifact is byte-stable across runs. *)
let xfer_counter = ref 0

(* Set while converting the call a {!caller_release_setup} hoisted, so the
   expression path does not insist on (or perform) the release itself. *)
let xfer_hoisting = ref false




let fresh_xfer_name (what: string): string =
  let n = !xfer_counter in
  incr xfer_counter;
  Printf.sprintf "__xfer_%s%d" what n

(* A concrete instance method is emitted once, under the instance-method id,
   and every `MConcreteMethodCall` names it by that id. Keying transfers the
   same way is what lets the method's own `match` release a checked union its
   caller passed in, the same argument `callee_decl_key` covers. *)
let callee_meth_key (id: ins_meth_id): string = "i" ^ show_ins_meth_id id

let concrete_method_symbol (id: ins_meth_id): string =
  "concrete_method_" ^ show_ins_meth_id id

(* The concrete methods this module declares. Their bodies are emitted under
   {!concrete_method_symbol}, so their call sites must use that name. Every
   other concrete method (an imported instance, or a Pervasive intrinsic the
   bridge registers under its qualified name) keeps the name the lowering used
   before: the id form would not resolve. *)
let local_concrete_methods : (ins_meth_id, unit) Hashtbl.t = Hashtbl.create 8

let concrete_method_call_name (id: ins_meth_id) (q: qident): string =
  if Hashtbl.mem local_concrete_methods id then concrete_method_symbol id
  else qident_to_string q

(* The release an expression's value must be given with, and the local that
   owns it when the value is named. A cast is the same value (the CPS lowering
   drops casts), so it is looked through. *)
let rec checked_value_free (expr: Mt.mexpr): (string * string option) option =
  match expr with
  | Mt.MConcreteFuncall (decl_id, _, _, _) ->
      Option.map (fun co -> (co.co_free_symbol, None))
        (Hashtbl.find_opt checked_option_adapters decl_id)
  | Mt.MLocalVar (id, _) | Mt.MTemporary (id, _) | Mt.MParamVar (id, _) ->
      let name = ident_string id in
      Option.map (fun free -> (free, Some name))
        (Hashtbl.find_opt checked_owned_locals name)
  | Mt.MCast (e, _) | Mt.MTypecast (e, _) -> checked_value_free e
  | _ -> None

(* Record whether `name` now owns a checked-adapter union. Ownership moves: a
   copy of an owned variable (`let t = value`, the monomorphizer's hoisted
   scrutinee) makes `t` the owner and drops the source, so one linear value is
   released once. Any other value clears a previous ownership, so a rebound
   name is never released twice. *)
let register_checked_owned (name: string) (src: Mt.mexpr): unit =
  match checked_value_free src with
  | Some (free, owner) ->
    (match owner with
     | Some owner when owner <> name -> Hashtbl.remove checked_owned_locals owner
     | _ -> ());
    Hashtbl.replace checked_owned_locals name free
  | None -> Hashtbl.remove checked_owned_locals name

(* The release a `match` must emit for its scrutinee, and the local to forget
   when the value came through a variable. *)
let checked_scrutinee_free (expr: Mt.mexpr): (string * string option) option =
  checked_value_free expr

let required_adapters : (string, adapter_requirement) Hashtbl.t = Hashtbl.create 8

(* U13: the owned-value handles this module imports, keyed by target. The
   typing pass has already validated the pragma and renamed the import to the
   derived `<target>__dispose`, so the descriptor is a rendering of facts the
   typechecker checked, not a second reading of them. *)
type handle_requirement = {
  hr_target: string;
  hr_type_name: string;
  hr_constructor: string;
  hr_destructor: string;
  hr_params: int;
  hr_use: string option;
  hr_write: string option;
}

let required_handles : (string, handle_requirement) Hashtbl.t = Hashtbl.create 4

(* The typed external name of a handle import is `<target>__dispose`; the
   descriptor names the target itself, because `interface.rs` derives both
   wrapper symbols from it. *)
let handle_target_of_dispose_symbol (symbol: string): string =
  let suffix = "__dispose" in
  let ls = String.length symbol and lf = String.length suffix in
  if ls > lf && String.sub symbol (ls - lf) lf = suffix then
    String.sub symbol 0 (ls - lf)
  else symbol

(* The descriptor schema's vocabulary: Unit, Bool, Int32, Nat32, Int64, Nat64,
   Float32, Float64. Anything else cannot be described yet, and is an error at
   emission rather than a silently guessed layout. *)
let adapter_scalar_name (ty: mono_ty): string option =
  match ty with
  | MonoBoolean -> Some "Bool"
  | MonoInteger (Signed, Width32) -> Some "Int32"
  | MonoInteger (Unsigned, Width32) -> Some "Nat32"
  | MonoInteger (Signed, Width64) -> Some "Int64"
  | MonoInteger (Unsigned, Width64) -> Some "Nat64"
  | MonoSingleFloat -> Some "Float32"
  | MonoDoubleFloat -> Some "Float64"
  | _ -> None

let mono_ty_show (ty: mono_ty): string = MonoType.show_mono_ty ty

(* U12: a byte buffer parameter. `Address[Nat8]` is a mutable address, so its
   bytes cross as `readwrite`; `Pointer[Nat8]` is read-only, so they cross as
   `read`. Either way the buffer's length is the parameter immediately after
   it, a 64-bit integer, and the descriptor states both indices. Version 1 of
   the buffer transport carries buffers, their lengths and a scalar result;
   version 2 adds 64-bit integer words beside a buffer (the GPU service's
   `(context, buffer, offset, address, length)` calls). Anything else is
   refused here rather than described by a guess. *)
type buffer_parameter = {
  bp_mode: string;
  bp_index: int;
  bp_length: int;
}

(* A 64-bit integer word that is not a buffer's length. Version 2 names it in
   the descriptor's `scalars` line, so a wrapper cannot mistake it for a
   length: the two are the same width and only the line tells them apart. *)
type scalar_parameter = {
  sp_index: int;
  sp_name: string;
}

let buffer_length_name (ty: mono_ty): string =
  match ty with
  | MonoInteger (Signed, Width64) -> "Int64"
  | _ -> "Nat64"

let is_nat8 (ty: mono_ty): bool =
  match ty with
  | MonoInteger (Unsigned, Width8) -> true
  | _ -> false

let is_length_type (ty: mono_ty): bool =
  match ty with
  | MonoInteger ((Signed | Unsigned), Width64) -> true
  | _ -> false

let buffer_mode_of (ty: mono_ty): string option =
  match ty with
  | MonoAddress inner when is_nat8 inner -> Some "readwrite"
  | MonoPointer inner when is_nat8 inner -> Some "read"
  | _ -> None

(* The `buffers` and `lengths` lines of an `abi buffer 1` block, or the reason
   the signature cannot be described. Mirrors the admission rule in
   TypingPass.check_rust_ownership: an admitted buffer signature always has a
   descriptor, and a refused one never names an adapter. *)
let buffer_layout (req: adapter_requirement)
  : (buffer_parameter list * scalar_parameter list * mono_ty, string) result =
  let params = Array.of_list req.ar_params in
  let n = Array.length params in
  let problem what =
    Error (Printf.sprintf "adapter for %s cannot be described: %s" req.ar_symbol what)
  in
  let needs_length i ty =
    problem (Printf.sprintf
               "parameter %d (%s) is a byte buffer but the parameter after it is not \
                its length; a buffer's length is a 64-bit integer (Nat64 or Int64) \
                directly after the buffer"
               i (mono_ty_show ty))
  in
  let rec go i acc scalars lengths =
    if i >= n then
      match List.rev lengths with
      | [] -> problem "the signature names no byte buffer"
      | first :: rest ->
        (match List.find_opt (fun ty -> ty <> first) rest with
         | Some other ->
           problem (Printf.sprintf
                      "its buffer lengths are %s and %s; one descriptor states one \
                       `lengths` type, so every length parameter must have the same \
                       64-bit integer type"
                      (mono_ty_show first) (mono_ty_show other))
         | None -> Ok (List.rev acc, List.rev scalars, first))
    else
      let ty = params.(i) in
      match buffer_mode_of ty with
      | Some mode ->
        if i + 1 >= n then needs_length i ty
        else if not (is_length_type params.(i + 1)) then needs_length i ty
        else
          go (i + 2)
            ({ bp_mode = mode; bp_index = i; bp_length = i + 1 } :: acc)
            scalars
            (params.(i + 1) :: lengths)
      | None ->
        if is_length_type ty then
          (* Version 2: a 64-bit integer that does not measure a buffer is a
             word the wrapper passes through unchanged. The descriptor names
             it, so the wrapper never treats it as a length. *)
          go (i + 1)
            acc
            ({ sp_index = i; sp_name = buffer_length_name ty } :: scalars)
            lengths
        else
          problem (Printf.sprintf
                     "parameter %d (%s) is neither one of the signature's Nat8 \
                      buffers, a buffer's 64-bit length, nor a 64-bit integer \
                      word; buffer transport version 2 carries byte buffers, \
                      their lengths, 64-bit words and a scalar result only"
                     i (mono_ty_show ty))
  in
  go 0 [] [] []

(* One descriptor block, canonical field order. Buffered signatures become
   `abi buffer 1` blocks; scalar-only ones keep their U11 `abi scalar 1` form. *)
let adapter_block (req: adapter_requirement): (string, string) result =
  let scalar_result () =
    match adapter_scalar_name req.ar_result with
    | Some name -> Ok name
    | None ->
      Error (Printf.sprintf
               "adapter for %s cannot be described: the result type %s is not a \
                scalar (Unit, Bool, Int32, Nat32, Int64, Nat64, Float32, \
                Float64), and the descriptor's `returns` line is a scalar"
               req.ar_symbol (mono_ty_show req.ar_result))
  in
  if List.exists is_buffer_param req.ar_params then
    match buffer_layout req with
    | Error e -> Error e
    | Ok (buffers, scalars, length_ty) ->
      (match scalar_result () with
       | Error e -> Error e
       | Ok returns ->
         let buffers =
           String.concat ","
             (List.map
                (fun b -> Printf.sprintf "%s:%d:%d" b.bp_mode b.bp_index b.bp_length)
                buffers)
         in
         (* Version 2 only when a word actually sits beside a buffer, so every
            signature version 1 describes keeps its byte-identical block. *)
         let abi_version = if scalars = [] then 1 else 2 in
         let scalars_line =
           match scalars with
           | [] -> []
           | _ ->
             [ "scalars "
               ^ String.concat ","
                   (List.map
                      (fun s -> Printf.sprintf "%d:%s" s.sp_index s.sp_name)
                      scalars) ]
         in
         Ok (String.concat "\n"
               ([ "schema 1";
                  Printf.sprintf "abi buffer %d" abi_version;
                  Printf.sprintf "symbol %s" req.ar_symbol;
                  Printf.sprintf "target %s" req.ar_target;
                  Printf.sprintf "params %d" (List.length req.ar_params);
                  Printf.sprintf "lengths %s" (buffer_length_name length_ty);
                  Printf.sprintf "buffers %s" buffers ]
                @ scalars_line
                @ [ "empty null-len0";
                    Printf.sprintf "returns %s" returns ])
              ^ "\n"))
  else
    match scalar_result () with
    | Error e -> Error e
    | Ok returns ->
      let unrepresentable =
        List.find_opt (fun ty -> adapter_scalar_name ty = None) req.ar_params
      in
      (match unrepresentable with
       | Some ty ->
         Error (Printf.sprintf
                  "adapter for %s cannot be described: parameter type %s is not a \
                   scalar (Unit, Bool, Int32, Nat32, Int64, Nat64, Float32, \
                   Float64)"
                  req.ar_symbol (mono_ty_show ty))
       | None ->
         let names =
           List.map
             (fun ty -> match adapter_scalar_name ty with Some n -> n | None -> assert false)
             req.ar_params
         in
         let status_lines =
           match req.ar_checked, req.ar_checked_union with
           | None, _ -> ""
           | Some (failure, success), None ->
             Printf.sprintf "status checked\nfailure %d\nsuccess %d\n"
               failure success
           | Some (failure, success), Some (union, fcase, scase, fpayload) ->
             Printf.sprintf
               "status checked\nfailure %d\nsuccess %d\nunion %s\nfailure-case \
                %s\nsuccess-case %s\nfailure-payload %s\n"
               failure success union fcase scase fpayload
         in
         Ok (Printf.sprintf
               "schema 1\nabi scalar 1\nsymbol %s\ntarget %s\nparams %s\nreturns \
                %s\n%s"
               req.ar_symbol req.ar_target (String.concat "," names) returns
               status_lines))

(* Canonical text: fixed field order, one fact per line, descriptors sorted by
   symbol, blocks separated by a blank line. Regenerating is byte-identical.
   A block is `abi buffer 1` when its signature carries a byte buffer and
   `abi scalar 1` otherwise; the reader keeps both kinds in one document. *)
(* U13: one `abi handle 1` block, in the canonical field order
   `HandleAdapter::descriptor_text` uses in `umbrella/src/interface.rs`. The
   registry tag is not written: the generator numbers registries when it reads
   the document. *)
let handle_block (req: handle_requirement): string =
  (* `use`/`write` are optional and sit where `HandleAdapter::descriptor_text`
     puts them: after the destructor, in that order, before `params`. A
     provider without a reader still gets the constructor and destructor
     wrappers, and one without a writer gets no mutation wrapper. *)
  let use_line =
    match req.hr_use with
    | Some accessor -> Printf.sprintf "use %s\n" accessor
    | None -> ""
  in
  let write_line =
    match req.hr_write with
    | Some writer -> Printf.sprintf "write %s\n" writer
    | None -> ""
  in
  Printf.sprintf
    "schema 1\nabi handle 1\nsymbol %s__new\ntarget %s\ntype %s\nconstructor %s\n\
     destructor %s\n%s%sparams %d\n"
    req.hr_target req.hr_target req.hr_type_name req.hr_constructor
    req.hr_destructor use_line write_line req.hr_params

let adapter_descriptors_text (): (string, string) result =
  let items =
    Hashtbl.fold (fun _ req acc -> req :: acc) required_adapters []
    |> List.sort (fun a b -> String.compare a.ar_symbol b.ar_symbol)
  in
  let rec describe acc = function
    | [] -> Ok (List.rev acc)
    | req :: rest ->
      (match adapter_block req with
       | Error e -> Error e
       | Ok text -> describe (text :: acc) rest)
  in
  match describe [] items with
  | Error e -> Error e
  | Ok blocks ->
     let buf = Buffer.create 256 in
     Buffer.add_string buf
       "# U11/U12 adapter descriptors, emitted by the compiler's CPS lowering.\n";
     Buffer.add_string buf
       "# Canonical form: fixed field order, one fact per line, descriptors\n";
     Buffer.add_string buf
       "# sorted by symbol, blocks separated by a blank line.\n";
     Buffer.add_string buf
       "# `umbrella/src/interface.rs` parses this text.\n";
     List.iter
       (fun text ->
         Buffer.add_string buf "\n";
         Buffer.add_string buf text)
       blocks;
     (* Handle blocks follow the scalar/buffer ones, sorted by target, so the
        document is byte-identical across runs and one registry tag per target
        is stable. *)
     Hashtbl.fold (fun _ req acc -> req :: acc) required_handles []
     |> List.sort (fun a b -> String.compare a.hr_target b.hr_target)
     |> List.iter (fun req ->
         Buffer.add_string buf "\n";
         Buffer.add_string buf (handle_block req));
     Ok (Buffer.contents buf)

let write_adapter_descriptors (path: string): (unit, string) result =
  match adapter_descriptors_text () with
  | Error e -> Error e
  | Ok text ->
     (try
        let oc = open_out path in
        output_string oc text;
        close_out oc;
        Ok ()
      with Sys_error e -> Error e)

let is_foreign_function id =
  Hashtbl.mem foreign_functions id

let foreign_external_name id =
  match Hashtbl.find_opt foreign_functions id with
  | Some name -> name
  | None -> ""

let rec find_slot_offset slots target_name current_offset =
  match slots with
  | [] -> 0
  | MonoSlot (name, ty) :: rest ->
      if Identifier.ident_string name = Identifier.ident_string target_name then
        current_offset
      else
        find_slot_offset rest target_name (current_offset + type_size ty)

let rec find_union_case_offset cases target_tag_name current_offset =
  match cases with
  | [] -> 0
  | MonoCase (name, _) :: rest ->
      if Identifier.ident_string name = Identifier.ident_string target_tag_name then
        current_offset
      else
        find_union_case_offset rest target_tag_name (current_offset + 1)

(* U15: an `rs_*` import whose declared result is the built-in `Option[T]` is
   the one shape the compiler can hand back as a tagged union: `None` has no
   payload and `Some` carries one scalar. Anything else keeps the bare payload
   word. The tags are the union's own indices (case order is `None`, `Some` in
   the pervasive declaration), read from the layout the `match` lowering uses. *)
(* The layout of `Option[T]`, or `None` when these cases are not that shape.
   Only `None` (no payload) and `Some` (one payload) qualify; a union with the
   same case names but another shape is not the builtin. *)
let option_layout cases =
  let case name =
    List.find_opt
      (function MonoCase (n, _) -> Identifier.ident_string n = name)
      cases
  in
  match case "None", case "Some" with
  | Some (MonoCase (_, [])), Some (MonoCase (_, [MonoSlot (_, payload)])) ->
    (match adapter_scalar_name payload with Some _ -> Some payload | None -> None)
  | _ -> None

let checked_option_of ~symbol ~result : checked_option option =
  let cases_for id =
    match Hashtbl.find_opt union_layouts id with
    | Some cases -> Some cases
    | None ->
      (* A foreign signature can name a union monomorph whose definition was
         emitted in the module that exports the interface, so this module's
         `MUnionMonomorph` declarations do not hold that id. The shape is still
         decidable when exactly one `Option`-shaped layout exists; with two
         payload types the choice would be a guess, so it stays undescribed (an
         error) instead. *)
      let candidates =
        Hashtbl.fold
          (fun _ cases acc ->
            match option_layout cases with Some _ -> cases :: acc | None -> acc)
          union_layouts []
      in
      (match candidates with [cases] -> Some cases | _ -> None)
  in
  match result with
  | MonoNamedType id ->
    (match cases_for id with
     | None -> None
     | Some cases ->
       (match option_layout cases with
        | None -> None
        | Some payload ->
          let case name =
            List.find (fun (MonoCase (n, _)) -> Identifier.ident_string n = name) cases
          in
          let none = case "None" and some = case "Some" in
          (match none, some with
           | MonoCase (n, []), MonoCase (s, _) ->
             Some {
               co_checked_symbol = symbol ^ "__checked";
               co_payload = payload;
               co_failure_tag = find_union_case_offset cases n 0;
               co_success_tag = find_union_case_offset cases s 0;
               co_failure_payload = None;
               co_union_name = "Option";
               co_failure_case = Identifier.ident_string n;
               co_success_case = Identifier.ident_string s;
               co_free_symbol = symbol ^ "__free";
             }
           | _ -> None)))
  | _ -> None

(* The typer resolved the union declaration and recorded the checked-wrapper
   decision; the payload it admitted is one of the adapter scalars, so its mono
   form is total here. *)
let mono_ty_of_checked_payload (ty: Type.ty): mono_ty option =
  match ty with
  | Type.Boolean -> Some MonoBoolean
  | Type.Integer (signedness, width) -> Some (MonoInteger (signedness, width))
  | Type.SingleFloat -> Some MonoSingleFloat
  | Type.DoubleFloat -> Some MonoDoubleFloat
  | _ -> None

let find_record_slot_offset id name =
  match Hashtbl.find_opt record_layouts id with
  | Some slots -> find_slot_offset slots name 0
  | None -> 0

(** The record a field access is really about.

    The typer hands a field access through a borrow the type of the borrow
    *temp* it introduced, and a re-borrow nests: `read(hot: &[Hot, R])`
    accessing `hot->hi` sees `&&Hot`, because the CPS local that carries the
    parameter is itself a read reference. Stripping one layer left a
    `MonoReadRef`, which is not a `MonoNamedType`, so every field of the record
    resolved to offset 0 — `hi` silently read `lo`'s word. Strip to the
    pointee, however many borrows the front-end wrapped around it. *)
let rec pointee_type (ty: mono_ty): mono_ty =
  match ty with
  | MonoPointer inner | MonoAddress inner
  | MonoReadRef (inner, _) | MonoWriteRef (inner, _) -> pointee_type inner
  | ty -> ty

let rec get_expr_type (expr: Mt.mexpr): mono_ty =
  match expr with
  | Mt.MNilConstant -> MonoUnit
  | Mt.MBoolConstant _ -> MonoBoolean
  | Mt.MIntConstant _ -> MonoInteger (Signed, Width64)
  | Mt.MFloatConstant _ -> MonoDoubleFloat
  | Mt.MStringConstant _ -> MonoPointer (MonoInteger (Unsigned, Width8))
  | Mt.MConstVar (_, ty) -> ty
  | Mt.MParamVar (_, ty) -> ty
  | Mt.MLocalVar (_, ty) -> ty
  | Mt.MTemporary (_, ty) -> ty
  | Mt.MGenericFunVar (_, ty) -> ty
  | Mt.MConcreteFunVar (_, ty) -> ty
  | Mt.MConcreteFuncall (_, _, _, ty) -> ty
  | Mt.MGenericFuncall (_, _, ty) -> ty
  | Mt.MConcreteMethodCall (_, _, _, ty) -> ty
  | Mt.MGenericMethodCall (_, _, _, ty) -> ty
  | Mt.MFptrCall (_, _, ty) -> ty
  | Mt.MCast (_, ty) -> ty
  | Mt.MComparison _ -> MonoBoolean
  | Mt.MConjunction _ -> MonoBoolean
  | Mt.MDisjunction _ -> MonoBoolean
  | Mt.MNegation _ -> MonoBoolean
  | Mt.MIfExpression (_, then_e, _) -> get_expr_type then_e
  | Mt.MRecordConstructor (ty, _) -> ty
  | Mt.MUnionConstructor (ty, _, _) -> ty
  | Mt.MEmbed (ty, _, _) -> ty
  | Mt.MDeref e -> (match get_expr_type e with MonoPointer ty | MonoAddress ty | MonoReadRef (ty, _) | MonoWriteRef (ty, _) -> ty | _ -> MonoUnit)
  | Mt.MTypecast (_, ty) -> ty
  | Mt.MSizeOf _ -> MonoInteger (Unsigned, Width64)
  | Mt.MSlotAccessor (_, _, ty) -> ty
  | Mt.MPointerSlotAccessor (_, _, ty) -> ty
  | Mt.MArrayIndex (e, _, _) -> (match get_expr_type e with MonoPointer ty | MonoAddress ty | MonoSpan (ty, _) | MonoSpanMut (ty, _) -> ty | _ -> MonoUnit)
  | Mt.MSpanIndex (e, _, _) -> (match get_expr_type e with MonoPointer ty | MonoAddress ty | MonoSpan (ty, _) | MonoSpanMut (ty, _) -> ty | _ -> MonoUnit)

(* U15c: when an argument of union type is a checked adapter's result (or a
   local that owns one), ownership moves to the callee's parameter. Record the
   transfer so the callee's `match` releases the union, and forget the
   caller's local: one linear value is released once. A borrowed value has a
   reference type and a copied word owns nothing, so neither is transferred. *)
(* A call whose transfer must be released by this compilation: a non-foreign
   callee whose body is not emitted here, with a checked-union argument and a
   non-union result. *)
let caller_release_needed (args: Mt.mexpr list) (ret: mono_ty): bool =
  (match ret with MonoNamedType _ -> false | _ -> true)
  && List.exists
       (fun arg ->
         match get_expr_type arg, checked_value_free arg with
         | MonoNamedType _, Some _ -> true
         | _ -> false)
       args

let refuse_unsequenced_release (): unit =
  Error.austral_raise Error.GenericError [
    ErrorText.Text
      "A checked union is passed to a function whose body this compilation does \
       not emit, so the caller must release it after the call returns. This call \
       is inside an expression that cannot sequence the release; bind it to a \
       local first (`let r = f(value);`) or use it as a statement (return, \
       discard, assignment).";
    ErrorText.Break;
    ErrorText.Text "No CPS artifact was written for this module." ]

let note_checked_argument_transfer (key: string) (args: Mt.mexpr list): unit =
  List.iteri
    (fun index arg ->
      match get_expr_type arg with
      | MonoNamedType _ ->
        let transferred = checked_value_free arg in
        (match transferred with
         | None -> ()
         | Some (free, local) ->
           if not (parameter_transfer_conflicted key index) then begin
             let entries =
               match Hashtbl.find_opt checked_owned_fn_params key with
               | Some entries -> entries
               | None -> []
             in
             (match List.assoc_opt index entries with
              | Some existing when existing = free -> ()
              | Some _ ->
                (* Two providers, one parameter: keep only the caller's
                   ownership. *)
                Hashtbl.replace checked_param_conflicts key
                  (index ::
                   match Hashtbl.find_opt checked_param_conflicts key with
                   | Some indices -> indices
                   | None -> []);
                Hashtbl.replace checked_owned_fn_params key
                  (List.remove_assoc index entries);
                checked_transfers_changed := true
              | None ->
                Hashtbl.replace checked_owned_fn_params key
                  ((index, free) :: entries);
                checked_transfers_changed := true);
             (match local with
              | Some name -> Hashtbl.remove checked_owned_locals name
              | None -> ())
           end)
      | _ -> ())
    args

(** `a - b` / `a * b` reach us as calls to the Pervasive trapping methods
    (TypeCheckExpr.augment_arithmetic), and the JIT has had `sub`/`mul` opcodes
    all along — unused, because a call was simpler. Emit the instruction now so
    the arithmetic layer is measurable and refinable; the JIT implements both
    opcodes with the same trap-on-overflow contract as the methods (`cps.rs`),
    so this is not a semantic change. `+` deliberately keeps its call: 0x05
    doubles as the `for`-loop increment, and leaving it alone keeps that path
    unchanged. *)
let trapping_arith_op (q: qident): [ `Sub | `Mul ] option =
  let name = qident_to_string q in
  if ends_with name "trappingSubtract" then Some `Sub
  else if ends_with name "trappingMultiply" then Some `Mul
  else None

(* Float arithmetic must not reach the integer opcodes below: `Sub`/`Mul` on an
   f64 word would subtract/multiply two IEEE bit patterns as integers (and trap
   on "overflow" for perfectly ordinary sums), and the `trapping*` symbols the
   fallback emits are the Int64 implementations. The mono AST's method-call node
   carries the call's result type, which is `MonoDoubleFloat` exactly for the
   Float64 instance of TrappingArithmetic, so the four operations are routed to
   the bridge's float primitives instead. They take and return the f64 bits in
   one word, like every other value on this transport.

   Both widths are routed: a Float32 word is narrowed, so it is widened to f64
   (`au_f32_to_f64`), operated on, and narrowed again (`au_f64_to_f32`) — see
   `float_arith_call` below. *)
let float_arith_op (q: qident): string option =
  let name = qident_to_string q in
  if ends_with name "trappingAdd" then Some "add"
  else if ends_with name "trappingSubtract" then Some "subtract"
  else if ends_with name "trappingMultiply" then Some "multiply"
  else if ends_with name "trappingDivide" then Some "divide"
  else None

(* The Float32 form of the same call: a Float32 word is the f32 bits
   zero-extended (the adapter ABI refuses a word with bits above 31), and the
   bridge has no f32 arithmetic, so both operands are widened to f64, the f64
   primitive runs, and the result is narrowed back to f32 bits. Each step is a
   grant-free `au_` symbol, so the JIT sees one ordinary call chain. *)
let float_arith_call (q: qident) (ty: mono_ty) (a: CpsGen.cps_expr)
    (b: CpsGen.cps_expr): CpsGen.cps_expr option =
  match float_arith_op q with
  | None -> None
  | Some op -> (
      match ty with
      | MonoDoubleFloat -> Some (App ("au_f64_" ^ op, [a; b]))
      | MonoSingleFloat ->
          let widen e = App ("au_f32_to_f64", [e]) in
          Some
            (App ("au_f64_to_f32", [App ("au_f64_" ^ op, [widen a; widen b])]))
      | _ -> None)

(* A float literal has no width in the mono AST (`Mtast.MFloatConstant` is a
   string), so it crosses as f64 bits by default. Where the destination's
   declared type is `Float32` the word has to be the f32 bits instead: the
   scalar ABI's `F32` decode refuses a word with anything above bit 31, and the
   JIT has no float arithmetic that could narrow the value later. Only a
   *literal* is narrowed — a Float32 that came back from a foreign call is
   already the target's f32 word.

   Three places know the destination's type, so three tables are filled before
   conversion: a callee's parameters (a literal written straight into an
   argument position), a local's declared type (from `MLet`), and the enclosing
   function's return type. *)
let function_params : (decl_id, mono_ty list) Hashtbl.t = Hashtbl.create 32

(* A function *used as a value* (`TFunVar`) is lowered to a `Var` that names
   the emitted function, so the JIT can hand its address to a foreign call
   (the `__with` callback). The name has to be the one the function is emitted
   under — `ident_string name` — not the numeric decl id, or the variable
   never resolves. Filled in the same first pass as `function_params`. *)
let function_value_names : (decl_id, string) Hashtbl.t = Hashtbl.create 32

let monomorph_params : (mono_id, mono_ty list) Hashtbl.t = Hashtbl.create 32

let local_types : (string, mono_ty) Hashtbl.t = Hashtbl.create 32

let current_return_type : mono_ty ref = ref MonoUnit

let param_types (params: Mt.mvalue_parameter list): mono_ty list =
  List.map (function Mtast.MValueParameter (_, ty) -> ty) params

let local_type (q: qident): mono_ty =
  match Hashtbl.find_opt local_types (ident_string (original_name q)) with
  | Some ty -> ty
  | None -> MonoUnit

(* The byte size of a type, for `sizeof`. Only the scalar and pointer types
   have a width the JIT model knows; anything else keeps the old placeholder
   of 0 rather than inventing a layout the runtime does not implement. *)
let size_of_mono_ty (ty: mono_ty): int =
  match ty with
  | MonoUnit | MonoRegionTy _ | MonoRegionTyVar _ -> 0
  | MonoBoolean | MonoInteger (_, Width8) -> 1
  | MonoInteger (_, Width16) -> 2
  | MonoInteger (_, Width32) | MonoSingleFloat -> 4
  | MonoInteger (_, (Width64 | WidthByteSize | WidthIndex))
  | MonoDoubleFloat | MonoPointer _ | MonoAddress _ -> 8
  | MonoNamedType _ | MonoReadRef _ | MonoWriteRef _ | MonoSpan _
  | MonoSpanMut _ | MonoFnPtr _ -> 0

(* Comparisons have the same problem as arithmetic, in a quieter form: `Cmp*`
   are `icmp` on the word, so on an f64 they order its IEEE *bit pattern*.
   That is right for positive values and reversed between two negatives
   (`-2.0` = 0xC000… sorts above `-1.0` = 0xBFF0… as a *signed* word, the
   opposite of their values), `Eq` calls `-0.0` unequal to `0.0`, and a NaN
   compares equal to itself. The operand's mono type says which world the
   comparison is in;
   a Float32 pair is widened to f64 first, which is exact — every f32 is an
   f64, order and NaN payload included — so one set of primitives serves both
   widths, as with the arithmetic above. *)
let float_compare_call (op: Common.comparison_operator) (ty: mono_ty)
    (a: CpsGen.cps_expr) (b: CpsGen.cps_expr): CpsGen.cps_expr option =
  let open Common in
  let sym =
    match op with
    | LessThan -> Some "au_f64_lt"
    | GreaterThan -> Some "au_f64_gt"
    | LessThanOrEqual -> Some "au_f64_le"
    | GreaterThanOrEqual -> Some "au_f64_ge"
    | Equal -> Some "au_f64_eq"
    | NotEqual -> Some "au_f64_ne"
  in
  match sym with
  | None -> None
  | Some sym -> (
      match ty with
      | MonoDoubleFloat -> Some (App (sym, [a; b]))
      | MonoSingleFloat ->
          let widen e = App ("au_f32_to_f64", [e]) in
          Some (App (sym, [widen a; widen b]))
      | _ -> None)

(* The type of the *comparison*, which is not always `get_expr_type e1`: a
   float literal is typed f64 (it has no width in the mono AST) even when the
   other operand is a Float32, so the narrower operand wins — the typechecker
   requires equal operands, so a MonoSingleFloat on either side means the
   comparison is a Float32 one. *)
let comparison_type (e1: Mt.mexpr) (e2: Mt.mexpr): mono_ty =
  let t1 = get_expr_type e1 and t2 = get_expr_type e2 in
  match t1, t2 with
  | MonoSingleFloat, _ | _, MonoSingleFloat -> MonoSingleFloat
  | MonoDoubleFloat, _ | _, MonoDoubleFloat -> MonoDoubleFloat
  | _ -> t1

let rec convert_expr (expr: Mt.mexpr): CpsGen.cps_expr =
  match expr with
  | Mt.MNilConstant -> IntLit 0L
  | Mt.MBoolConstant b -> BoolLit b
  | Mt.MIntConstant s ->
      (* Not `Int64.of_string` with a zero default: a `Nat64` literal above
         2^63-1 is a legal word, and answering 0 for it silently changed the
         program's constant. Out-of-word literals raise instead. *)
      IntLit (Util.int_literal_word s)
  | Mt.MFloatConstant s ->
      (try FloatLit (float_of_string s)
       with _ -> FloatLit 0.0)
  (* escaped_to_string yields the real processed bytes; unescape_string would
     re-insert C-format escape sequences for quotes/newlines, corrupting the
     embedded buffer (a JSON literal would gain stray backslashes). *)
  | Mt.MStringConstant s -> StringLit (Escape.escaped_to_string s)
  | Mt.MConstVar (q, _) -> 
      let name = qident_to_string q in
      if ends_with name ":ExitSuccess" || ends_with name ".ExitSuccess" || name = "ExitSuccess" then
        App ("au_exit", [IntLit 0L])
      else
        (* A module-level `constant` is compiled to a zero-argument function
           under its unqualified name (see `Mt.MConstant` below), and the JIT
           has no global variables, so a reference is a call. A qualified
           `Var` could never resolve: `0x02` reads a local slot. *)
        App (ident_string (original_name q), [])
  | Mt.MParamVar (id, _) -> Var (ident_string id)
  | Mt.MLocalVar (id, _) -> Var (ident_string id)
  | Mt.MTemporary (id, _) -> Var (ident_string id)
  | Mt.MGenericFunVar (id, _) -> Var ("monomorph_" ^ show_mono_id id)
  | Mt.MConcreteFunVar (id, _) ->
      (match Hashtbl.find_opt function_value_names id with
       | Some name -> Var name
       | None -> Var ("fun_" ^ show_decl_id id))
  | Mt.MConcreteFuncall (decl_id, q, args, ty) ->
      let name = qident_to_string q in
      let is_exit_success = 
        ends_with name ":ExitSuccess" || 
        ends_with name ".ExitSuccess" ||
        name = "ExitSuccess" 
      in
      if is_exit_success then
        App ("au_exit", [IntLit 0L])
      else if Hashtbl.mem checked_option_adapters decl_id then
        (* The compiler selected the checked wrapper for this import: it returns
           the Option union directly, so the call-site type and the `match`
           lowering already agree and no out slot or status branch is needed. *)
        App
          ( (Hashtbl.find checked_option_adapters decl_id).co_checked_symbol,
            narrow_args_for decl_id args )
      else if Hashtbl.mem foreign_functions decl_id then
        App (Hashtbl.find foreign_functions decl_id, narrow_args_for decl_id args)
      else if Hashtbl.mem foreign_functions_by_name (ident_string (original_name q)) then
        App (Hashtbl.find foreign_functions_by_name (ident_string (original_name q)),
             narrow_args_for decl_id args)
      else begin
        (* Non-foreign call, possibly cross-module (e.g. a module calling
           UnferKernel::wrapModel). Functions are emitted under their *unqualified*
           name (build_cps_function uses `ident_string name`) into a flat JIT
           function table, so the call must use the unqualified name too. The
           qualified debug name ("UnferKernel::wrapModel") would not resolve. *)
        let key = callee_decl_key decl_id in
        if Hashtbl.mem emitted_callee_keys key then
          note_checked_argument_transfer key args
        else if (not !xfer_hoisting) && caller_release_needed args ty then
          refuse_unsequenced_release ();
        (match args with
         | [a; b] ->
             let ca = convert_expr a and cb = convert_expr b in
             (match float_arith_call q ty ca cb with
              | Some e -> e
              | None ->
                (match trapping_arith_op q with
                 | Some `Sub -> Sub (ca, cb)
                 | Some `Mul -> Mul (ca, cb)
                 | None -> App (ident_string (original_name q), [ca; cb])))
         | _ -> App (ident_string (original_name q), List.map convert_expr args))
      end
  | Mt.MGenericFuncall (id, args, ty) ->
      let key = callee_mono_key id in
      if Hashtbl.mem emitted_callee_keys key then
        note_checked_argument_transfer key args
      else if (not !xfer_hoisting) && caller_release_needed args ty then
        refuse_unsequenced_release ();
      let args =
        match Hashtbl.find_opt monomorph_params id with
        | Some tys when List.length tys = List.length args ->
            List.map2 narrow_float_literal tys args
        | _ -> List.map convert_expr args
      in
      App ("monomorph_" ^ show_mono_id id, args)
  | Mt.MConcreteMethodCall (id, q, args, ty) ->
      let key = callee_meth_key id in
      if Hashtbl.mem emitted_callee_keys key then
        note_checked_argument_transfer key args
      else if (not !xfer_hoisting) && caller_release_needed args ty then
        refuse_unsequenced_release ();
      (match args with
       | [a; b] ->
           let ca = convert_expr a and cb = convert_expr b in
           (match float_arith_call q ty ca cb with
            | Some e -> e
            | None ->
              (match trapping_arith_op q with
               | Some `Sub -> Sub (ca, cb)
               | Some `Mul -> Mul (ca, cb)
               | None -> App (concrete_method_call_name id q, [ca; cb])))
       | _ -> App (concrete_method_call_name id q, List.map convert_expr args))
  | Mt.MGenericMethodCall (_, id, args, ty) ->
      let key = callee_mono_key id in
      if Hashtbl.mem emitted_callee_keys key then
        note_checked_argument_transfer key args
      else if (not !xfer_hoisting) && caller_release_needed args ty then
        refuse_unsequenced_release ();
      App ("method_" ^ show_mono_id id, List.map convert_expr args)
  | Mt.MFptrCall (id, args, _) ->
      App (ident_string id, List.map convert_expr args)
  | Mt.MCast (e, _) -> convert_expr e
  | Mt.MComparison (op, e1, e2) ->
      let ty = comparison_type e1 e2 in
      let left = narrow_float_literal ty e1
      and right = narrow_float_literal ty e2 in
      (match float_compare_call op ty left right with
       | Some e -> e
       | None ->
         (match op with
          | LessThan -> CmpLt (left, right)
          | GreaterThan -> CmpGt (left, right)
          | LessThanOrEqual -> CmpLte (left, right)
          | GreaterThanOrEqual -> CmpGte (left, right)
          | Equal -> CmpEq (left, right)
          | NotEqual -> CmpNeq (left, right)))
  | Mt.MConjunction (e1, e2) -> And (convert_expr e1, convert_expr e2)
  | Mt.MDisjunction (e1, e2) -> Or (convert_expr e1, convert_expr e2)
  | Mt.MNegation e -> Not (convert_expr e)
  | Mt.MIfExpression (cond, then_e, else_e) ->
      App ("__if_expr", [convert_expr cond; convert_expr then_e; convert_expr else_e])
  | Mt.MRecordConstructor (_ty, fields) ->
      (* Every slot is 8 bytes in the JIT, and the constructor stores exactly one
         value per field, so the field count is the authoritative size. Don't rely
         on record_layouts here: concrete (non-generic) records arrive as `MRecord`
         keyed by decl_id, which the layout pass does not register, so the old
         lookup silently fell back to 8 bytes and under-allocated multi-field
         records (heap corruption). *)
      let size = max (List.length fields * 8) 8 in
      App ("__record_new", IntLit (Int64.of_int size) :: List.map (fun (_, e) -> convert_expr e) fields)
  | Mt.MUnionConstructor (ty, tag, fields) ->
      let (tag_idx, size) = match ty with
        | MonoNamedType id ->
            (match Hashtbl.find_opt union_layouts id with
             | Some cases -> 
                 (* find_union_case_offset compares identifiers, pass raw tag *)
                 let idx = find_union_case_offset cases tag 0 in
                 let max_fields = List.fold_left (fun acc (MonoCase (_, s)) -> max acc (List.length s)) 0 cases in
                 (idx, (max_fields + 1) * 8)
             | None -> (0, 16))
        | _ -> (0, 16)
      in
      App ("__union_new", IntLit (Int64.of_int size) :: IntLit (Int64.of_int tag_idx) :: List.map (fun (_, e) -> convert_expr e) fields)
  | Mt.MEmbed (_, code, args) ->
      let cargs = List.map convert_expr args in
      (* Recognise the Span field accessors used by the standard library and the
         kernel bindings. A Span is represented in the JIT as a pointer to a heap
         {data@0, size@8} struct (see the 0x21 string-constant lowering in
         cps.rs), so `$1.data`/`$1.size` are slot loads. *)
      (match String.trim code, cargs with
       | "$1.data", [a] -> App ("__slot_get", [a; IntLit 0L])
       | "$1.size", [a] -> App ("__slot_get", [a; IntLit 8L])
       | "NULL", [] -> IntLit 0L
       | "$1", [a] -> a
       (* The memory intrinsics the linear buffer wrappers use. Both are real
          runtime symbols; lowering them as calls keeps the `au_calloc`
          zeroing and the `au_free` null check rather than re-implementing
          them in the lowering. *)
       | "au_calloc($1, $2)", [elem; count] -> App ("au_calloc", [elem; count])
       | "au_free($1)", [a] -> App ("au_free", [a])
       | _ -> App ("__embed", cargs))
  | Mt.MDeref (Mt.MSlotAccessor (e, name, ty)) ->
      (* A record value's field read arrives here as a dereference of the slot
         accessor, because the typing pass makes the accessor yield the field's
         address. The C backend can keep both halves, but the CPS accessor is
         `__slot_get`, which already performs the load, so the enclosing
         `Deref` would load the field's value as an address — a segfault for
         any value that is not one. Drop the redundant dereference. *)
      convert_expr (Mt.MSlotAccessor (e, name, ty))
  | Mt.MDeref e ->
      (* The load has its own opcode (0x20), and the JIT decodes it as a load
         from the pointer — `__deref` survives in `cps.rs` for binaries emitted
         before this line existed. Emitting the instruction rather than a call
         is what lets `scripts/isa-census` see memory traffic at all. *)
      Deref (convert_expr e)
  | Mt.MTypecast (e, _) -> convert_expr e
  | Mt.MSizeOf ty -> IntLit (Int64.of_int (size_of_mono_ty ty))
  | Mt.MSlotAccessor (e, name, _) -> 
      (* `__slot_get` already performs the load, so the address it is given may
         be the borrow's; only the offset lookup needs the pointee record. *)
      let offset = match pointee_type (get_expr_type e) with
        | MonoNamedType id -> find_record_slot_offset id name
        | _ -> 0
      in
      App ("__slot_get", [convert_expr e; IntLit (Int64.of_int offset)])
  | Mt.MPointerSlotAccessor (e, name, _) -> 
      let offset = match pointee_type (get_expr_type e) with
        | MonoNamedType id -> find_record_slot_offset id name
        | _ -> 0
      in
      App ("__ptr_slot_get", [convert_expr e; IntLit (Int64.of_int offset)])
  | Mt.MArrayIndex (arr, idx, _) -> App ("__array_index", [convert_expr arr; convert_expr idx])
  | Mt.MSpanIndex (span, idx, _) -> App ("__span_index", [convert_expr span; convert_expr idx])

(* A literal written straight into an argument position is converted against
   the callee's declared parameter type, not the literal's default width. *)
and narrow_args_for (decl_id: decl_id) (args: Mt.mexpr list): CpsGen.cps_expr list =
  match Hashtbl.find_opt function_params decl_id with
  | Some tys when List.length tys = List.length args ->
      List.map2 narrow_float_literal tys args
  | _ -> List.map convert_expr args

(* The typechecker wraps a float literal in a cast when the declared type is
   narrower than the literal's default f64 (and the CPS back end lowers both
   casts as no-ops — there is one float width on the wire), so the literal is
   looked for through casts. *)
and float_literal (expr: Mt.mexpr): float option =
  match expr with
  | Mt.MFloatConstant s ->
      Some (try float_of_string s with _ -> 0.0)
  | Mt.MCast (e, _) | Mt.MTypecast (e, _) -> float_literal e
  | _ -> None

and narrow_float_literal (ty: mono_ty) (expr: Mt.mexpr): CpsGen.cps_expr =
  match ty, float_literal expr with
  | MonoSingleFloat, Some f -> FloatLit32 f
  | _ -> convert_expr expr

(* U15c cross-module release. A checked union passed by value to a callee whose
   body this compilation does not emit is released by the caller once the call
   returns: the allocation belongs to the caller's provider, whose
   `<symbol>__free` only the caller can name. The callee, lowered in its own
   front-end run, has no transfer record and emits no release, so exactly one
   side frees. A call whose declared result is a union may hand the value back
   (the callee could return the pointer it was given), so the caller keeps
   ownership there and a later `match` releases it.

   The CPS IR has no expression-level sequencing, so the release is hoisted at
   the statement boundary: the transferred argument and the call are bound to
   temporaries, the call's release follows them, and the statement reads the
   result temporary. `xfer_hoisting` marks the conversion of that call so the
   expression path does not try to release it a second time. *)

(* The parts of a call expression a caller-side release needs, and how to
   rebuild it with substituted arguments. *)
type call_shape = {
  cs_key: string;
  cs_args: Mt.mexpr list;
  cs_ret: mono_ty;
  cs_rebuild: Mt.mexpr list -> Mt.mexpr;
}

(******************************************************************************)
(* MAST → CPS STATEMENT CONVERSION *)
(******************************************************************************)

(* The call a caller-side release can act on, if `expr` (through casts) is one.
   A foreign import is not: its adapter wrapper owns the value, and a checked
   adapter call *produces* the union rather than consuming one. *)
let rec call_shape (expr: Mt.mexpr): call_shape option =
  match expr with
  | Mt.MCast (e, _) | Mt.MTypecast (e, _) -> call_shape e
  | Mt.MConcreteFuncall (decl_id, q, args, ty) ->
      let name = qident_to_string q in
      let is_exit_success =
        ends_with name ":ExitSuccess" || ends_with name ".ExitSuccess"
        || name = "ExitSuccess"
      in
      if is_exit_success
         || Hashtbl.mem checked_option_adapters decl_id
         || Hashtbl.mem foreign_functions decl_id
         || Hashtbl.mem foreign_functions_by_name (ident_string (original_name q))
      then None
      else
        Some {
          cs_key = callee_decl_key decl_id;
          cs_args = args;
          cs_ret = ty;
          cs_rebuild = (fun args -> Mt.MConcreteFuncall (decl_id, q, args, ty));
        }
  | Mt.MGenericFuncall (id, args, ty) ->
      Some {
        cs_key = callee_mono_key id;
        cs_args = args;
        cs_ret = ty;
        cs_rebuild = (fun args -> Mt.MGenericFuncall (id, args, ty));
      }
  | Mt.MConcreteMethodCall (id, q, args, ty) ->
      Some {
        cs_key = callee_meth_key id;
        cs_args = args;
        cs_ret = ty;
        cs_rebuild = (fun args -> Mt.MConcreteMethodCall (id, q, args, ty));
      }
  | Mt.MGenericMethodCall (a, id, args, ty) ->
      Some {
        cs_key = callee_mono_key id;
        cs_args = args;
        cs_ret = ty;
        cs_rebuild = (fun args -> Mt.MGenericMethodCall (a, id, args, ty));
      }
  | _ -> None

(* The statements a caller-side release needs, and the temporary naming the
   call's result. `None` means no release is due: the callee's body is emitted
   here (its own `match` releases), the call returns a union (ownership may
   pass through), or no argument owns a checked union. *)
let caller_release_setup (expr: Mt.mexpr): (CpsGen.cps_stmt * string) option =
  match call_shape expr with
  | None -> None
  | Some cs ->
    if Hashtbl.mem emitted_callee_keys cs.cs_key
       || (match cs.cs_ret with MonoNamedType _ -> true | _ -> false)
    then None
    else
      let transfers =
        List.filter_map
          (fun (index, arg) ->
            match get_expr_type arg, checked_value_free arg with
            | MonoNamedType _, Some (free, local) -> Some (index, free, local)
            | _ -> None)
          (List.mapi (fun index arg -> (index, arg)) cs.cs_args)
      in
      if transfers = [] then None
      else begin
        let bound =
          List.map
            (fun (index, free, local) ->
              (index, fresh_xfer_name "arg", free, local))
            transfers
        in
        let substituted =
          List.mapi
            (fun index arg ->
              match List.find_opt (fun (i, _, _, _) -> i = index) bound with
              | Some (_, name, _, _) ->
                  Mt.MLocalVar (make_ident name, get_expr_type arg)
              | None -> arg)
            cs.cs_args
        in
        (* Ownership leaves the caller's table here: the value is released by
           this setup, so a later `match` on the same name must not free it
           again. *)
        List.iter
          (fun (_, _, _, local) ->
            match local with
            | Some name -> Hashtbl.remove checked_owned_locals name
            | None -> ())
          bound;
        let ret_name = fresh_xfer_name "ret" in
        let call_expr =
          xfer_hoisting := true;
          let converted =
            try convert_expr (cs.cs_rebuild substituted)
            with exn -> xfer_hoisting := false; raise exn
          in
          xfer_hoisting := false;
          converted
        in
        let releases =
          List.fold_right
            (fun (_, name, free, _) acc ->
              Block (Discard (App (free, [Var name])), acc))
            bound Skip
        in
        let call =
          List.fold_right
            (fun (index, name, _, _) acc ->
              let arg = List.nth cs.cs_args index in
              Block (Let (name, convert_expr arg, Skip), acc))
            bound
            (Block (Let (ret_name, call_expr, Skip),
              Block (releases, Skip)))
        in
        Some (call, ret_name)
      end

let rec convert_stmt (stmt: Mt.mstmt): CpsGen.cps_stmt =
  match stmt with
  | Mt.MSkip -> Skip
  | Mt.MLet (id, ty, body) ->
      let var_name = ident_string id in
      Hashtbl.replace local_types var_name ty;
      let init = zero_value (mono_ty_to_cps_type ty) in
      Let (var_name, init, convert_stmt body)
  | Mt.MDestructure (bindings, expr, body) ->
      let tmp_name = "__destructure_tmp" in
      let tmp_let = Let (tmp_name, convert_expr expr, Skip) in
      (* `__slot_get` takes a numeric byte offset, not the field name. Resolve
         each field's offset from the record's slot layout, mirroring how
         MSlotAccessor lowers `record.field`. Passing `Var name` here (the old
         behaviour) made the JIT look up a non-existent variable named after the
         field -> "Undefined variable: <field>". *)
      let recty = pointee_type (get_expr_type expr) in
      let binding_stmts = List.map (fun (Mtast.MonoBinding { rename; name; _ }) ->
        let offset = match recty with
          | MonoNamedType id -> find_record_slot_offset id name
          | _ -> 0
        in
        Let (ident_string rename, App ("__slot_get", [Var tmp_name; IntLit (Int64.of_int offset)]), Skip)
      ) bindings in
      let body_stmt = convert_stmt body in
      let combined = List.fold_right (fun s acc -> Block (s, acc)) binding_stmts body_stmt in
      Block (tmp_let, combined)
  | Mt.MAssign (dest, src) ->
      let dest_expr = convert_expr dest in
      let src_expr = narrow_float_literal (get_expr_type dest) src in
      (* The pointer-like test must come *first*: `MAssign` is the store form
         (`*r = v`, typed as a write reference, see TypingPass), and its target
         is often a plain `Var`, which the second branch would otherwise rebind
         instead of storing through. *)
      (match dest_expr with
       | _ when is_pointer_like (get_expr_type dest) ->
           (* `*dest = src`: the frontend hands us the *address* to write, so
              this is the store instruction (0x30). The old fallback assigned
              to a variable named `__assign_target` that nothing reads — the
              store was silently dropped, which is why the census never saw
              0x30. *)
           Store (dest_expr, src_expr)
       | Var name -> Assign (name, src_expr)
       | _ -> Assign ("__assign_target", src_expr))
  | Mt.MAssignVar (q, src) ->
      (* Use the unqualified name: MLet declares the local under `ident_string id`,
         but the qualified debug name (e.g. "T1::x") would assign a *different*
         JIT variable, leaving the declared one at its zero-init. *)
      (match caller_release_setup src with
       | Some (setup, ret_name) ->
           Block (setup, Assign (ident_string (original_name q), Var ret_name))
       | None ->
           register_checked_owned (ident_string (original_name q)) src;
           Assign (ident_string (original_name q), narrow_float_literal (local_type q) src))
  | Mt.MInitialAssign (q, src) ->
      (match caller_release_setup src with
       | Some (setup, ret_name) ->
           Block (setup, Assign (ident_string (original_name q), Var ret_name))
       | None ->
           register_checked_owned (ident_string (original_name q)) src;
           Assign (ident_string (original_name q), narrow_float_literal (local_type q) src))
  | Mt.MIf (cond, then_stmt, else_stmt) ->
      If (convert_expr cond, convert_stmt then_stmt, convert_stmt else_stmt)
  | Mt.MCase (expr, whens, _) ->
      let scrutinee = convert_expr expr in
      let scrutinee_ty = get_expr_type expr in
      let tmp_name = "__match_scrutinee" in
      let scrutinee_let = Let (tmp_name, scrutinee, Skip) in
      (* For union types, load the discriminant tag from offset 0 *)
      let tag_name = "__match_tag" in
      let tag_load = match scrutinee_ty with
        | MonoNamedType _ ->
            (* Union pointer: load tag from offset 0 *)
            Let (tag_name, App ("__slot_get", [Var tmp_name; IntLit 0L]), Skip)
        | _ ->
            (* Primitive: compare directly *)
            Let (tag_name, Var tmp_name, Skip)
      in
      let case_stmts fetch = List.mapi (fun i (Mtast.MTypedWhen (tag, bindings, body)) ->
        (* Compute the numeric index of this tag in the union layout *)
        let tag_idx = match scrutinee_ty with
          | MonoNamedType id ->
              (match Hashtbl.find_opt union_layouts id with
               | Some cases -> find_union_case_offset cases tag 0
               | None -> i)
          | _ -> i
        in
        (* Bind union fields: `fetch j` is where slot `j`'s value comes from *)
        let binding_stmts = List.mapi (fun j (Mtast.MonoBinding { rename; _ }) ->
          Let (ident_string rename, fetch j, Skip)
        ) bindings in
        let body_stmt = convert_stmt body in
        let combined = List.fold_right (fun s acc -> Block (s, acc)) binding_stmts body_stmt in
        (Int64.of_int tag_idx, combined)
      ) whens in
      let dispatch fetch = Match (Var tag_name, case_stmts fetch, Skip) in
      let slot j =
        App ("__slot_get", [Var tmp_name; IntLit (Int64.of_int ((j + 1) * 8))])
      in
      (* U15c: a checked adapter call returns a union the wrapper allocated
         (`Box::into_raw`). Copy the tag and every slot a case binds into
         locals, release the union, then dispatch on the copies: a `return`
         inside a case cannot skip the release, and no case body reads memory
         that was freed. The transport admits scalar payloads, so a copied word
         owns nothing and needs no release of its own. A result bound to a
         local is covered through `checked_owned_locals`, which follows the
         name to this match, and a parameter a call transferred the union into
         is registered the same way; a rebinding clears the record (a leak at
         worst, never a second free). *)
      let checked_free, owned_local = match checked_scrutinee_free expr with
        | Some (free, owned) -> (Some free, owned)
        | None -> (None, None)
      in
      (match owned_local with
       | Some name -> Hashtbl.remove checked_owned_locals name
       | None -> ());
      (* 0x0A, not a chain of `If`s. The JIT's match instruction evaluates the
         scrutinee once and dispatches on the tag, which is exactly what the
         chain computed — but as one instruction the tag dispatch is visible to
         the census and to the ISA model, instead of being a control-flow
         pattern the lowering hid. The empty default branch reproduces the old
         `Skip` for a tag no case names. *)
      (match checked_free with
       | None ->
         Block (scrutinee_let, Block (tag_load, dispatch slot))
       | Some free ->
         let max_bindings =
           List.fold_left
             (fun acc (Mtast.MTypedWhen (_, bindings, _)) ->
               max acc (List.length bindings))
             0 whens
         in
         let slot_lets =
           List.init max_bindings (fun j ->
               Let ("__match_slot" ^ string_of_int j, slot j, Skip))
         in
         (* Release *before* the dispatch: a case body that `return`s ends the
            function, so a release placed after the `match` would be skipped for
            exactly the shapes every checked fixture uses. Every value a case
            binds was copied into `__match_slot*` above, so no body reads the
            freed union. *)
         let releases =
           List.fold_right (fun s acc -> Block (s, acc)) slot_lets
             (Discard (App (free, [Var tmp_name])))
         in
         Block (scrutinee_let, Block (tag_load,
           Block (releases,
             dispatch (fun j -> Var ("__match_slot" ^ string_of_int j))))))
  | Mt.MWhile (cond, body) ->
      While (convert_expr cond, convert_stmt body)
  | Mt.MFor (id, start_expr, end_expr, body) ->
      (* Austral's specification and the C renderer use an **inclusive**
         endpoint: `for i from 0 to K` executes the body K+1 times (see
         `CRenderer.CFor`, which renders `i <= K`). This lowering used to emit
         `CmpLt`, silently running one iteration fewer than the defined
         semantics — `for i from 0 to 0` did nothing here and one update under
         C. `CmpLte` is already an opcode (0x12) the JIT implements, so the
         loop condition is now the same relation the specification names. *)
      let var_name = ident_string id in
      let init_let = Let (var_name, convert_expr start_expr, Skip) in
      let cond = CmpLte (Var var_name, convert_expr end_expr) in
      let inc = Assign (var_name, Add (Var var_name, IntLit 1L)) in
      let loop_body = Block (convert_stmt body, inc) in
      Block (init_let, While (cond, loop_body))
  | Mt.MBorrow { original; rename; orig_type; body; _ } ->
      (* The JIT represents a record (or union) value, and a reference, as a
         pointer, so borrowing one is an alias of that value, not an address it
         cannot take: `&context` and `context->field` are the same pointer. The
         old lowering emitted the constant 0 here, which made every
         `record->field` read through a borrow dereference null. Scalar borrows
         still have no cell to point at, so they keep the 0 placeholder rather
         than inventing aliasing semantics the model does not implement. *)
      let reference =
        match orig_type with
        | MonoNamedType _ | MonoReadRef _ | MonoWriteRef _ | MonoAddress _
        | MonoPointer _ -> Var (ident_string original)
        | _ -> IntLit 0L
      in
      Let (ident_string rename, reference, convert_stmt body)
  | Mt.MBlock (s1, s2) ->
      (* Sequence the conversions explicitly: OCaml would evaluate the two
         arguments right-to-left, so a statement's side effects on the
         conversion tables (`local_types`, and `checked_owned_locals` for
         U15c) would run after the statements that read them. The emitted
         block is the same either way. *)
      let first = convert_stmt s1 in
      let second = convert_stmt s2 in
      Block (first, second)
  | Mt.MDiscarding expr ->
      (match caller_release_setup expr with
       | Some (setup, _) -> Block (setup, Skip)
       | None -> Discard (convert_expr expr))
  | Mt.MReturn expr ->
      (match caller_release_setup expr with
       | Some (setup, ret_name) -> Block (setup, Return (Var ret_name))
       | None -> Return (narrow_float_literal !current_return_type expr))
  | Mt.MLetTmp (id, ty, expr) ->
      Hashtbl.replace local_types (ident_string id) ty;
      (match caller_release_setup expr with
       | Some (setup, ret_name) ->
           Block (setup, Let (ident_string id, Var ret_name, Skip))
       | None ->
           register_checked_owned (ident_string id) expr;
           Let (ident_string id, narrow_float_literal ty expr, Skip))
  | Mt.MAssignTmp (id, expr) ->
      let ty = match Hashtbl.find_opt local_types (ident_string id) with
        | Some ty -> ty
        | None -> MonoUnit
      in
      (match caller_release_setup expr with
       | Some (setup, ret_name) -> Block (setup, Assign (ident_string id, Var ret_name))
       | None -> Assign (ident_string id, narrow_float_literal ty expr))

and zero_value (ty: CpsGen.cps_type): CpsGen.cps_expr =
  match ty with
  | I64 -> IntLit 0L
  | I32 -> IntLit 0L
  | Bool -> BoolLit false
  | Unit -> IntLit 0L
  | String -> IntLit 0L
  | F64 -> FloatLit 0.0

(******************************************************************************)
(* BUILD CPS FUNCTION FROM MAST DECLARATION *)
(******************************************************************************)

(* U15c: the parameters a call site transferred a checked union into. They are
   registered as owned before the body is converted, so a `match` on one
   releases it through the same path a local's release uses. *)
let register_owned_parameters (key: string)
    (params: Mt.mvalue_parameter list): unit =
  match Hashtbl.find_opt checked_owned_fn_params key with
  | None -> ()
  | Some entries ->
    List.iter
      (fun (index, free) ->
        match List.nth_opt params index with
        | Some (Mt.MValueParameter (pid, _)) ->
          Hashtbl.replace checked_owned_locals (ident_string pid) free
        | None -> ())
      entries

let build_cps_function (decl: Mt.mdecl): CpsGen.function_def option =
  match decl with
  | Mt.MFunction (decl_id, name, params, ret_ty, body) ->
      let func_name = ident_string name in
      let param_names = List.map (fun (Mtast.MValueParameter (id, _)) -> ident_string id) params in
      let return_type = mono_ty_to_cps_type ret_ty in
      current_return_type := ret_ty;
      Hashtbl.reset local_types;
      Hashtbl.reset checked_owned_locals;
      register_owned_parameters (callee_decl_key decl_id) params;
      let body_stmt = convert_stmt body in
      Some {
        name = func_name;
        params = param_names;
        return_type = return_type;
        body = body_stmt
      }
  | Mt.MFunctionMonomorph (id, params, ret_ty, body) ->
      let func_name = "monomorph_" ^ show_mono_id id in
      let param_names = List.map (fun (Mtast.MValueParameter (pid, _)) -> ident_string pid) params in
      let return_type = mono_ty_to_cps_type ret_ty in
      current_return_type := ret_ty;
      Hashtbl.reset local_types;
      Hashtbl.reset checked_owned_locals;
      register_owned_parameters (callee_mono_key id) params;
      let body_stmt = convert_stmt body in
      Some {
        name = func_name;
        params = param_names;
        return_type = return_type;
        body = body_stmt
      }
  | Mt.MMethodMonomorph (id, params, ret_ty, body) ->
      let func_name = "method_" ^ show_mono_id id in
      let param_names = List.map (fun (Mtast.MValueParameter (pid, _)) -> ident_string pid) params in
      let return_type = mono_ty_to_cps_type ret_ty in
      current_return_type := ret_ty;
      Hashtbl.reset local_types;
      Hashtbl.reset checked_owned_locals;
      register_owned_parameters (callee_mono_key id) params;
      let body_stmt = convert_stmt body in
      Some {
        name = func_name;
        params = param_names;
        return_type = return_type;
        body = body_stmt
      }
  | Mt.MConstant (_, name, ty, expr) ->
      let const_name = ident_string name in
      let return_type = mono_ty_to_cps_type ty in
      current_return_type := ty;
      Hashtbl.reset local_types;
      Hashtbl.reset checked_owned_locals;
      Some {
        name = const_name;
        params = [];
        return_type = return_type;
        body = Return (narrow_float_literal ty expr)
      }
  | _ -> None

(* A concrete instance method is emitted under its instance-method id, the
   name every `MConcreteMethodCall` uses, so a call resolves to the body that
   owns the parameter. Its transfer table entry is read before the body is
   converted, exactly as `build_cps_function` does for a top-level function. *)
let build_concrete_method_function
    (Mt.MConcreteMethod (id, _name, params, ret_ty, body))
    : CpsGen.function_def option =
  let func_name = concrete_method_symbol id in
  let param_names = List.map (fun (Mtast.MValueParameter (pid, _)) -> ident_string pid) params in
  let return_type = mono_ty_to_cps_type ret_ty in
  current_return_type := ret_ty;
  Hashtbl.reset local_types;
  Hashtbl.reset checked_owned_locals;
  register_owned_parameters (callee_meth_key id) params;
  let body_stmt = convert_stmt body in
  Some {
    name = func_name;
    params = param_names;
    return_type = return_type;
    body = body_stmt
  }

(* One declaration can carry several functions: a concrete instance is a list
   of methods. *)
let build_cps_decl (decl: Mt.mdecl): CpsGen.function_def list =
  match decl with
  | Mt.MConcreteInstance (_, _, _, methods) ->
      List.filter_map build_concrete_method_function methods
  | _ ->
      (match build_cps_function decl with Some f -> [f] | None -> [])

(******************************************************************************)
(* MAIN ENTRY POINT: COMPILE MODULE *)
(******************************************************************************)

let compile_module_cps (mono_module: Mt.mono_module): (CpsGen.function_def list * string) =
  match mono_module with
  | Mt.MonoModule (name, decls) ->
      (* U15: union layouts must be known before the foreign pass classifies an
         `Option` result, regardless of where the union declaration sits. *)
      List.iter
        (function
          | Mt.MUnionMonomorph (id, cases) ->
            Hashtbl.replace union_layouts id cases
          | _ -> ())
        decls;
      (* First pass: collect record layouts and foreign function mappings *)
      List.iter (function
        | Mt.MRecordMonomorph (id, slots) -> Hashtbl.replace record_layouts id slots
        | Mt.MUnionMonomorph (id, cases) -> Hashtbl.replace union_layouts id cases
        | Mt.MFunction (id, name, params, _, _) ->
            Hashtbl.replace function_params id (param_types params);
            Hashtbl.replace function_value_names id (ident_string name)
        | Mt.MFunctionMonomorph (id, params, _, _) ->
            Hashtbl.replace monomorph_params id (param_types params)
        | Mt.MForeignFunction (id, name, params, rt, ext_name, handle) ->
            Hashtbl.replace function_params id (param_types params);
            (match handle with
             | Some h ->
                let target = handle_target_of_dispose_symbol ext_name in
                Hashtbl.replace required_handles target
                  { hr_target = target;
                    hr_type_name = h.mh_type_name;
                    hr_constructor = h.mh_constructor;
                    hr_destructor = h.mh_destructor;
                    hr_params = h.mh_params;
                    hr_use = h.mh_use;
                    hr_write = h.mh_write }
             | None -> ());
            let ext_name =
              if is_rust_import ext_name && foreign_needs_adapter params rt then begin
                let symbol = adapter_symbol ext_name in
                let all_scalar =
                  List.for_all
                    (function Mt.MValueParameter (_, ty) ->
                      adapter_scalar_name ty <> None)
                    params
                in
                let checked =
                  if all_scalar then
                    match CheckedOptions.find ext_name with
                    | Some decision ->
                      (match mono_ty_of_checked_payload decision.CheckedOptions.payload with
                       | Some payload ->
                         let failure_payload =
                           match decision.failure_payload with
                           | None -> Some None
                           | Some ty ->
                             Option.map
                               (fun p -> Some p)
                               (mono_ty_of_checked_payload ty)
                         in
                         (match failure_payload with
                          | Some failure_payload ->
                            Some {
                              co_checked_symbol = symbol ^ "__checked";
                              co_payload = payload;
                              co_failure_tag = decision.failure_tag;
                              co_success_tag = decision.success_tag;
                              co_failure_payload = failure_payload;
                              co_union_name = decision.union_name;
                              co_failure_case = decision.failure_case;
                              co_success_case = decision.success_case;
                              co_free_symbol = symbol ^ "__free";
                            }
                          | None -> checked_option_of ~symbol ~result:rt)
                       | None -> checked_option_of ~symbol ~result:rt)
                    | None -> checked_option_of ~symbol ~result:rt
                  else None
                in
                (match checked with
                 | Some co ->
                   (* The JIT's `match` compares a loaded tag against the union's
                      *declaration-order* index. An imported union has no
                      `MUnionMonomorph` declaration in this module, so without
                      a layout the lowering falls back to the source order of
                      the `when` clauses, which is not the same thing. The
                      typer's decision supplies the declaration order
                      (`None` then `Some`), so register the shape under the
                      signature's id. *)
                   (match rt with
                    | MonoNamedType result_id ->
                      if not (Hashtbl.mem union_layouts result_id) then
                        (match co.co_failure_payload with
                         | None ->
                           Hashtbl.replace union_layouts result_id
                             [ MonoCase (make_ident "None", []);
                               MonoCase
                                 (make_ident "Some",
                                  [ MonoSlot (make_ident "value", co.co_payload) ]) ]
                         | Some left ->
                           (* `Either` carries a payload in both cases, so the
                              layout has one slot per case and the tag decides
                              which type the slot holds. The tags are the
                              declaration indices, which the typer pinned to
                              0 and 1 before this record existed. *)
                           Hashtbl.replace union_layouts result_id
                             [ MonoCase
                                 (make_ident co.co_failure_case,
                                  [ MonoSlot (make_ident "value", left) ]);
                               MonoCase
                                 (make_ident co.co_success_case,
                                  [ MonoSlot (make_ident "value", co.co_payload) ]) ])
                                        | _ -> ());
                   (* The descriptor's tags must be the indices of the layout
                      the `match` lowering reads, and a module that uses the
                      union itself carries its own monomorph — whose case
                      order is not always the declaration order the typer saw.
                      Read the tags back from that layout instead of trusting
                      the typer's order; with no layout (an imported union) the
                      registered shape above is the layout, so this is the
                      same answer. *)
                   let co =
                     match rt with
                     | MonoNamedType result_id ->
                       (match Hashtbl.find_opt union_layouts result_id with
                        | Some cases ->
                          { co with
                            co_failure_tag =
                              find_union_case_offset cases
                                (make_ident co.co_failure_case) 0;
                            co_success_tag =
                              find_union_case_offset cases
                                (make_ident co.co_success_case) 0 }
                        | None -> co)
                     | _ -> co
                   in
                   Hashtbl.replace checked_option_adapters id co;
                   Hashtbl.replace required_adapters symbol
                     { ar_symbol = symbol;
                       ar_target = ext_name;
                       ar_params =
                         List.map (function Mt.MValueParameter (_, ty) -> ty) params;
                       ar_result = co.co_payload;
                       ar_checked = Some (co.co_failure_tag, co.co_success_tag);
                       ar_checked_union =
                         Option.map
                           (fun left ->
                             (co.co_union_name, co.co_failure_case,
                              co.co_success_case,
                              match adapter_scalar_name left with
                              | Some name -> name
                              | None -> assert false))
                           co.co_failure_payload }
                 | None ->
                   Hashtbl.replace required_adapters symbol
                     { ar_symbol = symbol;
                       ar_target = ext_name;
                       ar_params =
                         List.map (function Mt.MValueParameter (_, ty) -> ty) params;
                       ar_result = rt;
                       ar_checked = None;
                       ar_checked_union = None });
                symbol
              end else ext_name
            in
            Hashtbl.replace foreign_functions id ext_name;
            Hashtbl.replace foreign_functions_by_name (ident_string name) ext_name
        | Mt.MRecord (_, _, _) -> () (* FIXME: handle global records if needed *)
        | Mt.MUnion (_, _, _) -> ()
        | _ -> ()
      ) decls;
      (* Second pass: convert functions. A transfer discovered while converting
         a call site can make the callee's parameter owned, and that callee can
         transfer the value on again, so convert until a pass adds no new
         transfer. Entries are only ever added, so this terminates; only the
         last pass's code is kept. *)
      Hashtbl.reset local_concrete_methods;
      List.iter
        (function
          | Mt.MConcreteInstance (_, _, _, methods) ->
            List.iter
              (function
                | Mt.MConcreteMethod (id, _, _, _, _) ->
                  Hashtbl.replace local_concrete_methods id ())
              methods
          | _ -> ())
        decls;
      (* Which callees this compilation emits. A call into any other one is
         released by its caller (see {!caller_release_setup}). *)
      Hashtbl.reset emitted_callee_keys;
      xfer_counter := 0;
      List.iter
        (function
          | Mt.MFunction (id, _, _, _, _) ->
            Hashtbl.replace emitted_callee_keys (callee_decl_key id) ()
          | Mt.MFunctionMonomorph (id, _, _, _)
          | Mt.MMethodMonomorph (id, _, _, _) ->
            Hashtbl.replace emitted_callee_keys (callee_mono_key id) ()
          | Mt.MConcreteInstance (_, _, _, methods) ->
            List.iter
              (function
                | Mt.MConcreteMethod (id, _, _, _, _) ->
                  Hashtbl.replace emitted_callee_keys (callee_meth_key id) ())
              methods
          | _ -> ())
        decls;
      Hashtbl.reset checked_owned_fn_params;
      Hashtbl.reset checked_param_conflicts;
      let rec convert_until_stable () =
        checked_transfers_changed := false;
        let funcs = List.concat_map build_cps_decl decls in
        if !checked_transfers_changed then convert_until_stable () else funcs
      in
      (convert_until_stable (), mod_name_string name)

(******************************************************************************)
(* BINARY GENERATION AND WRITING *)
(******************************************************************************)

let write_cps_binary (funcs: CpsGen.function_def list) (module_name: string) (output_path: string): unit =
  let binary = CpsGen.serialize_functions ~module_name funcs in
  let oc = open_out_bin output_path in
  output_string oc binary;
  close_out oc

let compile_and_save (mono_module: Mt.mono_module) (output_path: string): unit =
  let (funcs, name) = compile_module_cps mono_module in
  if List.length funcs > 0 then
    write_cps_binary funcs name output_path

(******************************************************************************)
(* TESTING HELPERS *)
(******************************************************************************)

let debug_print_mono_module (mono_module: Mt.mono_module) =
  match mono_module with
  | Mt.MonoModule (name, decls) ->
      Printf.printf "Module: %s\n" (mod_name_string name);
      Printf.printf "Declarations: %d\n" (List.length decls);
      List.iter (function
        | Mt.MFunction (_, name, params, _, _) ->
            Printf.printf "  Function: %s (params: %d)\n"
              (ident_string name) (List.length params)
        | Mt.MFunctionMonomorph (id, params, _, _) ->
            Printf.printf "  FunctionMonomorph: %s (params: %d)\n"
              (show_mono_id id) (List.length params)
        | Mt.MConstant (_, name, _, _) ->
            Printf.printf "  Constant: %s\n" (ident_string name)
        | Mt.MRecord (_, name, _) ->
            Printf.printf "  Record: %s\n" (ident_string name)
        | _ -> Printf.printf "  Other declaration\n"
      ) decls

(** The wire type name the CPS artifact encodes for a monomorphic type. This
    is the type a compiled export actually crosses, not the source spelling. *)
let cps_type_name (ty: mono_ty): string =
  match mono_ty_to_cps_type ty with
  | I64 -> "Int64"
  | I32 -> "Int32"
  | Bool -> "Bool"
  | Unit -> "Unit"
  | String -> "String"
  | F64 -> "Float64"

(** The typed exports a compile-only artifact carries: exactly the
    function/constant declarations the CPS lowering turns into functions, with
    their wire parameter and return types. Foreign declarations are imports,
    not exports, so they are omitted. The names match {!build_cps_function}.
*)
let typed_exports (Mt.MonoModule (_, decls)): (string * string list * string) list =
  let params (ps: Mt.mvalue_parameter list): string list =
    List.map (fun (Mt.MValueParameter (_, ty)) -> cps_type_name ty) ps
  in
  let decl_exports (decl: Mt.mdecl): (string * string list * string) list =
    match decl with
    | Mt.MFunction (_, name, ps, ret_ty, _) ->
        [ (ident_string name, params ps, cps_type_name ret_ty) ]
    | Mt.MFunctionMonomorph (id, ps, ret_ty, _) ->
        [ ("monomorph_" ^ show_mono_id id, params ps, cps_type_name ret_ty) ]
    | Mt.MMethodMonomorph (id, ps, ret_ty, _) ->
        [ ("method_" ^ show_mono_id id, params ps, cps_type_name ret_ty) ]
    | Mt.MConcreteInstance (_, _, _, methods) ->
        List.map
          (fun (Mt.MConcreteMethod (id, _, ps, ret_ty, _)) ->
            (concrete_method_symbol id, params ps, cps_type_name ret_ty))
          methods
    | Mt.MConstant (_, name, ty, _) ->
        [ (ident_string name, [], cps_type_name ty) ]
    | _ -> []
  in
  List.concat_map decl_exports decls

let debug_print_cps_functions (funcs: CpsGen.function_def list) =
  Printf.printf "CPS Functions: %d\n" (List.length funcs);
  List.iter (fun func ->
    Printf.printf "  %s(%s) -> " func.name (String.concat ", " func.params);
    Printf.printf "%s\n" (match func.return_type with
      | I64 -> "I64" | I32 -> "I32" | Bool -> "Bool" | Unit -> "Unit" | String -> "String" | F64 -> "F64");
    Printf.printf "    Body: %s\n" (CpsGen.string_of_stmt func.body)
  ) funcs
