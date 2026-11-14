(*************************************************************************)
(* CpsCodecTest — the CPS opcode table is a codec, and its two halves     *)
(* are one table.                                                        *)
(*                                                                       *)
(* What this checks, and what it deliberately does not:                   *)
(*   - `of_byte (to_byte op) = Some op` for all 23 instructions;          *)
(*   - over all 256 bytes, exactly the 23 table bytes decode;             *)
(*   - names and bytes agree with `mm0/isa.mm0`'s `-- op 0x.. name` table, *)
(*     which `scripts/isa-census` checks against the Rust decoder.        *)
(* It does not re-decode a module's operands — that is the JIT decoder's  *)
(* job and the census's measurement; here the claim is about the *opcode  *)
(* byte*, which is what `mm0/isa.mm0`'s encdec theorems are about.        *)
(*************************************************************************)

open OUnit2

let printer_op op = Printf.sprintf "0x%02X %s" (Cps_codec.to_byte op)
    (Cps_codec.name op)

(* Every instruction survives the round trip. *)
let test_round_trip _ =
  List.iter
    (fun op ->
      assert_equal ~printer:(fun o -> match o with
          | Some x -> printer_op x
          | None -> "None")
        (Some op) (Cps_codec.of_byte (Cps_codec.to_byte op)))
    Cps_codec.all

(* The decoder is total: it answers for every byte, and accepts exactly the
   declared ones. An opcode silently accepted outside the table would be an
   instruction the model does not cover. *)
let test_decoder_is_total_and_exact _ =
  let declared = List.map Cps_codec.to_byte Cps_codec.all in
  let accepted = ref [] in
  for b = 0 to 255 do
    match Cps_codec.of_byte b with
    | Some _ -> accepted := b :: !accepted
    | None -> ()
  done;
  let accepted = List.sort compare !accepted in
  assert_equal ~printer:string_of_int 23 (List.length accepted);
  assert_equal ~printer:(fun l -> String.concat "," (List.map string_of_int l))
    (List.sort compare declared) accepted

(* 23 distinct bytes and 23 distinct names: a duplicate would make the table
   an ambiguous codec while still passing a per-entry round trip. *)
let test_injective _ =
  let bytes = List.sort compare (List.map Cps_codec.to_byte Cps_codec.all) in
  let names = List.sort compare (List.map Cps_codec.name Cps_codec.all) in
  assert_equal ~printer:string_of_int 23
    (List.length (List.sort_uniq compare bytes));
  assert_equal ~printer:string_of_int 23
    (List.length (List.sort_uniq compare names))

(* `of_name` is the inverse of `name`, for tools that read tables by name. *)
let test_names_are_reversible _ =
  List.iter
    (fun op ->
      assert_equal ~printer:(fun o -> match o with
          | Some x -> printer_op x
          | None -> "None")
        (Some op) (Cps_codec.of_name (Cps_codec.name op)))
    Cps_codec.all

(* The artifact's table and this table are the same table. `mm0/isa.mm0` is
   the file the ISA model and its census agree on, so if a constructor or a
   byte moves on one side only, this is where it surfaces without needing the
   Rust bridge. Parse `-- op 0xNN name` lines, ignoring every other line. *)
let parse_isa_ops path =
  let ic = open_in path in
  let re = Str.regexp "^-- op 0x\\([0-9A-Fa-f]+\\) \\([a-z_]+\\)$" in
  let acc = ref [] in
  (try
     while true do
       let line = input_line ic in
       if Str.string_match re line 0 then
         acc := (int_of_string ("0x" ^ Str.matched_group 1 line),
                 Str.matched_group 2 line) :: !acc
     done
   with End_of_file -> close_in ic);
  List.rev !acc

(* Where the staged artifact sits depends on how the test is run: `dune
   runtest` runs it inside `_build/default/test`, `dune exec` runs it in the
   build root. Both are legitimate; only a *missing* file is a failure. *)
let artifact_path () =
  match
    List.find_opt Sys.file_exists
      ["../mm0/isa.mm0"; "mm0/isa.mm0"; "../../mm0/isa.mm0"]
  with
  | Some p -> p
  | None -> "../mm0/isa.mm0"

let test_matches_isa_artifact _ =
  let path = artifact_path () in
  if not (Sys.file_exists path) then
    (* The gate is `scripts/verify-invariants`; a test that cannot see the
       artifact must say so rather than pass by vacuity. *)
    assert_failure (path ^ " not staged by dune — add it to the test deps")
  else begin
    let artifact = parse_isa_ops path in
    let mine = List.map (fun op -> (Cps_codec.to_byte op, Cps_codec.name op))
        Cps_codec.all in
    assert_equal ~printer:string_of_int
      23 (List.length artifact);
    assert_equal
      ~printer:(fun l ->
        String.concat " " (List.map (fun (b, n) -> Printf.sprintf "0x%02X:%s" b n) l))
      (List.sort compare mine) (List.sort compare artifact)
  end

let suite =
  "CpsCodec" >::: [
    "round trip over the 23 instructions" >:: test_round_trip;
    "decoder is total and exact over 256 bytes" >:: test_decoder_is_total_and_exact;
    "bytes and names are injective" >:: test_injective;
    "names are reversible" >:: test_names_are_reversible;
    "table matches mm0/isa.mm0" >:: test_matches_isa_artifact;
  ]

let () = run_test_tt_main suite
