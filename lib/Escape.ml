(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open Util
open Error
open Sexplib
open Std

type escaped_string = EscapedString of string
[@@deriving (show, sexp)]

(* Iterative, over the string, into one buffer.

   This was `string_implode (escape_list (string_explode s))`: `string_explode`
   built a cons cell per character and `escape_list` recursed once per cell, not
   in tail position — so the depth was proportional to the literal's length, and
   `escape_list` was never called with an empty accumulator to close over. A
   multi-megabyte literal (a base64 blob, an `@embed` payload) was a
   `Stack_overflow` waiting to happen, on top of the O(n^2) `string_implode`.

   The rewrite is behaviour-preserving; `CRendererTest` pins the interesting
   cases (quote, escaped quote, whitespace continuation). *)
(* `escape_list` / `consume_whitespace`, the char-list version this replaced,
   were removed: nothing else in the tree called them. *)
let escape_string (s: string) : escaped_string =
  let n = String.length s in
  let b = Buffer.create n in
  let i = ref 0 in
  let c i = String.get s i in
  (* `\` followed by whitespace: drop the whitespace run up to and including the
     closing `\`, exactly as `consume_whitespace` did. *)
  let skip_whitespace () =
    let continue_ = ref true in
    while !continue_ do
      match if !i < n then c !i else '\000' with
      | ' ' | '\n' | '\r' | '\t' -> incr i
      | '\\' -> incr i; continue_ := false
      | _ -> err "Bad whitespace escape sequence"
    done
  in
  while !i < n do
    (match c !i with
     | '\\' when !i + 1 < n ->
         (match c (!i + 1) with
          | 'n'  -> Buffer.add_char b '\n'; i := !i + 2
          | 'r'  -> Buffer.add_char b '\r'; i := !i + 2
          | 't'  -> Buffer.add_char b '\t'; i := !i + 2
          | '\'' -> Buffer.add_char b '\''; i := !i + 2
          | '"'  -> Buffer.add_char b '"';  i := !i + 2
          | '\\' -> Buffer.add_char b '\\'; i := !i + 2
          | (' ' | '\n' | '\r' | '\t') -> i := !i + 1; skip_whitespace ()
          | _ ->
              (* A trailing lone backslash, or one before an unescapable
                 character: keep it, as the list version did by falling through
                 to the `(head :: rest)` case on the next cell. *)
              Buffer.add_char b '\\'; incr i)
     | ch -> Buffer.add_char b ch; incr i)
  done;
  EscapedString (Buffer.contents b)

let escaped_to_string (EscapedString s) =
  s

let rec unescape_string (EscapedString s) =
  String.concat "" (List.map unescape_char (string_explode s))

and unescape_char = function
  | '\n' -> "\\n"
  | '\r' -> "\\r"
  | '\t' -> "\\t"
  | '\'' -> "\\\'"
  | '\\' -> "\\\\"
  | '"' -> "\\\""
  | c -> String.make 1 c
