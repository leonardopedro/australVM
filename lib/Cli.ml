(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
*)
open CliUtil
open CliParser
open CliEngine
open HtmlError
open Error

let rec main (args: string list): unit =
  try
    main' args;
    exit 0
  with Austral_error error ->
    Printf.eprintf "%s" (render_error_to_plain error);
    html_error_dump error;
    dump_and_die ()

and main' (args: string list): unit =
  (* S36 unified application/VM: the application boots by loading its
     plugins. `Vm_plugin.boot` registers the built-in compiler as the
     application's compiler plugin (which installs the Why3-derived gate as
     a plugin of that compiler); every compile request is then routed
     through the registry (`Vm_plugin.run_compiler` in CliEngine) instead of
     a hard-coded pipeline call. *)
  Vm_plugin.boot ();
  let arglist: arglist = parse_args args in
  let cmd: cmd = parse arglist in
  exec cmd

and dump_and_die _: unit =
  print_endline "Compiler call tree printed to calltree.html";
  Reporter.dump ();
  exit (-1)
