(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
(** Executes parsed CLI commands. *)
open CliParser

val exec : cmd -> unit

(** Render an `execute_function` result for the REPL: the JIT_PANIC sentinel
    (i64::MIN) with a recorded reason becomes `ERROR <reason>`; anything else
    becomes `RESULT <value>`. Pure; pinned by the test suite. *)
val format_exec_result : int64 -> string option -> string
