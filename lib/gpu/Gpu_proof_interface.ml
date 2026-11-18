(*
   PC04: source → KernelIR reflection for the clamp fragment.

   Input is the *checked monomorphic typed AST* the subset pass consumes
   (`Stages.Mtast`) and the KernelIR that pass produced — not a retyped model
   of the sample. The module reflects one shape:

       function f(x: Nat32): Nat32 is
           if x > 100 then return 100; else return x; end if;
       end;

   from the typed AST (`MIf`/`MComparison`/`MIntConstant`/`MParamVar`) and
   from the actual IR (`If (Compare (GreaterThan, Param 0, U32 "100"), …)`).
   `witness` requires the two reflections to agree. The reflection itself —
   AST constructors and IR nodes to MM0 names — is a **trusted, recorded edge**
   (`docs/proof/SOURCE_IR.md`); the bound below it is proved in
   `mm0/proof_chain/clamp_source.mm0`. Nothing here claims the parser or
   decoder is proved.

   If the actual source condition is reversed the reflected operator becomes
   `ujoin` and the emitted bound theorem no longer checks; if the IR carries a
   different parameter or literal the two reflections disagree and `witness`
   refuses. Digests are recorded on the artifact and never make a mismatched
   correspondence pass: `check_artifact` compares the recorded reflection lines
   and the emitted definition against a fresh reflection.
*)

module Mt = Stages.Mtast

exception Refused of string

let refuse fmt = Printf.ksprintf (fun s -> raise (Refused s)) fmt

type reflection = {
  entry: string;
  param_name: string;
  param_index: int;
  comparison: Common.comparison_operator;
  literal: string;
  branch: string;
  operator: string;
}

let comparison_name: Common.comparison_operator -> string = function
  | Common.Equal -> "Equal"
  | Common.NotEqual -> "NotEqual"
  | Common.LessThan -> "LessThan"
  | Common.LessThanOrEqual -> "LessThanOrEqual"
  | Common.GreaterThan -> "GreaterThan"
  | Common.GreaterThanOrEqual -> "GreaterThanOrEqual"

(* `x > 100` returns 100 for the high branch; over PC02's total order that is
   the meet. `x < 100` is the join. Any other comparison is outside the
   reflected fragment and is refused rather than approximated. *)
let operator_of: Common.comparison_operator -> string = function
  | Common.GreaterThan -> "umeet"
  | Common.LessThan -> "ujoin"
  | op ->
     refuse "comparison %s is not the reflected clamp shape (> or < only)"
       (comparison_name op)

let flip: Common.comparison_operator -> Common.comparison_operator = function
  | Common.LessThan -> Common.GreaterThan
  | Common.LessThanOrEqual -> Common.GreaterThanOrEqual
  | Common.GreaterThan -> Common.LessThan
  | Common.GreaterThanOrEqual -> Common.LessThanOrEqual
  | (Common.Equal | Common.NotEqual) as op -> op

let describe (r: reflection): string =
  Printf.sprintf
    "entry=%s param=%s:%d comparison=%s literal=%s branch=%s operator=%s"
    r.entry r.param_name r.param_index (comparison_name r.comparison)
    r.literal r.branch r.operator

let rec show_expr: Mt.mexpr -> string = function
  | Mt.MParamVar (id, _) ->
     Printf.sprintf "MParamVar(%s)" (Identifier.ident_string id)
  | Mt.MLocalVar (id, _) ->
     Printf.sprintf "MLocalVar(%s)" (Identifier.ident_string id)
  | Mt.MTemporary (id, _) ->
     Printf.sprintf "MTemporary(%s)" (Identifier.ident_string id)
  | Mt.MIntConstant s -> Printf.sprintf "MIntConstant(%s)" s
  | Mt.MComparison (op, a, b) ->
     Printf.sprintf "MComparison(%s, %s, %s)" (comparison_name op)
       (show_expr a) (show_expr b)
  | Mt.MCast (e, _) -> Printf.sprintf "MCast(%s)" (show_expr e)
  | Mt.MTypecast (e, _) -> Printf.sprintf "MTypecast(%s)" (show_expr e)
  | Mt.MConjunction (a, b) ->
     Printf.sprintf "MConjunction(%s, %s)" (show_expr a) (show_expr b)
  | Mt.MDisjunction (a, b) ->
     Printf.sprintf "MDisjunction(%s, %s)" (show_expr a) (show_expr b)
  | Mt.MNegation a -> Printf.sprintf "MNegation(%s)" (show_expr a)
  | _ -> "<unsupported expression>"

