(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   The NPU DMA gate compiler pass (GPU.md): rejects any compiled module whose
   declared DMA transfers would overflow the NPU SRAM. The decision is made
   by `Npu_dma_gate.dma_verdict` — the pinned extraction of
   `why3_plugin/npu_dma_gate.mlw` (emitted by the unfer probability kernel's
   `uk_whyml_emit`, WhymlProgram::NpuDmaGate, and machine-checked by Why3).
   By the WhyML postcondition (soundness) the gate never returns 0 for an
   over-limit transfer, so the compiler can never emit a module whose
   hardware loads exceed the physical SRAM — the GPU.md hardware-invariant
   loop, enforced on the same `Compiler_plugin.run_on_typed` seam as the
   Why3 authorization gate.

   DMA surface: a module declares its hardware transfers as top-level
   constants named `dma_<label>_bytes` (transfer size) and optionally
   `dma_<label>_offset` (start offset, default 0). Only integer-literal
   initializers are decidable; a `dma_*` constant with a non-literal
   initializer is skipped (no-op, same as the deltanet gate's subset rule).
   Modules without `dma_*` constants are unaffected.
*)

open Compiler_plugin
open Stages.Tast

(* ── evaluate integer constant initializers (the deltanet subset) ───── *)

let eval_int (e : texpr) : int option =
  match e with
  | TIntConstant s -> (try Some (int_of_string s) with _ -> None)
  | _ -> None

(* ── parse `dma_<label>_bytes` / `dma_<label>_offset` constant names ── *)

let strip_prefix ~prefix s =
  let lp = String.length prefix in
  if String.length s >= lp && String.sub s 0 lp = prefix then
    Some (String.sub s lp (String.length s - lp))
  else None

let has_suffix s suf =
  let ls = String.length s in
  let lf = String.length suf in
  ls >= lf && String.sub s (ls - lf) lf = suf

(** Returns `(label, kind)` where kind is `bytes` or `offset`. *)
let parse_dma_constant (id : Identifier.identifier) : (string * string) option =
  let name = Identifier.ident_string id in
  match strip_prefix ~prefix:"dma_" name with
  | None -> None
  | Some rest ->
      if has_suffix rest "_bytes" then
        Some (String.sub rest 0 (String.length rest - 6), "bytes")
      else if has_suffix rest "_offset" then
        Some (String.sub rest 0 (String.length rest - 7), "offset")
      else None

(* ── the gate check ──────────────────────────────────────────────────── *)

let check ~module_name ~foreign_externals:_ ~constants : verdict =
  (* Collect the module's declared transfers: label -> (offset, bytes). *)
  let offsets = Hashtbl.create 8 in
  let bytes = Hashtbl.create 8 in
  List.iter
    (fun (id, init) ->
      match (parse_dma_constant id, eval_int init) with
      | Some (label, "offset"), Some v -> Hashtbl.replace offsets label v
      | Some (label, "bytes"), Some v -> Hashtbl.replace bytes label v
      | _ -> ())
    constants;
  let labels =
    List.sort_uniq String.compare
      (Hashtbl.fold (fun l _ acc -> l :: acc) bytes [])
  in
  List.fold_left
    (fun acc label ->
      match acc with
      | VerdictReject _ -> acc
      | VerdictOk ->
          let nbytes = Hashtbl.find bytes label in
          let offset =
            match Hashtbl.find_opt offsets label with
            | Some v -> v
            | None -> 0
          in
          if nbytes < 0 || offset < 0 then
            VerdictReject
              (Printf.sprintf
                 "module %s declares dma_%s with negative size/offset \
                  (offset %d, %d bytes) — a negative DMA transfer is malformed"
                 module_name label offset nbytes)
          else
            match Npu_dma_gate.dma_verdict offset nbytes with
            | 0 -> VerdictOk
            | _ ->
                VerdictReject
                  (Printf.sprintf
                     "module %s declares DMA transfer dma_%s (offset %d, %d bytes) \
                      which overflows the NPU SRAM (%d bytes) — the WhyML-verified \
                      gate (npu_dma_gate.mlw) guarantees no over-limit load is \
                      accepted"
                     module_name label offset nbytes 262144))
    VerdictOk labels

(** Register the pass (idempotent). Called from `Vm_plugin.boot` next to
    `Why3_plugin.install` and `Deltanet_plugin.install`. *)
let install () =
  if List.mem "npu_dma_gate" (list_registered ()) then ()
  else register ~name:"npu_dma_gate" check
