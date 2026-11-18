(*
   Part of the Austral project, under the Apache License v2.0 with LLVM Exceptions.
   See LICENSE file for details.

   SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

   PLAN_liquid_types.md L11b acceptance, as executable cases. Every row of the
   plan's accept/refuse list is one test below, and the three conversion
   answers are asserted separately so "unknown" can never be read as
   "different" or as "convertible":

     accept  `n + 0` converts to `n`
     require checked evidence for `m + n = n + m` (not a conversion rule)
     refuse  `n + 1` where `n` is required
     refuse  an ill-sorted index
     refuse  a wrong element width
     refuse  an unbound index
     shadowed names resolve to the right binder
     a budget-exhausted comparison is unknown and fails a required check

   Plus the two structural claims the module is built on: canonical identity is
   independent of source spans and of source names, and diagnostics are not
   part of a term's identity.
*)

open LiquidTypes

let failures = ref 0

let ok name cond =
  if cond then Printf.printf "  ok   %s\n" name
  else begin
    incr failures;
    Printf.printf "  FAIL %s\n" name
  end

let contains ~needle haystack =
  let n = String.length needle and h = String.length haystack in
  let rec go i = i + n <= h && (String.sub haystack i n = needle || go (i + 1)) in
  n = 0 || go 0

let diag_string = function
  | Ok _ -> "<ok>"
  | Error d -> LiquidIndex.string_of_diag d

(* A formula surface for the fixtures, with spans that differ between cases so
   a span-dependent answer would show up. *)
let v ?(sp = LiquidTypes.no_span) ?sort name = TVar (name, sort, sp)
let i ?(sp = LiquidTypes.no_span) n = TInt (Z.of_int n, sp)
let add ?(sp = LiquidTypes.no_span) a b = TAdd (a, b, sp)

(* The environment of the acceptance fixtures: `n` and `m` are immutable
   natural inputs, `x8` is a Nat8-typed input, `x16` a Nat16 one, and `b` is a
   boolean so the ill-sorted case has something to be ill-sorted about. *)
let env =
  let env, n = LiquidIndex.bind LiquidIndex.empty ~name:"n" ~sort:SNat in
  let env, m = LiquidIndex.bind env ~name:"m" ~sort:SNat in
  let env, x8 =
    LiquidIndex.bind ~width:LiquidIndex.E8 env ~name:"x8" ~sort:SNat
  in
  let env, x16 =
    LiquidIndex.bind ~width:LiquidIndex.E16 env ~name:"x16" ~sort:SNat
  in
  let env, b = LiquidIndex.bind env ~name:"b" ~sort:SBool in
  (env, n, m, x8, x16, b)

let env, bn, bm, bx8, bx16, bb = env