let rec first_if: Mt.mstmt -> (Mt.mexpr * Mt.mstmt * Mt.mstmt) option = function
  | Mt.MIf (c, t, f) -> Some (c, t, f)
  | Mt.MBlock (a, b) ->
     (match first_if a with Some x -> Some x | None -> first_if b)
  | _ -> None

(* The checked AST hoists subexpressions into compiler temporaries, so the
   condition is usually `MTemporary(_t…)` whose defining expression is a
   `MLetTmp`/`MAssignTmp` earlier in the body. Reflection follows those
   bindings; it never resolves a parameter or local to anything but itself. *)
let collect_temps (body: Mt.mstmt): (string * Mt.mexpr) list =
  let rec go acc = function
    | Mt.MLetTmp (id, _, e) | Mt.MAssignTmp (id, e) ->
       (Identifier.ident_string id, e) :: acc
    | Mt.MBlock (a, b) -> go (go acc a) b
    | Mt.MIf (_, t, f) -> go (go acc t) f
    | Mt.MReturn _ -> acc
    | _ -> acc
  in
  go [] body

let resolve (temps: (string * Mt.mexpr) list) (e: Mt.mexpr): Mt.mexpr =
  let rec go fuel e =
    if fuel = 0 then e
    else
      match e with
      | Mt.MTemporary (id, _) ->
         (match List.assoc_opt (Identifier.ident_string id) temps with
          | Some binding -> go (fuel - 1) binding
          | None -> e)
      | _ -> e
  in
  go 64 e

let typed_entry (entry: string) (mono: Mt.mono_module) =
  let Mt.MonoModule (_, decls) = mono in
  match
    List.find_map
      (function
        | Mt.MFunction (_, name, params, _, body)
          when Identifier.ident_string name = entry ->
           Some (params, body)
        | _ -> None)
      decls
  with
  | Some pair -> pair
  | None -> refuse "entry %s is not a function of the typed module" entry

let of_typed ~(entry: string) (mono: Mt.mono_module): reflection =
  let (params, body) = typed_entry entry mono in
  let names =
    List.map
      (fun (Mt.MValueParameter (pid, _)) -> Identifier.ident_string pid)
      params
  in
  if List.length names <> 1 then
    refuse "entry %s takes %d parameter(s); the reflected fragment has one"
      entry (List.length names);
  let param_name = List.hd names in
  let temps = collect_temps body in
  let (cond, then_branch, else_branch) =
    match first_if body with
    | Some triple -> triple
    | None -> refuse "entry %s body is not an if statement" entry
  in
  let cond =
    match resolve temps cond with
    | Mt.MComparison (op, a, b) ->
       Mt.MComparison (op, resolve temps a, resolve temps b)
    | other -> other
  in
  let (comparison, literal) =
    match cond with
    | Mt.MComparison (op, Mt.MParamVar (pid, _), Mt.MIntConstant lit)
      when Identifier.ident_string pid = param_name ->
       (op, lit)
    | Mt.MComparison (op, Mt.MIntConstant lit, Mt.MParamVar (pid, _))
      when Identifier.ident_string pid = param_name ->
       (flip op, lit)
    | other ->
       refuse
         "entry %s condition is not `<param> <cmp> <literal>` for parameter %s: %s"
         entry param_name (show_expr other)
  in
  let resolved_branch =
    match then_branch with
    | Mt.MReturn e -> Mt.MReturn (resolve temps e)
    | s -> s
  in
  let branch =
    match resolved_branch with
    | Mt.MReturn (Mt.MIntConstant v) -> v
    | Mt.MReturn other ->
       refuse "entry %s true branch returns %s, not a literal" entry
         (show_expr other)
    | _ -> refuse "entry %s true branch is not a return" entry
  in
  (match else_branch with
   | Mt.MReturn e ->
      (match resolve temps e with
       | Mt.MParamVar (pid, _)
         when Identifier.ident_string pid = param_name -> ()
       | other ->
          refuse "entry %s false branch returns %s, not parameter %s"
            entry (show_expr other) param_name)
   | _ ->
      refuse "entry %s false branch is not `return %s`" entry param_name);
  { entry; param_name; param_index = 0; comparison;
    literal; branch; operator = operator_of comparison }

