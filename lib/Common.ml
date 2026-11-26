(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
(** Simple types used throughout the codebase. *)

(** The four arithmetic operators. *)
type arithmetic_operator =
  | Add
  | Subtract
  | Multiply
  | Divide
[@@deriving (show, sexp)]

(** The comparison operators. *)
type comparison_operator =
  | Equal
  | NotEqual
  | LessThan
  | LessThanOrEqual
  | GreaterThan
  | GreaterThanOrEqual
[@@deriving (show, sexp)]

(** The type of documentation strings. *)
type docstring = Docstring of string

(** The type of declaration pragmas. *)
type pragma =
  | ForeignImportPragma of string
  | ForeignExportPragma of string
  | UnsafeModulePragma
  (** Liquid contract pragmas (docs/LIQUID.md §3): the first component is
      the kind ("Requires", "Ensures", "Invariant", "Trusted", "Measure",
      "Fold"), the second the contract string ("" for marker pragmas). *)
  | LiquidPragma of string * string
  (** PLAN_valen §4, tier 1 (`extern rust`): ownership annotations for an
      `rs_*` foreign import. The first component is one annotation per
      parameter ("borrowed" or "owned"), the second the return value's
      annotation. Checked on the typed AST and erased before CPS: the
      monomorphized foreign declaration carries no pragmas. *)
  | RustOwnershipPragma of string list * string
  (** U13: a foreign import that brings an owned value across as a handle. The
      import's external name is the handle's *target*; the lowering derives
      `<target>__dispose` for the call and emits an `abi handle 1` descriptor
      for `<target>__new` / `<target>__dispose`, so neither wrapper name is
      written in Austral source. The components are the object type name the
      registry compares handles against, the provider's constructor and
      destructor symbols, and the constructor's arity in 64-bit words. The
      optional last component is the provider's reader (`*mut c_void -> i64`),
      which the generated `<target>__use` wrapper calls; without it the handle
      can be created and destroyed but not read. The optional sixth component
      is the provider's writer (a pointer and one `i64` word), which the
      generated `<target>__borrow_mut`/`<target>__write` wrappers use; without
      it a module can hold a read-only view but cannot write. *)
  | HandleOwnershipPragma of string * string * string * int * string option * string option

(** Whether a variable is immutable or mutable. *)
type mutability =
  | Immutable
  | Mutable
[@@deriving (show)]

(** Whether a reference is a read reference or a write reference. *)
type borrowing_mode =
  | ReadBorrow
  | WriteBorrow
[@@deriving (show, sexp)]

(** Whether a borrow statement is borrowing as read, write, or reborrow. *)
type borrow_stmt_kind =
  | Read
  | Write
  | Reborrow
[@@deriving (show, sexp)]

(** Whether a module is safe or unsafe. *)
type module_kind =
  | SafeModule
  | UnsafeModule

(** The visibility of a type: public, opaque, or private. *)
type type_vis =
  | TypeVisPublic
  | TypeVisOpaque
  | TypeVisPrivate

(** The visibility of another declaration: public or private. *)
type vis =
  | VisPublic
  | VisPrivate
