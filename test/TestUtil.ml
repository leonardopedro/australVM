(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open OUnit2
open Identifier
open Span

let eq = assert_equal
let i = make_ident

let span l c l' c' =
  Span {
      filename = "";
      startp = Position {
                   line = l;
                   column = c;
                 };
      endp = Position {
                 line = l';
                 column = c';
               };
  }

let simplespan s =
  span 1 0 1 (String.length s)

(* A repository-relative path, resolved from wherever dune runs the test.

   Dune runs a test with its working directory set to the test's build
   directory, so `examples/gpu/pointwise/Pointwise.aum` only resolves once the
   stanza lists it in `(deps …)` *and* the test walks back up to the staged
   copy. Trying the candidates in order keeps the same call working from the
   repository root (where a human or a script runs the binary directly) and
   from `_build/default/test` (where dune does). *)
let fixture path =
  let candidates = [ path; "../" ^ path; "../../" ^ path ] in
  match List.find_opt Sys.file_exists candidates with
  | Some p -> p
  | None ->
     failwith
       (Printf.sprintf "fixture %s not found (tried %s); is it listed in (deps …)?"
          path (String.concat ", " candidates))