(* The IR hoists the comparison into a `let` consumed by the `if`, exactly as
   the typed AST hoists it into a temporary. *)
let rec first_ir_if: Gpu_ir.stmt -> (Gpu_ir.expr * Gpu_ir.stmt * Gpu_ir.stmt) option =
  function
  | Gpu_ir.If (c, t, f) -> Some (c, t, f)
  | Gpu_ir.Seq items ->
     List.fold_left
       (fun found s ->
         match found with Some _ -> found | None -> first_ir_if s)
       None items
  | _ -> None

let rec collect_lets (body: Gpu_ir.stmt): (int * Gpu_ir.expr) list =
  match body with
  | Gpu_ir.Seq items -> List.concat_map collect_lets items
  | Gpu_ir.Let (n, e) -> [(n, e)]
  | Gpu_ir.If (_, t, f) -> collect_lets t @ collect_lets f
  | _ -> []

let resolve_ir (lets: (int * Gpu_ir.expr) list) (e: Gpu_ir.expr): Gpu_ir.expr =
  let rec go fuel e =
    if fuel = 0 then e
    else
      match e with
      | Gpu_ir.Local n ->
         (match List.assoc_opt n lets with
          | Some binding -> go (fuel - 1) binding
          | None -> e)
      | _ -> e
  in
  go 64 e

let of_ir ~(entry: string) (kernel: Gpu_ir.kernel): reflection =
  if kernel.Gpu_ir.entry_name <> entry then
    refuse "kernel entry is %s, not %s" kernel.Gpu_ir.entry_name entry;
  let lets = collect_lets kernel.Gpu_ir.body in
  let (cond, then_branch, else_branch) =
    match first_ir_if kernel.Gpu_ir.body with
    | Some triple -> triple
    | None ->
       refuse "kernel body has no if; the reflected fragment needs one"
  in
  let cond =
    match resolve_ir lets cond with
    | Gpu_ir.Compare (cmp, a, b) ->
       Gpu_ir.Compare (cmp, resolve_ir lets a, resolve_ir lets b)
    | other -> other
  in
  let (op, index, literal, branch, else_index) =
    match (cond, then_branch, else_branch) with
    | ( Gpu_ir.Compare (cmp, Gpu_ir.Param i, Gpu_ir.U32 lit),
        Gpu_ir.Return (Gpu_ir.U32 v),
        Gpu_ir.Return (Gpu_ir.Param j) ) ->
       (cmp, i, lit, v, j)
    | _ ->
       refuse
         "kernel body is not `if <param> <cmp> <literal> then return <literal> \
          else return <param>`"
  in
  if index <> else_index then
    refuse "kernel condition reads parameter %d but the else branch reads %d"
      index else_index;
  let comparison =
    match op with
    | Gpu_ir.GreaterThan -> Common.GreaterThan
    | Gpu_ir.LessThan -> Common.LessThan
    | other ->
       refuse "kernel comparison %s is not the reflected clamp shape"
         (match other with
          | Gpu_ir.Eq -> "eq" | Gpu_ir.NotEqual -> "ne"
          | Gpu_ir.LessThanOrEqual -> "le"
          | Gpu_ir.GreaterThanOrEqual -> "ge"
          | Gpu_ir.LessThan -> "lt" | Gpu_ir.GreaterThan -> "gt")
  in
  { entry; param_name = "?"; param_index = index; comparison;
    literal; branch; operator = operator_of comparison }

let witness (source: reflection) (ir: reflection): reflection =
  if source.comparison <> ir.comparison then
    refuse "source comparison %s does not match the IR comparison %s"
      (comparison_name source.comparison) (comparison_name ir.comparison);
  if source.literal <> ir.literal then
    refuse "source literal %s does not match the IR literal %s"
      source.literal ir.literal;
  if source.branch <> ir.branch then
    refuse "source branch value %s does not match the IR branch value %s"
      source.branch ir.branch;
  if source.param_index <> ir.param_index then
    refuse "source parameter %s:%d maps to IR parameter %d"
      source.param_name source.param_index ir.param_index;
  if source.literal <> "100" then
    refuse
      "literal %s is not the fixture literal 100 bound to PC02's `u100`; extend the shared word theory before reflecting another literal"
      source.literal;
  { source with param_index = ir.param_index }

