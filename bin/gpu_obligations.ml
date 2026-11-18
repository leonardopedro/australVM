(* G13 obligation emitter: reads a published bundle, writes the WhyML and MM0
   views beside a manifest. The compiler itself is not a prover; this tool
   only emits the checked artifacts and refuses a bundle that is not the
   admitted v1 identity wrapper. *)

let usage () =
  prerr_endline
    "usage: gpu_obligations emit BUNDLE OUTDIR\n\
     \       gpu_obligations check BUNDLE";
  exit 2

let write path contents =
  let oc = open_out_bin path in
  Fun.protect
    ~finally:(fun () -> close_out oc)
    (fun () -> output_string oc contents)

let emit bundle outdir =
  let t =
    try Gpu_obligations.of_bundle bundle
    with Gpu_obligations.Error msg ->
      prerr_endline ("gpu_obligations: " ^ msg);
      exit 1
  in
  if Sys.file_exists outdir then begin
    prerr_endline
      (Printf.sprintf "gpu_obligations: refusing to reuse existing output %s" outdir);
    exit 1
  end;
  Unix.mkdir outdir 0o755;
  write (Filename.concat outdir "gpu_memory.mlw") (Gpu_obligations.render_whyml t);
  write (Filename.concat outdir "gpu_memory.mm0") (Gpu_obligations.render_mm0 t);
  write (Filename.concat outdir "gpu_memory.mm1") (Gpu_obligations.render_mm1 t);
  write (Filename.concat outdir "manifest.json")
    (Yojson.Safe.pretty_to_string (Gpu_obligations.manifest_json t) ^ "\n");
  Printf.printf
    "emitted bundle=%s ir=%s wgsl=%s plan=%s\n"
    outdir t.Gpu_obligations.ir_sha256 t.Gpu_obligations.wgsl_sha256
    t.Gpu_obligations.plan_sha256

let check bundle =
  let t =
    try Gpu_obligations.of_bundle bundle
    with Gpu_obligations.Error msg ->
      prerr_endline ("gpu_obligations: " ^ msg);
      exit 1
  in
  Printf.printf
    "bundle=%s entry=%s arity=%d workgroup_size=%d max_elements=%d ir=%s wgsl=%s plan=%s\n"
    bundle t.Gpu_obligations.entry t.Gpu_obligations.input_arity
    t.Gpu_obligations.workgroup_size t.Gpu_obligations.max_elements
    t.Gpu_obligations.ir_sha256 t.Gpu_obligations.wgsl_sha256
    t.Gpu_obligations.plan_sha256

let () =
  match Array.to_list Sys.argv with
  | [_; "emit"; bundle; outdir] -> emit bundle outdir
  | [_; "check"; bundle] -> check bundle
  | _ -> usage ()
