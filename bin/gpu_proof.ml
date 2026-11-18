(* PC04: emit or check the source → KernelIR reflection artifact.

   `emit <aui> <aum> <entry> <outdir>` compiles the real fixture, builds the
   actual KernelIR, reflects both sides, requires them to agree and writes
   `clamp_source.mm0` / `.mm1` (the names are fixed: one reflected fixture).

   `check <aui> <aum> <entry> <artifact.mm0>` re-reflects and refuses an
   artifact whose recorded reflection lines or `ir_clamp` definition disagree,
   regardless of the digests recorded on it.

   `--tamper ir-param` / `--tamper ir-compare` mutate the IR before reflection
   and are used by the test to show that a wrong parameter mapping or a changed
   IR comparison is refused rather than reflected. *)

let read_file path =
  let ic = open_in_bin path in
  Fun.protect ~finally:(fun () -> close_in ic) (fun () ->
      let n = in_channel_length ic in
      really_input_string ic n)

let write_file path contents =
  let oc = open_out_bin path in
  output_string oc contents;
  close_out oc

let compiled ~aui ~aum =
  let env = Compiler.cenv Compiler.empty_compiler in
  let source =
    Compiler.TwoFileModuleSource
      { int_filename = aui;
        int_code = read_file aui;
        body_filename = aum;
        body_code = read_file aum }
  in
  let (_, p) = Compiler.prepare_module env source in
  (p.Compiler.prepared_env, p.Compiler.prepared_mono)

let sha256 s = Digestif.SHA256.(to_hex (digest_string s))

let tamper_compare (k: Gpu_ir.kernel) =
  let body =
    match k.Gpu_ir.body with
    | Gpu_ir.If (Gpu_ir.Compare (Gpu_ir.GreaterThan, a, b), t, f) ->
       Gpu_ir.If (Gpu_ir.Compare (Gpu_ir.LessThan, a, b), t, f)
    | Gpu_ir.If (Gpu_ir.Compare (Gpu_ir.LessThan, a, b), t, f) ->
       Gpu_ir.If (Gpu_ir.Compare (Gpu_ir.GreaterThan, a, b), t, f)
    | body -> body
  in
  { k with Gpu_ir.body }

let tamper_param (k: Gpu_ir.kernel) =
  let body =
    match k.Gpu_ir.body with
    | Gpu_ir.If (Gpu_ir.Compare (op, Gpu_ir.Param _, b), t, f) ->
       Gpu_ir.If (Gpu_ir.Compare (op, Gpu_ir.Param 1, b), t, f)
    | body -> body
  in
  { k with Gpu_ir.body }

let emitted ~aui ~aum ~entry ~tamper =
  let (env, mono) = compiled ~aui ~aum in
  let kernel = Gpu_subset.build ~entry env mono in
  let kernel =
    match tamper with
    | None -> kernel
    | Some "ir-param" -> tamper_param kernel
    | Some "ir-compare" -> tamper_compare kernel
    | Some other -> failwith ("unknown tamper " ^ other)
  in
  let source = Gpu_proof_interface.of_typed ~entry mono in
  let ir = Gpu_proof_interface.of_ir ~entry kernel in
  let r = Gpu_proof_interface.witness source ir in
  let source_sha256 = sha256 (read_file aui ^ read_file aum) in
  let ir_sha256 = Gpu_ir.sha256_hex (Gpu_ir.canonical_bytes kernel) in
  (r, Gpu_proof_interface.render ~source_sha256 ~ir_sha256 r)

let () =
  match Array.to_list Sys.argv with
  | [_; "emit"; aui; aum; entry; _outdir; "--tamper"; tamper] ->
     let (r, _) = emitted ~aui ~aum ~entry ~tamper:(Some tamper) in
     print_endline (Gpu_proof_interface.describe r);
     print_endline (Printf.sprintf "tamper=%s" tamper)
  | [_; "emit"; aui; aum; entry; outdir] ->
     let (r, (mm0, mm1)) = emitted ~aui ~aum ~entry ~tamper:None in
     write_file (Filename.concat outdir "clamp_source.mm0") mm0;
     write_file (Filename.concat outdir "clamp_source.mm1") mm1;
     print_endline (Gpu_proof_interface.describe r);
     print_endline
       (Printf.sprintf "[pass] reflected %s into %s/clamp_source.mm0"
          entry outdir)
  | [_; "check"; aui; aum; entry; artifact] ->
     let (r, _) = emitted ~aui ~aum ~entry ~tamper:None in
     Gpu_proof_interface.check_artifact r (read_file artifact);
     print_endline
       (Printf.sprintf "[pass] %s records the fresh reflection of %s" artifact entry)
  | [_; "dump-ir"; aui; aum; entry] ->
     let (env, mono) = compiled ~aui ~aum in
     let kernel = Gpu_subset.build ~entry env mono in
     print_endline (Gpu_ir.canonical_bytes kernel)
  | [_; "describe"; aui; aum; entry] ->
     let (r, _) = emitted ~aui ~aum ~entry ~tamper:None in
     print_endline (Gpu_proof_interface.describe r)
  | _ ->
     prerr_endline
       "usage: gpu_proof (emit <aui> <aum> <entry> <outdir> | \
        emit <aui> <aum> <entry> <outdir> --tamper ir-param|ir-compare | \
        check <aui> <aum> <entry> <artifact.mm0> | \
        describe <aui> <aum> <entry> | dump-ir <aui> <aum> <entry>)";
     exit 2