let render ?(source_sha256 = "") ?(ir_sha256 = "") (r: reflection)
  : string * string =
  let header = [
    "-- clamp_source.mm0 — PC04 source→KernelIR reflection (generated).";
    "--";
    Printf.sprintf "-- reflect: %s" (describe r);
    Printf.sprintf "-- source_sha256=%s" source_sha256;
    Printf.sprintf "-- ir_sha256=%s" ir_sha256;
    Printf.sprintf
      "-- typed AST: MIf (MComparison (%s, MParamVar (%s), MIntConstant %s),"
      (comparison_name r.comparison) r.param_name r.literal;
    Printf.sprintf
      "--            MReturn (MIntConstant %s), MReturn (MParamVar %s))"
      r.branch r.param_name;
    Printf.sprintf
      "-- KernelIR:  If (Compare (%s, Param %d, U32 %s), Return (U32 %s), Return (Param %d))"
      (comparison_name r.comparison) r.param_index r.literal r.branch
      r.param_index;
    "--";
    "-- trusted-edge: the typed-AST and KernelIR nodes above name the two";
    "--   semantic objects; mapping them to `source_clamp`/`ir_clamp` is";
    "--   recorded here, not proved. The bound below is proved from PC02's";
    "--   order laws. A reversed source condition changes `operator` and the";
    "--   bound proof no longer checks.";
    "--";
    "-- This fragment is assembled after `mm0/proof_chain/word.mm0`.";
  ] in
  let decls = [
    Printf.sprintf "def ir_clamp (x: u32): u32 = $ %s x u100 $;" r.operator;
    "theorem ir_clamp_matches_source (x: u32):";
    "  $ ueq (ir_clamp x) (source_clamp x) $;";
    "theorem ir_inherits_source_bound (x: u32):";
    "  $ ule (ir_clamp x) u100 $;";
  ] in
  let proof = [
    "-- clamp_source.mm1 — proofs for clamp_source.mm0 (PC04).";
    "--";
    "-- Both definitions unfold to the reflected operator, so the value";
    "-- witness is reflexivity and the bound is PC02's meet lower bound. Under";
    "-- a reversed source condition (`ujoin`) neither proof unifies: the bound";
    "-- is tied to the actual comparison in the fixture, not to a digest.";
    Printf.sprintf "def ir_clamp (x: u32): u32 = $ %s x u100 $;" r.operator;
    "pub theorem ir_clamp_matches_source (x: u32):";
    "  $ ueq (ir_clamp x) (source_clamp x) $";
    "  = '(ueq_refl);";
    "pub theorem ir_inherits_source_bound (x: u32):";
    "  $ ule (ir_clamp x) u100 $";
    "  = '(umeet_lb_r);";
  ] in
  (String.concat "\n" (header @ decls) ^ "\n",
   String.concat "\n" proof ^ "\n")

let check_artifact (r: reflection) (text: string): unit =
  let lines = String.split_on_char '\n' text in
  let recorded =
    List.filter_map
      (fun line ->
        let prefix = "-- reflect: " in
        let n = String.length prefix in
        if String.length line >= n && String.sub line 0 n = prefix then
          Some (String.sub line n (String.length line - n))
        else None)
      lines
  in
  (match recorded with
   | [line] when line = describe r -> ()
   | [] -> refuse "artifact has no `-- reflect:` line"
   | [line] ->
      refuse "artifact records `%s`, re-reflection gives `%s`"
        line (describe r)
   | _ -> refuse "artifact has more than one `-- reflect:` line");
  let definition =
    List.find_opt
      (fun line ->
        let prefix = "def ir_clamp" in
        String.length line >= String.length prefix
        && String.sub line 0 (String.length prefix) = prefix)
      lines
  in
  match definition with
  | Some line ->
     let expected = Printf.sprintf "$ %s x u100 $" r.operator in
     let contains haystack needle =
       let n = String.length needle and m = String.length haystack in
       let rec go i =
         i + n <= m
         && (String.sub haystack i n = needle || go (i + 1))
       in
       go 0
     in
     if not (contains line expected) then
       refuse
         "artifact defines ir_clamp as a different operator than the source \
          reflection (%s)" r.operator
  | None -> refuse "artifact does not define ir_clamp"
