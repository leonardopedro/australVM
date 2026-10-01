(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L2: the TRL parser. Hand-written recursive descent over
   the contract string carried by a `Liquid_*` pragma, producing the
   `LiquidTypes` AST. The grammar is normative in `docs/LIQUID.md` §2.2:

     trl-term    ::= int-lit | real-lit | "true" | "false" | var
                   | trl-term "+" trl-term | trl-term "-" trl-term
                   | trl-term "*" trl-term | "-" trl-term
                   | "if" trl-term "then" trl-term "else" trl-term
                   | fun-name "(" [trl-term ("," trl-term)*] ")"
     trl-formula ::= trl-term relop trl-term | "!" trl-formula
                   | trl-formula ("&&" | "||" | "==>") trl-formula
                   | "(" trl-formula ")" | "true" | "false"

   Two deliberate restrictions, both normative:

   - **no quantifiers** (§2.2: `forall`/`exists` are MUST NOT in v1), so
     `forall`/`exists` are rejected as reserved words rather than silently
     treated as variables;
   - integer literals MUST lie in [-2^62, 2^62) (§2.2), enforced here.

   Errors carry a `LiquidTypes.span` into the contract string so the compiler
   can point at the offending characters.
*)

(* The lexer and the parser share one cursor: an offset into the string plus
   the end offset. `LiquidTypes.no_span` is used for "not applicable". *)
type state = {
  src   : string;
  mutable pos : int;
  (** Is this name a type that is *in scope*? See `parse_ty_arg`: a bracketed
      argument that is neither a primitive sort nor a known type is read as an
      index term, and the difference between `Vector[T, n]` and `Vector[n]` is
      not decidable from the token stream alone. Resolving it against the
      module's declared types is what makes the plan's syntax mean what it looks
      like it means, and it is scope-sensitive for the same reason WF1 is: a
      contract may only mention names its declaration has. *)
  mutable is_known_type : string -> bool;
}

let err_at (st : state) (start : int) (stop : int) (msg : string) : 'a =
  failwith
    (Printf.sprintf "Liquid contract error at %d-%d: %s\n  in: %s"
       start stop msg st.src)

let len (st : state) = String.length st.src

(* ── lexing helpers ─────────────────────────────────────────────────────── *)

let is_digit c = c >= '0' && c <= '9'

