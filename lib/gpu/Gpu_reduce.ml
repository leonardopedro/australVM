(*
   G22a: the fixed `map-sum-u32` reduction — its definition and CPU schedule.

   The operation is one `Nat32 -> Nat32` mapper from the Austral source and an
   explicit modular sum modulo 2^32. Nothing here emits a shader: this module is
   the plan both the WGSL emitter and the Rust schedule have to agree with, so
   the counting and padding are decided once, with checked arithmetic, instead
   of once per consumer.

   The schedule is a list of *output counts*, pass 0 first. Pass 0 applies the
   mapper; later passes only add partial sums. A pass of `n > 0` produces
   `ceil(n/64)` outputs, so a singleton still runs pass 0 and the empty input
   runs no pass at all (the result is the identity, 0, and no binding is made).

   The partition is `i = 64 * group + lane` with `0 <= lane < 64`; a group's
   last tile is short, and its missing lanes contribute the identity — they are
   *not* mapped, which is why the oracle uses a mapper with `f(0) <> 0`.
*)

let operation = "map-sum-u32"
let profile = "reduce-u32-v1"
let tile = 64
let identity = 0
let max_inputs = 1_048_576
let bytes_per_element = 4
(* One workgroup's scratch: `tile` u32 words. *)
let scratch_bytes = tile * bytes_per_element

type error =
  | Negative_input of int
  | Oversized_input of int
  | Capacity_overflow of string

let error_message = function
  | Negative_input n -> Printf.sprintf "input count %d is negative" n
  | Oversized_input n ->
      Printf.sprintf "input count %d exceeds the %d-element v1 maximum" n
        max_inputs
  | Capacity_overflow what -> Printf.sprintf "%s does not fit in a host int" what

let checked_inputs n =
  if n < 0 then Error (Negative_input n)
  else if n > max_inputs then Error (Oversized_input n)
  else Ok n

let ceil_div a b = (a + b - 1) / b

(* Output count of each pass, pass 0 first; `[]` for empty input. *)
let schedule n : (int list, error) result =
  match checked_inputs n with
  | Error e -> Error e
  | Ok 0 -> Ok []
  | Ok n ->
      (* Pass 0 reads the caller's `n` bytes; every later pass reads the
         previous pass's output count, and a pass that outputs one element is
         the last one. `n = 0` is handled above: no pass, no binding. *)
      let rec go out acc =
        if out <= 1 then List.rev (out :: acc)
        else go (ceil_div out tile) (out :: acc)
      in
      Ok (go (ceil_div n tile) [])

let pass_count n = match schedule n with Ok s -> List.length s | Error _ -> 0

(* Byte capacity of one pass's input or output. `schedule` already bounds the
   count, so this only has to refuse an int that would overflow the multiply. *)
let capacity_bytes n : (int, error) result =
  match checked_inputs n with
  | Error e -> Error e
  | Ok n ->
      if n > max_int / bytes_per_element then
        Error (Capacity_overflow (Printf.sprintf "%d elements" n))
      else Ok (n * bytes_per_element)

(* The host's total allocation for a run: every pass's outputs, plus the
   caller's input. Distinct storage per pass, so nothing is shared or reused. *)
let total_bytes n : (int, error) result =
  match (checked_inputs n, schedule n, capacity_bytes n) with
  | Error e, _, _ | _, Error e, _ | _, _, Error e -> Error e
  | Ok _, Ok counts, Ok input_bytes ->
      let outputs = List.fold_left ( + ) 0 counts in
      if outputs > max_int / bytes_per_element then
        Error (Capacity_overflow (Printf.sprintf "%d pass outputs" outputs))
      else Ok (input_bytes + (outputs * bytes_per_element))

(* The only index a `(group, lane)` pair names, or `None` for a lane outside a
   workgroup. Coverage (each valid index exactly once) is a property of the
   full `(group, lane)` space; this is the definition both sides share. *)
let partition_index ~group ~lane =
  if group < 0 || lane < 0 || lane >= tile then None
  else Some ((tile * group) + lane)

let program_text =
  Printf.sprintf
    "operation=%s\nprofile=%s\nmapper=scalar:Nat32->Nat32\n\
     operator=wrapping-add-u32\nidentity=%d\ntile=%d\nscratch-bytes=%d\n\
     max-inputs=%d\n"
    operation profile identity tile scratch_bytes max_inputs

let program_id = Gpu_ir.sha256_hex program_text

(* A launch plan is per input length: the pass counts and byte capacities are
   derived from the validated input, never accepted from the artifact. *)
let launch_plan_text n : (string, error) result =
  match (schedule n, capacity_bytes n) with
  | Error e, _ | _, Error e -> Error e
  | Ok counts, Ok input_bytes ->
      let buffer = Buffer.create 256 in
      Buffer.add_string buffer (Printf.sprintf "program=%s\n" program_id);
      Buffer.add_string buffer (Printf.sprintf "inputs=%d\n" n);
      Buffer.add_string buffer (Printf.sprintf "passes=%d\n" (List.length counts));
      let previous = ref input_bytes in
      List.iteri
        (fun pass outputs ->
          let input_bytes = !previous in
          let output_bytes = outputs * bytes_per_element in
          Buffer.add_string buffer
            (Printf.sprintf
               "pass=%d inputs=%d outputs=%d input-bytes=%d output-bytes=%d\n"
               pass (input_bytes / bytes_per_element) outputs input_bytes
               output_bytes);
          previous := output_bytes)
        counts;
      Ok (Buffer.contents buffer)

let launch_plan_id n =
  match launch_plan_text n with
  | Ok text -> Ok (Gpu_ir.sha256_hex text)
  | Error e -> Error e
