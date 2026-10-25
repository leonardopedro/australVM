(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)

(** The linearity ledger (docs/memsafe.md §3).

    {!LinearityCheck} decides, at every point where a linear resource changes
    hands, whether the program is legal. This module records those decisions as
    a trace, so a frozen verifier can re-check them without trusting the
    checker's control flow.

    A ledger is a sequence of [(variable, event)] rows per function, each row
    tagged with the branch path it was recorded on. {!timelines} resolves rows
    into one timeline per (variable, complete execution path): that is the
    shape the MM0 automaton of [docs/memsafe.mm0] consumes.

    Recording is off by default — [Compiler] turns it on only for
    `--emit-linearity-ledger`, so ordinary compilation pays nothing. *)

(** One legal transition of a linear resource. *)
type event =
  | Acquire      (** a linear value enters scope, unconsumed *)
  | Consume      (** the one legal consumption of an acquired value *)
  | Reacquire    (** an assignment restores a consumed variable to unconsumed *)
  | BorrowRead   (** a read borrow of an acquired value starts *)
  | BorrowWrite  (** a write borrow of an acquired value starts *)
  | Reborrow     (** a `reborrow` of an acquired value starts *)
  | Unborrow     (** the borrow's scope ends; the value is owned again *)
  | Drop         (** a linear reference-type is discharged when its scope ends *)

(** The wire spelling of an event, used by the JSON export. *)
val event_token : event -> string

(** The [docs/memsafe.mm0] constructor of an event, used by [ledger2mm0]. *)
val event_rule : event -> string

(** One recorded decision. [path] is innermost-first: [(branch_id, arm)]. *)
type row = {
    var: string;
    ty: string;
    depth: int;
    event: event;
    path: (int * int) list;
  }

(** A resolved per-variable timeline on one complete execution path. *)
type timeline = {
    t_var: string;
    t_ty: string;
    t_path: (int * int) list;
    t_events: event list;
  }

(** Enable / disable recording. Disabling clears the ledger. *)
val set_enabled : bool -> unit

(** Whether recording is on. Hooks check this before doing any work. *)
val is_enabled : unit -> bool

(** Forget every recorded row and function, and reset branch ids. Called once
    per module by [Compiler] when the export flag is on. *)
val clear : unit -> unit

(** Start recording for one function or method. Implicitly ends any function
    still open. *)
val begin_function : name:string -> kind:string -> unit

(** Close the current function. A no-op when none is open. *)
val end_function : unit -> unit

(** A fresh branch id for an [if]/[case] site. Ids are unique per module, so a
    nested branch can never share its parent's id. *)
val next_branch : unit -> int

(** Run [f] as the arm [arm] (0 = then/first case arm) of branch [id]. When
    recording is off this is just [f ()] — no path bookkeeping at all. *)
val with_arm : int -> int -> (unit -> 'a) -> 'a

(** Record one decision for a linear variable. Ignored when recording is off or
    no function is open. *)
val record : var:string -> ty:string -> depth:int -> event -> unit

(* Borrow nesting is *checker* state, not ledger state: {!LinearityCheck}
   counts it (`borrow_push`/`borrow_pop` in LinearityCheck.mli) because it
   also decides what a nested borrow scope restores, and tells this module
   when to record `Unborrow`. Keeping one counter where both decisions are
   made is the point — see docs/memsafe.md §5.1. *)

(** Resolve a function's rows into timelines, one per (variable, maximal
    path). A row belongs to a timeline when its path is a prefix of the
    timeline's path — that is, when the event is on that execution path.
    Timelines are ordered by path then by first appearance of the variable;
    maximal paths are ordered by their label. *)
val timelines : row list -> timeline list

(** The human-readable form of a branch path: ["entry"] when empty, else
    ["b0.a0/b2.a1"] (outermost first). Deterministic, so it is safe to pin in
    an artifact. *)
val path_label : (int * int) list -> string

(** Write the ledger for module [name] to [path], merging with any module
    entries already in that file (the entry for [name] is replaced).

    Each module entry is
    {{{
    {"name":"Test","functions":[
        {"name":"main","kind":"function","timelines":[
          {"var":"r","ty":"R","path":"entry",
           "events":["acquire","consume"]}]}]}
    }}}
    Functions are in declaration order (the order the checker walked them) and
    modules are sorted by name, so a multi-module build is byte-deterministic.
    This is the pinned artifact [scripts/ledger2mm0] consumes. *)
val write_json : path:string -> name:string -> unit

(** The same serialization, from a synthetic function list — the ledger unit
    test and the emitter's self-test use it to build a ledger whose timelines
    have been tampered with. *)
val to_json_string : name:string -> functions:(string * string * row list) list -> string
