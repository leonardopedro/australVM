(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
(** Executes parsed CLI commands. *)
open CliParser

val exec : cmd -> unit

(** Handle one JIT-server command line (`call`/`swap`/`exit`). Never raises:
    a command whose handling throws (e.g. a swap of a nonexistent module) is
    reported as `ERROR <reason>` on the protocol channel and the server keeps
    serving — one bad command must not kill the session, and the driving
    script must never be left waiting on a silent server exit. *)
val handle_server_line : string -> unit

(** Render an `execute_function` result for the REPL: the JIT_PANIC sentinel
    (i64::MIN) with a recorded reason becomes `ERROR <reason>`; anything else
    becomes `RESULT <value>`. Pure; pinned by the test suite. *)
val format_exec_result : int64 -> string option -> string