let is_ident_start c =
  (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c = '_'

let is_ident_char c = is_ident_start c || is_digit c

let skip_ws (st : state) =
  while st.pos < len st && (st.src.[st.pos] = ' ' || st.src.[st.pos] = '\t'
                            || st.src.[st.pos] = '\n' || st.src.[st.pos] = '\r')
  do
    st.pos <- st.pos + 1
  done

let at_end (st : state) = st.pos >= len st

let peek (st : state) =
  if at_end st then None else Some st.src.[st.pos]

(** Two-character lookahead for the operators §2.2 needs. *)
let peek2 (st : state) =
  if st.pos + 1 < len st then Some (String.sub st.src st.pos 2) else None

let starts_with (st : state) (s : string) =
  let n = String.length s in
  st.pos + n <= len st && String.sub st.src st.pos n = s

let eat (st : state) (s : string) =
  if starts_with st s then (st.pos <- st.pos + String.length s; true) else false

let expect (st : state) (s : string) (start : int) =
  if not (eat st s) then
    err_at st start st.pos (Printf.sprintf "expected %S" s)

(* ── reserved words (§2.2: no quantifiers in v1) ────────────────────────── *)

let reserved =
  [ "forall"; "exists"; "lambda"; "fun"; "let"; "match" ]

(* ── the two integer bounds of §2.2 ─────────────────────────────────────── *)

(* [-2^62, 2^62): computed rather than written so the constant cannot drift
   from the documented range. *)
let int_min = Z.neg (Z.shift_left Z.one 62)
let int_max = Z.sub (Z.shift_left Z.one 62) Z.one

let check_int_range (z : Z.t) (start : int) (stop : int) (st : state) =
  if Z.compare z int_min < 0 || Z.compare z int_max > 0 then
    err_at st start stop
      (Printf.sprintf "integer literal out of range: must lie in [%s, %s)"
         (Z.to_string int_min) (Z.to_string int_max))

(* ── terms ──────────────────────────────────────────────────────────────── *)

(** `trl-term "+" trl-term`, `trl-term "-" trl-term`.

    §2.2 lists these and the parser had no level for them, so `x + 1 == y` did
    not parse at all: `parse_term` stopped after one primary, leaving `+ 1` to
    be read as trailing input. L9 surfaced this because an *index* is meant to be
    a term and a term could only be a literal or a variable.

    Left-associative, matching the arithmetic these stand for, and `*` binds
    tighter (below). Unary minus stays a prefix at the tighter level, so `-x` and
    `a - b` are both unambiguous without needing whitespace conventions. *)
let rec parse_term (st : state) : LiquidTypes.term =
  let lhs = parse_multiplicative st in
  let rec loop acc =
    skip_ws st;
    let start = st.pos in
    if eat st "+" then begin
      let rhs = parse_multiplicative st in
      loop (LiquidTypes.TAdd (acc, rhs, { LiquidTypes.start = start; stop = st.pos }))
    end
    else if eat st "-" then begin
      let rhs = parse_multiplicative st in
      loop (LiquidTypes.TSub (acc, rhs, { LiquidTypes.start = start; stop = st.pos }))
    end
    else acc
  in
  loop lhs

(** `trl-term "*" trl-term` — binds tighter than `+`/`-`. *)
and parse_multiplicative (st : state) : LiquidTypes.term =
  let lhs = parse_unary_term st in
  let rec loop acc =
    skip_ws st;
    let start = st.pos in
    if eat st "*" then begin
      let rhs = parse_unary_term st in
      loop (LiquidTypes.TMul (acc, rhs, { LiquidTypes.start = start; stop = st.pos }))
    end
    else acc
  in
  loop lhs

(** A primary, plus unary minus and the conditional form of §2.2. *)
and parse_unary_term (st : state) : LiquidTypes.term =
  skip_ws st;
  let start = st.pos in
  (* unary minus is a term on its own right in §2.2 *)
  if eat st "-" then begin
    let t = parse_unary_term st in
    LiquidTypes.TNeg (t, { LiquidTypes.start = start; stop = st.pos })
  end
  else if eat st "if" then begin
    let c = parse_term st in
    skip_ws st; expect st "then" (LiquidTypes.span_of_term c).LiquidTypes.start;
    let a = parse_term st in
    skip_ws st; expect st "else" (LiquidTypes.span_of_term a).LiquidTypes.start;
    let b = parse_term st in
    LiquidTypes.TIf (c, a, b, { LiquidTypes.start = start; stop = st.pos })
  end
  else
    match peek st with
    | Some c when is_digit c -> parse_int_or_real st start
    | Some c when is_ident_start c -> parse_var_or_call st start
    | _ ->
       err_at st start st.pos "expected a refinement term"

and parse_int_or_real (st : state) (start : int) : LiquidTypes.term =
  let digits_start = st.pos in
  while (match peek st with Some c when is_digit c -> true | _ -> false) do
    st.pos <- st.pos + 1
  done;
  (* A '.' followed by a digit makes it a Real; a bare '.' ends the literal so
     "0 <= x" does not try to read a fractional part. *)
  let is_real =
    (match peek st with Some '.' -> true | _ -> false)
    && (match peek2 st with Some s when String.length s = 2 -> is_digit s.[1] | _ -> false)
  in
  if is_real then begin
    st.pos <- st.pos + 1;
    let frac_start = st.pos in
    while (match peek st with Some c when is_digit c -> true | _ -> false) do
      st.pos <- st.pos + 1
    done;
    if frac_start = st.pos then
      err_at st digits_start st.pos "expected digits after the decimal point";
    let text = String.sub st.src start (st.pos - start) in
    match float_of_string_opt text with
    | Some f -> LiquidTypes.TReal (f, { LiquidTypes.start = start; stop = st.pos })
    | None -> err_at st start st.pos ("malformed real literal: " ^ text)
  end
  else
    let text = String.sub st.src digits_start (st.pos - digits_start) in
    (* The lexer above consumed digits only, so Z.of_string cannot fail here. *)
    let z = Z.of_string text in
    check_int_range z start st.pos st;
    LiquidTypes.TInt (z, { LiquidTypes.start = start; stop = st.pos })

and parse_var_or_call (st : state) (start : int) : LiquidTypes.term =
  while (match peek st with Some c when is_ident_char c -> true | _ -> false) do
    st.pos <- st.pos + 1
  done;
  let name = String.sub st.src start (st.pos - start) in
  if List.mem name reserved then
    err_at st start st.pos
      (Printf.sprintf "%S is not available in Liquid Austral v1: the TRL has no quantifiers or binders (§2.2)" name);
  (* `true`/`false` are literals of §2.2, not variables: a formula may be a
     bare `true`, and a term may be a boolean. They are recognised here so
     they never become a `TVar` with no declared sort. *)
  if (name = "true" || name = "false") then
    LiquidTypes.TBool
      (name = "true", { LiquidTypes.start = start; stop = st.pos })
  else begin
    skip_ws st;
    if eat st "(" then begin
      let args = ref [] in
      skip_ws st;
      if not (eat st ")") then begin
        let rec loop () =
          let a = parse_term st in
          args := a :: !args;
          skip_ws st;
          if eat st "," then (skip_ws st; loop ())
          else if not (eat st ")") then
            err_at st start st.pos "expected \",\" or \")\" in a function application"
        in
        loop ()
      end;
      LiquidTypes.TCall
        (name, List.rev !args, { LiquidTypes.start = start; stop = st.pos })
    end
    else
      LiquidTypes.TVar (name, None, { LiquidTypes.start = start; stop = st.pos })
  end

(* ── formulas ───────────────────────────────────────────────────────────── *)

let parse_relop (st : state) : LiquidTypes.relop option =
  (* Two-char operators first: ">=" must not be read as ">" then "=". *)
  if eat st "==" then Some LiquidTypes.REq
  else if eat st "!=" then Some LiquidTypes.RNe
  else if eat st "<=" then Some LiquidTypes.RLe
  else if eat st ">=" then Some LiquidTypes.RGe
  else if eat st "<" then Some LiquidTypes.RLt
  else if eat st ">" then Some LiquidTypes.RGt
  else None

(** Right-associative `||`, mirroring the precedence of the connective
    families in §2.2. `&&` binds tighter than `||`, which binds tighter than
    `==>`. *)
(** Recognised primitive sorts, spelled as they are in Austral. A type argument
    that is one of these is a *type*; anything else in an argument position is
    read as an index *term*. *)
let sort_of_name (name : string) : LiquidTypes.sort option =
  match name with
  | "Nat" | "Nat8" -> Some LiquidTypes.SNat
  | "Int" | "Int32" | "Int64" -> Some LiquidTypes.SInt
  | "Bool" -> Some LiquidTypes.SBool
  | "Real" | "Float64" -> Some LiquidTypes.SReal
  | "Prob" -> Some LiquidTypes.SProb
  | _ -> None


let rec parse_implies (st : state) : LiquidTypes.formula =
  let lhs = parse_or st in
  skip_ws st;
  if eat st "==>" then begin
    let rhs = parse_implies st in
    LiquidTypes.FImplies (lhs, rhs, { LiquidTypes.start = (LiquidTypes.span_of_formula lhs).LiquidTypes.start; stop = st.pos })
  end
  else lhs

and parse_or (st : state) : LiquidTypes.formula =
  let lhs = parse_and st in
  skip_ws st;
  if eat st "||" then begin
    let rhs = parse_or st in
    LiquidTypes.FOr (lhs, rhs, { LiquidTypes.start = (LiquidTypes.span_of_formula lhs).LiquidTypes.start; stop = st.pos })
  end
  else lhs

and parse_and (st : state) : LiquidTypes.formula =
  let lhs = parse_unary st in
  skip_ws st;
  if starts_with st "&&" && eat st "&&" then begin
    let rhs = parse_and st in
    LiquidTypes.FAnd (lhs, rhs, { LiquidTypes.start = (LiquidTypes.span_of_formula lhs).LiquidTypes.start; stop = st.pos })
  end
  else lhs

and parse_unary (st : state) : LiquidTypes.formula =
  skip_ws st;
  let start = st.pos in
  if eat st "!" then begin
    let g = parse_unary st in
    LiquidTypes.FNot (g, { LiquidTypes.start = start; stop = st.pos })
  end
  else if eat st "(" then begin
    (* Either a parenthesised formula or a parenthesised term compared
       against one; try the formula reading, and fall back. *)
    let save = st.pos in
    let g =
      try parse_implies st
      with Failure _ -> st.pos <- save; parse_relational st
    in
    skip_ws st;
    expect st ")" (LiquidTypes.span_of_formula g).LiquidTypes.start;
    g
  end
  else parse_relational st

and parse_relational (st : state) : LiquidTypes.formula =
  skip_ws st;
  let start = st.pos in
  let lhs = parse_term st in
  skip_ws st;
  match parse_relop st with
  | Some op ->
     let rhs = parse_term st in
     LiquidTypes.FRel
       (op, lhs, rhs, { LiquidTypes.start = start; stop = st.pos })
  | None when eat st ":" ->
     (* L9: `e : T` — a type annotation over an index-carrying type. *)
     let ty = parse_ty st in
     LiquidTypes.FType
       (lhs, ty, { LiquidTypes.start = start; stop = st.pos })
  | None ->
     (* No relop: §2.2 allows a bare "true"/"false" as a formula. *)
     (match lhs with
      | LiquidTypes.TBool (true, sp) -> LiquidTypes.FTrue sp
      | LiquidTypes.TBool (false, sp) -> LiquidTypes.FFalse sp
      | _ ->
         err_at st start st.pos
           "expected a relation: a formula is a comparison, a conjunction, \
            a disjunction, an implication, a type annotation, a negation, or \
            true/false")

(* ── L9: types ───────────────────────────────────────────────────────────── *)

and parse_ty (st : state) : LiquidTypes.ty =
  skip_ws st;
  let start = st.pos in
  (match peek st with
   | Some c when is_ident_start c ->
       while (match peek st with Some c when is_ident_char c -> true | _ -> false) do
         st.pos <- st.pos + 1
       done;
       let name = String.sub st.src start (st.pos - start) in
       (match sort_of_name name with
        | Some s -> LiquidTypes.TySort s
        | None ->
            skip_ws st;
            if eat st "[" then begin
              let args = ref [] in
              skip_ws st;
              if not (eat st "]") then begin
                let rec loop () =
                  let a = parse_ty_arg st in
                  args := a :: !args;
                  skip_ws st;
                  if eat st "," then (skip_ws st; loop ())
                  else if not (eat st "]") then
                    err_at st start st.pos
                      "expected \",\" or \"]\" in a type argument list"
                in
                loop ()
              end;
              LiquidTypes.TyApp
                (name, List.rev !args, { LiquidTypes.start = start; stop = st.pos })
            end
            else LiquidTypes.TyApp (name, [], { LiquidTypes.start = start; stop = st.pos }))
   | _ -> err_at st start st.pos "expected a type after `:`")

(** One bracketed type argument: a primitive sort is a type, anything else is an
    index term.

    This is a deliberate, documented limitation rather than a parser quirk. The
    two cases are genuinely ambiguous from the token stream alone —
    `Vector[Foo]` could be a vector of the named type `Foo` or one indexed by a
    variable — and picking either silently would let `Vector[Bar]` mean
    different things in different contracts. Reading a non-sort as an index
    keeps the rule predictable: sorts are types, everything else is a term.

    The cost is that a *named* (non-sort) type argument is not yet expressible
    inside brackets, which is what `Vector[T, n]` from the plan needs. Writing
    it today requires the named type to be spelled as a sort; L9 leaves that
    gap recorded rather than papering over it with a guess. *)
and parse_ty_arg (st : state) : LiquidTypes.ty_arg =
  skip_ws st;
  let start = st.pos in
  let identifier_at () =
    match peek st with
    | Some c when is_ident_start c ->
      while (match peek st with Some c when is_ident_char c -> true | _ -> false) do
        st.pos <- st.pos + 1
      done;
      Some (String.sub st.src start (st.pos - start))
    | _ -> None
  in
  match identifier_at () with
  | Some name -> (
    match sort_of_name name with
    | Some s -> LiquidTypes.TyArgTy (LiquidTypes.TySort s)
    | None when st.is_known_type name ->
      (* A declared type, in scope. `Vector[T, n]` from the plan needs exactly
         this: without it, `T` reads as an index term and the contract silently
         means something else. *)
      LiquidTypes.TyArgTy
        (LiquidTypes.TyApp
           (name, [], { LiquidTypes.start = start; stop = st.pos }))
    | None ->
      (* Not a sort, so it is an index term. Rewind and let the term parser
         have the whole thing, which is what makes `Span[Nat8, n + 1]` work:
         the index is an arbitrary expression, not just a bare name. *)
      st.pos <- start;
      let t = parse_term st in
      LiquidTypes.TyArgIndex t)
  | None ->
    (* Not an identifier at all: the argument can only be an index term. *)
    let t = parse_term st in
    LiquidTypes.TyArgIndex t

(* ── entry points ───────────────────────────────────────────────────────── *)

(** Parse a whole contract string. The string must be consumed exactly: a
    trailing token is an error, not silently ignored, because a typo in a
    contract must not read as a weaker contract. *)
let parse_formula ?(is_known_type = fun _ -> false) (src : string)
  : LiquidTypes.formula =
  let st = { src; pos = 0; is_known_type } in
  skip_ws st;
  if at_end st then
    err_at st 0 0 "empty contract: expected a refinement formula";
  let f = parse_implies st in
  skip_ws st;
  if not (at_end st) then
    err_at st st.pos (len st) "unexpected trailing input after the formula";
  f

(** Parse the contract of a pragma kind. Marker kinds ([KMeasure], [KFold])
    carry no formula and so reject any non-empty contract string. *)
let parse_contract ?(is_known_type = fun _ -> false)
    (kind : LiquidTypes.contract_kind) (src : string)
  : LiquidTypes.contract =
  match kind with
  | LiquidTypes.KMeasure | LiquidTypes.KFold ->
     if String.trim src = "" then
       { kind; source = src; formula = None }
     else
       err_at { src; pos = 0; is_known_type } 0 (String.length src)
         (Printf.sprintf "%s is a marker pragma and takes no contract"
            (LiquidTypes.string_of_contract_kind kind))
  | _ ->
     { kind; source = src; formula = Some (parse_formula ~is_known_type src) }

(** Parse the contract of a pragma kind, given the kind already resolved from
    its pragma name. Marker kinds ([KMeasure], [KFold]) carry no formula and
    so reject any non-empty contract string. Raises [Failure] with the span
    formatted into the message. *)
let parse_contract_exn ?(is_known_type = fun _ -> false) (kind_name : string)
    (src : string) : LiquidTypes.contract =
  match LiquidTypes.contract_kind_of_string kind_name with
  | None ->
     err_at { src; pos = 0; is_known_type } 0 (String.length src)
       (Printf.sprintf "unknown Liquid pragma kind %S" kind_name)
  | Some kind -> parse_contract ~is_known_type kind src

(** Parse the contract string carried by a `LiquidPragma (kind_name, src)`.
    Returns [None] for a kind name that is not one of the six. *)
let parse_pragma ?(is_known_type = fun _ -> false) (kind_name : string) (src : string)
  : LiquidTypes.contract option =
  match LiquidTypes.contract_kind_of_string kind_name with
  | None -> None
  | Some kind -> Some (parse_contract ~is_known_type kind src)