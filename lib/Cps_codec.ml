(*************************************************************************)
(* The CPS-IR instruction byte table.                                    *)
(*                                                                       *)
(* The emitter (this directory) writes opcode bytes; the JIT's decoder    *)
(* (`safestos/cranelift/src/cps.rs`) reads them. Until this module        *)
(* existed, that table was written out as bare hex literals on the OCaml  *)
(* side, so "the encoder and the decoder agree" was a fact about two      *)
(* hand-edited lists with nothing tying them together.                    *)
(*                                                                       *)
(* Here the table is a value: [all] is the subset, [to_byte] is the       *)
(* encoder, [of_byte] the decoder, and `test/CpsCodecTest.ml` checks the  *)
(* round trip exhaustively over all 256 bytes and against the same table  *)
(* `mm0/isa.mm0` declares (which `scripts/isa-census` in turn checks      *)
(* against the Rust decoder). Three copies of one table is two too many,  *)
(* but they are now *checked* copies rather than three memories.          *)
(*                                                                       *)
(* `mm0/isa.mm0` proves `decode (encode i) = i` for each entry below, so  *)
(* a constructor added here needs an `-- op` line, an `ISA_REGISTRY`      *)
(* entry and a proved instance there; the gates say so.                   *)
(*************************************************************************)

(** One instruction of the emitted CPS subset, in opcode order. *)
type op =
  | Int_lit      (* 0x01 *)
  | Var          (* 0x02 *)
  | Let          (* 0x03 *)
  | App          (* 0x04 *)
  | Add          (* 0x05 *)
  | Sub          (* 0x06 *)
  | Return       (* 0x07 *)
  | If           (* 0x08 *)
  | While        (* 0x09 *)
  | Match        (* 0x0A *)
  | Cmp_lt       (* 0x10 *)
  | Cmp_gt       (* 0x11 *)
  | Cmp_lte      (* 0x12 *)
  | Cmp_gte      (* 0x13 *)
  | Cmp_eq       (* 0x14 *)
  | Cmp_neq      (* 0x15 *)
  | And          (* 0x16 *)
  | Or           (* 0x17 *)
  | Mul          (* 0x18 *)
  | Not          (* 0x19 *)
  | Deref        (* 0x20 *)
  | String_lit   (* 0x21 *)
  | Store        (* 0x30 *)

(** The subset, in the order `ISA_REGISTRY` and `mm0/isa.mm0` use. *)
let all = [
  Int_lit; Var; Let; App; Add; Sub; Return; If; While; Match;
  Cmp_lt; Cmp_gt; Cmp_lte; Cmp_gte; Cmp_eq; Cmp_neq;
  And; Or; Mul; Not; Deref; String_lit; Store;
]

(** The encoder: opcode -> the byte that introduces it. *)
let to_byte = function
  | Int_lit -> 0x01
  | Var -> 0x02
  | Let -> 0x03
  | App -> 0x04
  | Add -> 0x05
  | Sub -> 0x06
  | Return -> 0x07
  | If -> 0x08
  | While -> 0x09
  | Match -> 0x0A
  | Cmp_lt -> 0x10
  | Cmp_gt -> 0x11
  | Cmp_lte -> 0x12
  | Cmp_gte -> 0x13
  | Cmp_eq -> 0x14
  | Cmp_neq -> 0x15
  | And -> 0x16
  | Or -> 0x17
  | Mul -> 0x18
  | Not -> 0x19
  | Deref -> 0x20
  | String_lit -> 0x21
  | Store -> 0x30

(** The decoder: byte -> instruction, or [None] for a byte this subset does
    not use. Total over all 256 inputs on purpose — an opcode the decoder
    rejects must be *representable* as a rejection, not an exception. *)
let of_byte = function
  | 0x01 -> Some Int_lit
  | 0x02 -> Some Var
  | 0x03 -> Some Let
  | 0x04 -> Some App
  | 0x05 -> Some Add
  | 0x06 -> Some Sub
  | 0x07 -> Some Return
  | 0x08 -> Some If
  | 0x09 -> Some While
  | 0x0A -> Some Match
  | 0x10 -> Some Cmp_lt
  | 0x11 -> Some Cmp_gt
  | 0x12 -> Some Cmp_lte
  | 0x13 -> Some Cmp_gte
  | 0x14 -> Some Cmp_eq
  | 0x15 -> Some Cmp_neq
  | 0x16 -> Some And
  | 0x17 -> Some Or
  | 0x18 -> Some Mul
  | 0x19 -> Some Not
  | 0x20 -> Some Deref
  | 0x21 -> Some String_lit
  | 0x30 -> Some Store
  | _ -> None

(** The name the model, the census and the registry use. Kept identical to
    `ISA_REGISTRY`'s `name` field; `test/CpsCodecTest.ml` checks that. *)
let name = function
  | Int_lit -> "int_lit"
  | Var -> "var"
  | Let -> "let"
  | App -> "app"
  | Add -> "add"
  | Sub -> "sub"
  | Return -> "return"
  | If -> "if"
  | While -> "while"
  | Match -> "match"
  | Cmp_lt -> "cmp_lt"
  | Cmp_gt -> "cmp_gt"
  | Cmp_lte -> "cmp_lte"
  | Cmp_gte -> "cmp_gte"
  | Cmp_eq -> "cmp_eq"
  | Cmp_neq -> "cmp_neq"
  | And -> "and"
  | Or -> "or"
  | Mul -> "mul"
  | Not -> "not"
  | Deref -> "deref"
  | String_lit -> "string_lit"
  | Store -> "store"

(** Name -> instruction, for tools that read the tables by name. *)
let of_name n =
  let rec go = function
    | [] -> None
    | op :: rest -> if name op = n then Some op else go rest
  in
  go all