let () =
  print_endline "LiquidIndex (L11b)";

  (* ── accept: `n + 0` converts to `n` ──────────────────────────────────── *)
  (match (LiquidIndex.infer env (add (v "n") (i 0)), LiquidIndex.infer env (v "n")) with
   | Ok sum, Ok n ->
      ok "n + 0 elaborates" true;
      (match LiquidIndex.convert env sum n with
       | LiquidIndex.Convertible -> ok "n + 0 converts to n" true
       | s ->
          incr failures;
          Printf.printf "  FAIL n + 0 converts to n (%s)\n"
            (LiquidIndex.string_of_status s))
   | Error d, _ ->
      incr failures;
      Printf.printf "  FAIL n + 0 elaborates (%s)\n" (diag_string (Error d))
   | _, Error d ->
      incr failures;
      Printf.printf "  FAIL n elaborates (%s)\n" (diag_string (Error d)));

  (* The right-spine rule: (n + 1) + 2 converts to n + 3. *)
  (match (LiquidIndex.infer env (add (add (v "n") (i 1)) (i 2)),
          LiquidIndex.infer env (add (v "n") (i 3)))
   with
   | Ok a, Ok b -> (
       match LiquidIndex.convert env a b with
       | LiquidIndex.Convertible -> ok "(n + 1) + 2 converts to n + 3" true
       | s ->
          incr failures;
          Printf.printf "  FAIL (n + 1) + 2 converts to n + 3 (%s)\n"
            (LiquidIndex.string_of_status s))
   | _ ->
      incr failures;
      Printf.printf "  FAIL right-spine fixtures elaborate\n");

  (* ── an equality beyond conversion needs checked evidence ─────────────── *)
  (match (LiquidIndex.infer env (add (v "m") (v "n")),
          LiquidIndex.infer env (add (v "n") (v "m")))
   with
   | Ok lhs, Ok rhs ->
      (match LiquidIndex.convert env lhs rhs with
       | LiquidIndex.Different _ -> ok "m + n is not convertible to n + m" true
       | s ->
          incr failures;
          Printf.printf "  FAIL m + n must not convert to n + m (%s)\n"
            (LiquidIndex.string_of_status s));
      let goal = LiquidIndex.goal_of env lhs rhs in
      let text = LiquidIndex.goal_text goal in
      ok "the commutativity goal is a stated equality"
        (text = "(v1 + v0) = (v0 + v1)");
      (match LiquidIndex.require goal ~checked:[] with
       | LiquidIndex.Unchecked g -> ok "no evidence means unchecked"
                                       (LiquidIndex.goal_text g = text)
       | LiquidIndex.Discharged law ->
          incr failures;
          Printf.printf "  FAIL no evidence must not discharge (%s)\n" law);
      (match LiquidIndex.require goal ~checked:[ ("add_comm", text) ] with
       | LiquidIndex.Discharged "add_comm" -> ok "matching evidence discharges" true
       | _ ->
          incr failures;
          Printf.printf "  FAIL matching evidence must discharge\n");
      (* Evidence for a *different* instance must not carry over. *)
      let other =
        LiquidIndex.goal_of env (Option.get (match LiquidIndex.infer env (v "n") with Ok x -> Some x | Error _ -> None)) lhs
      in
      (match LiquidIndex.require other ~checked:[ ("add_comm", text) ] with
       | LiquidIndex.Unchecked _ -> ok "evidence is bound to its own instance" true
       | LiquidIndex.Discharged law ->
          incr failures;
          Printf.printf "  FAIL evidence leaked to another instance (%s)\n" law)
   | _ ->
      incr failures;
      Printf.printf "  FAIL commutativity fixtures elaborate\n");

  (* ── refuse `n + 1` where `n` is required ─────────────────────────────── *)
  (match (LiquidIndex.infer env (v "n"), LiquidIndex.infer env (add (v "n") (i 1))) with
   | Ok want, Ok have ->
      (match LiquidIndex.require_convertible env have want with
       | Ok () ->
          incr failures;
          Printf.printf "  FAIL n + 1 must not satisfy a requirement for n\n"
       | Error d ->
          ok "n + 1 is refused where n is required"
            (contains ~needle:"indices differ" d.LiquidIndex.detail))
   | _ ->
      incr failures;
      Printf.printf "  FAIL n / n + 1 elaborate\n");

  (* ── refuse an ill-sorted index ───────────────────────────────────────── *)
  (match LiquidIndex.infer env (v "b") with
   | Error d ->
      ok "a boolean is ill-sorted as an index"
        (contains ~needle:"ill-sorted index" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL a boolean must not elaborate as an index\n");
  (match LiquidIndex.infer env (TReal (1.0, LiquidTypes.no_span)) with
   | Error d ->
      ok "a real is ill-sorted as an index"
        (contains ~needle:"ill-sorted index" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL a real must not elaborate as an index\n");

  (* ── refuse a wrong element width ─────────────────────────────────────── *)
  (match LiquidIndex.infer env (add (v "x8") (v "x16")) with
   | Error d ->
      ok "operands of different widths are refused"
        (contains ~needle:"wrong element width" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL n8 + n16 must be refused\n");
  (match LiquidIndex.check env ~expected:(Some LiquidIndex.E16) (add (v "x8") (i 1)) with
   | Error d ->
      ok "checking a Nat8 term at Nat16 is refused"
        (contains ~needle:"wrong element width" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL a Nat8 term must not check at Nat16\n");
  (match LiquidIndex.check env ~expected:(Some LiquidIndex.E8) (i 3) with
   | Ok t -> ok "a literal takes the expected width" (t.LiquidIndex.elem = Some LiquidIndex.E8)
   | Error _ ->
      incr failures;
      Printf.printf "  FAIL a literal must take the expected width\n");

  (* ── refuse an unbound index ──────────────────────────────────────────── *)
  (match LiquidIndex.infer env (v "nowhere") with
   | Error d ->
      ok "an unbound index is refused"
        (contains ~needle:"unbound index `nowhere`" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL an unbound index must be refused\n");

  (* ── shadowed names resolve to the right binder ───────────────────────── *)
  (let outer = v "n" in
   let env2 = LiquidIndex.push_scope env in
   let env2, inner_binder =
     LiquidIndex.bind env2 ~name:"n" ~sort:SNat
   in
   match (LiquidIndex.infer env outer, LiquidIndex.infer env2 outer) with
   | Ok a, Ok b ->
      ok "the shadowed name resolves to the inner binder"
        (LiquidIndex.canonical a = "v0"
         && LiquidIndex.canonical b = Printf.sprintf "v%d" inner_binder);
      ok "outer and inner `n` are different terms" (LiquidIndex.canonical a <> LiquidIndex.canonical b);
      (match LiquidIndex.convert env2 a b with
       | LiquidIndex.Different _ -> ok "two binders named n are not convertible" true
       | s ->
          incr failures;
          Printf.printf "  FAIL shadowing must not collapse binders (%s)\n"
            (LiquidIndex.string_of_status s))
   | _ ->
      incr failures;
      Printf.printf "  FAIL shadowed fixtures elaborate\n");

  (* ── canonical identity ignores spans and names ───────────────────────── *)
  (let far = v ~sp:{ LiquidTypes.start = 40; stop = 41 } "n" in
   let near = v ~sp:{ LiquidTypes.start = 0; stop = 1 } "n" in
   match (LiquidIndex.infer env far, LiquidIndex.infer env near) with
   | Ok a, Ok b ->
      ok "spans do not enter canonical identity"
        (LiquidIndex.canonical a = LiquidIndex.canonical b)
   | _ ->
      incr failures;
      Printf.printf "  FAIL span fixtures elaborate\n");
  (let env3, _ = LiquidIndex.bind LiquidIndex.empty ~name:"renamed" ~sort:SNat in
   match (LiquidIndex.infer env (v "n"), LiquidIndex.infer env3 (v "renamed")) with
   | Ok a, Ok b ->
      ok "names do not enter canonical identity"
        (LiquidIndex.canonical a = LiquidIndex.canonical b)
   | _ ->
      incr failures;
      Printf.printf "  FAIL renaming fixtures elaborate\n");

  (* ── a budget-exhausted comparison is unknown, and fails a required check *)
  (let deep = ref (v "n") in
   for _ = 1 to 40 do deep := add (i 1) !deep done;
   match LiquidIndex.infer env !deep with
   | Error d ->
      ok "a deeply nested index hits the elaboration depth budget"
        (contains ~needle:"depth budget" d.LiquidIndex.detail)
   | Ok t ->
      let budget = { LiquidIndex.default_budget with LiquidIndex.max_depth = 2 } in
      (match LiquidIndex.convert ~budget env t t with
       | LiquidIndex.Unknown why ->
          ok "an exhausted comparison is unknown"
            (contains ~needle:"budget exhausted" why)
       | s ->
          incr failures;
          Printf.printf "  FAIL an exhausted comparison must be unknown (%s)\n"
            (LiquidIndex.string_of_status s));
      (match LiquidIndex.require_convertible ~budget env t t with
       | Error d ->
          ok "a required check cannot succeed on unknown"
            (contains ~needle:"cannot succeed" d.LiquidIndex.detail)
       | Ok () ->
          incr failures;
          Printf.printf "  FAIL a required check must not pass on unknown\n"));

  (* ── the fragment boundaries are explicit ─────────────────────────────── *)
  (match LiquidIndex.infer env (TMul (v "n", i 2, LiquidTypes.no_span)) with
   | Error d ->
      ok "multiplication is outside the initial fragment"
        (contains ~needle:"not in the initial index fragment" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL multiplication must be refused\n");
  (match LiquidIndex.infer env (TCall ("mystery", [ v "n" ], LiquidTypes.no_span)) with
   | Error d ->
      ok "an unknown definition is refused"
        (contains ~needle:"not a closed index definition" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL an unknown definition must be refused\n");

  (* A registered closed definition inlines, so it can convert. *)
  (let env4 =
     LiquidIndex.define env ~name:"twice" ~params:1
       (LiquidIndex.add (LiquidIndex.var 0) (LiquidIndex.var 0))
   in
   match (LiquidIndex.infer env4 (TCall ("twice", [ v "n" ], LiquidTypes.no_span)),
          LiquidIndex.infer env4 (add (v "n") (v "n")))
   with
   | Ok a, Ok b -> (
       match LiquidIndex.convert env4 a b with
       | LiquidIndex.Convertible -> ok "a closed definition inlines before comparison" true
       | s ->
          incr failures;
          Printf.printf "  FAIL inlining (%s)\n" (LiquidIndex.string_of_status s))
   | _ ->
      incr failures;
      Printf.printf "  FAIL definition fixtures elaborate\n");

  (* ── natural, modular and machine arithmetic stay distinct ────────────── *)
  (match (LiquidIndex.infer env (TSub (i 3, i 5, LiquidTypes.no_span)),
          LiquidIndex.infer env (i 0))
   with
   | Ok a, Ok b ->
      ok "natural subtraction saturates at zero"
        (LiquidIndex.convert env a b = LiquidIndex.Convertible)
   | _ ->
      incr failures;
      Printf.printf "  FAIL natural subtraction fixtures elaborate\n");
  (match (LiquidIndex.infer env (TSub (v "n", v "m", LiquidTypes.no_span)),
          LiquidIndex.infer env (add (v "n") (v "m")))
   with
   | Ok a, Ok b ->
      ok "natural subtraction is not addition"
        (match LiquidIndex.convert env a b with
         | LiquidIndex.Different _ -> true
         | _ -> false)
   | _ ->
      incr failures;
      Printf.printf "  FAIL subtraction fixtures elaborate\n");
  (let m = LiquidIndex.machine ~elem:LiquidIndex.E8 LiquidIndex.MAdd
             (LiquidIndex.lit 200) (LiquidIndex.lit 100) in
   ok "a machine add that overflows does not fold"
     (LiquidIndex.normalize LiquidIndex.empty m |> LiquidIndex.canonical
      = "(#Nat8 + 200 100)"));

  (* ── the width obligation on machine conversion ───────────────────────── *)
  (let n = Option.get (match LiquidIndex.infer env (add (v "n") (i 1)) with Ok x -> Some x | Error _ -> None) in
   (match LiquidIndex.machine_of ~width:LiquidIndex.E16 n with
    | Error (LiquidIndex.Width_obligation g) ->
       ok "a symbolic machine conversion returns the width obligation"
         (LiquidIndex.goal_text g = "(v0 + 1) = 65535")
    | Error (LiquidIndex.Not_an_index _) ->
       incr failures;
       Printf.printf "  FAIL unexpected machine refusal\n"
    | Ok _ ->
       incr failures;
       Printf.printf "  FAIL a symbolic conversion must not fabricate a machine term\n");
   match LiquidIndex.machine_of ~width:LiquidIndex.E16 (LiquidIndex.lit 7) with
   | Ok t -> ok "a fitting literal discharges its own width bound"
               (t.LiquidIndex.elem = Some LiquidIndex.E16)
   | Error _ ->
      incr failures;
      Printf.printf "  FAIL a fitting literal must convert\n");

  (* ── families: explicit arity and kinds ───────────────────────────────── *)
  (let ty_arg x = LiquidTypes.TyArgTy x in
   let elem8 = LiquidTypes.TySort (LiquidTypes.SAdt "Nat8") in
   (match LiquidIndex.family_of_ty
            (LiquidTypes.TyApp ("Span", [ ty_arg elem8; LiquidTypes.TyArgIndex (i 0);
                                          LiquidTypes.TyArgIndex (v "n") ], LiquidTypes.no_span))
    with
    | Ok (f, _) -> ok "Span[Nat8, 0, n] fits its family" (f.LiquidIndex.name = "Span")
    | Error d ->
       incr failures;
       Printf.printf "  FAIL Span[Nat8, 0, n] must elaborate (%s)\n"
         (LiquidIndex.string_of_diag d));
   (match LiquidIndex.family_of_ty
            (LiquidTypes.TyApp ("Span", [ ty_arg elem8; LiquidTypes.TyArgIndex (v "n") ],
                                LiquidTypes.no_span))
    with
    | Error d ->
       ok "Span with the wrong arity is refused"
         (contains ~needle:"takes 3 argument(s)" d.LiquidIndex.detail)
    | Ok _ ->
       incr failures;
       Printf.printf "  FAIL Span[Nat8, n] must be refused\n");
   (match LiquidIndex.family_of_ty
            (LiquidTypes.TyApp ("Span", [ ty_arg elem8; ty_arg elem8;
                                          LiquidTypes.TyArgIndex (v "n") ], LiquidTypes.no_span))
    with
    | Error d ->
       ok "a type where a nat index belongs is refused"
         (contains ~needle:"expects a nat index" d.LiquidIndex.detail)
    | Ok _ ->
       incr failures;
       Printf.printf "  FAIL a type in an index position must be refused\n");
   match LiquidIndex.family_of_ty
           (LiquidTypes.TyApp ("Mystery", [], LiquidTypes.no_span)) with
   | Error d ->
      ok "an unknown family is refused"
        (contains ~needle:"is not an index family" d.LiquidIndex.detail)
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL an unknown family must be refused\n");

  (* ── the source width is not the target sort ──────────────────────────── *)
  ok "every width lowers to the same target sort"
    (LiquidIndex.why3_sort (Some LiquidIndex.E8) = "int"
     && LiquidIndex.why3_sort (Some LiquidIndex.E64) = "int");

  (* ── diagnostics are separate from identity ───────────────────────────── *)
  (match LiquidIndex.infer env (v "nowhere") with
   | Error d ->
      ok "a refusal carries a rule, a detail and a span"
        (d.LiquidIndex.rule <> "" && d.LiquidIndex.detail <> ""
         && LiquidIndex.string_of_diag d <> "")
   | Ok _ ->
      incr failures;
      Printf.printf "  FAIL the unbound fixture must be refused\n");

  if !failures = 0 then begin
    print_endline "OK";
    exit 0
  end
  else begin
    Printf.printf "%d failure(s)\n" !failures;
    exit 1
  end
