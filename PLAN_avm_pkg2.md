  Empirical Review of the Lean 4 → MM0 Translator (on ConLeche.Model)

  Using the ConLeche.Model correctness-proof export (../con-leche/_out/capstone.ndjson, 484 MiB, 8.95M expressions, 3,975 declarations; current sweep at 3,871/3,975 verified, 0 logic rejections, 104 resource
  failures) as the diagnostic benchmark, profiling and artifact inspection exposed six concrete root causes:

  1. The Primary Wall (all 104 remaining ConLeche failures): mm0-rs elaboration memory (20–36× file size)
  • Every single non-passing ConLeche row fails on mm0-rs memory/time or emit time—none is rejected for soundness.
  • Measured on ConLeche.Model.EnvModelM.renameCore (full.mm0 243 MB, full.mm1 251 MB):
      • mm0-rs compile: 373 s, 17.1 GB peak RSS, producing a 30.0 MB .mmb (_build/conleche/as-check.mmb).
      • mm0-c (_build/mm0-c-store2x/mm0-c-store2x) verifying that exact .mmb against full.mm0: 2.74 s, 313 MB peak RSS (store 41.1/134.2 MB, heap 87.6K/131K, stack 850/65.5K).
  • On larger rows (admVal_kid, genArgs_graded, blockTablesStage_of, swapP, swapTyP), mm0-rs OOM-kills past the host's 21 GiB ceiling because it re-expands def sdK at every use during elaboration.
  • Implication: Direct .mmb synthesis from the derivation (still checked end-to-end by the frozen mm0-c C verifier against full.mm0, adding zero to the TCB) completely bypasses mm0-rs's 36× RAM wall.
  2. Massive Redundancy in Generated .mm0 Files (causing 1.05 GB .mm0 files that also exceed mm0-c's 512 MiB INPUT_SIZE)

  Measured on existing failing ConLeche artifacts:

   Artifact                         │                       Total .mm0 │                             def sdK │                       axiom ax_* │       Repeated ctx inside axioms │                             thm
  ──────────────────────────────────┼──────────────────────────────────┼─────────────────────────────────────┼──────────────────────────────────┼──────────────────────────────────┼─────────────────────────────────
   EnvModelM.renameCore             │                         243.5 MB │   92.0 MB (8,600 defs, max 14.5 MB) │         119.9 MB (39,763 axioms) │                          77.0 MB │                         27.2 MB
   admVal_kid                       │                         356.6 MB │              158.0 MB (14,071 defs) │         147.5 MB (44,543 axioms) │                         105.2 MB │                         44.8 MB
   blockTablesStage_of              │                       1,048.5 MB │ 655.8 MB (27,344 defs, max 19.9 MB) │         356.1 MB (78,858 axioms) │                         313.3 MB │                         30.2 MB

  Three specific emitter bugs/omissions drive this bloat:

  • Undeduplicated axioms in scripts/lean-typing-proof:3543-3750: const_ax, beta_ax, and field_ax are deduplicated, but deq_ax (ax_lev/ax_deq), ax_type, and ax_leaf are never deduplicated. Measured on renameCore:
      • ax_lev: 13,919 emitted → 27 unique (99.8% duplicates!)
      • ax_deq: 9,625 emitted → 1,034 unique (89.3% duplicates!)
      • ax_type: 1,581 emitted → 413 unique (73.9% duplicates, 19.8 MB wasted!)
      • ax_leaf: 103 emitted → 36 unique (65.0% duplicates!)
      • Total axioms on renameCore: 39,763 → 16,045 (−59.6%).
  • Unshared context spines in Deriver.ctx_str: renameCore's 5,733 ax_const axioms reference only 23 distinct constants across 694 distinct contexts, repeating (tcons ... tnil) for 64.75 MB (and 313.3 MB on
  blockTablesStage_of). Sharing contexts as incremental nullary defs (def cxK: Term = $ (tcons <dom> cxParent) $;) collapses 88% of axiom bytes to <1 MB.
  • Unsplit def bodies: LTP_DEF_BODY_MAX defaults to 1 GiB (scripts/lean-typing-proof:511), so 14–20 MB def bodies are never split (655 MB of defs on blockTablesStage_of).
  3. Emitter CPU Hotspots (Measured with cProfile on ConLeche.Model.declQuotPB_at, 74.8 s total)
  1. Export.__init__ re-indexing every subprocess: 26.44 s (35.3%) — scripts/lean-proof-scan:849-867 spawns a fresh lean-typing-proof process per declaration, re-hashing 507 MB with SHA-256 (0.80 s, unused by
  lean-typing-proof) and re-scanning 9.36M lines (calling fh.tell() 9.36M times = 3.12 s). Plus Deriver.ind_type / ind_decl re-scan all 507 MB again (1.73 s) even though Export.__init__ already parsed every
  "inductive" record.
  2. Writing unused _obligations.jsonl: 23.33 s (31.2%) + 200 MB–3.6 GB disk I/O — lean-proof-scan never disables obligation output even though it immediately unlink()s *_obligations.jsonl on every pass.
  3. Tree-walking Enc._ds_bytes: 11.45 s (15.3%) — _ds_bytes walks the expanded tree with _ds_walk=True without consulting self._ds_size on children, doing redundant exponential work on DAGs instead of O(|V|)
  bottom-up DP.
  4. Tree-walking Enc.digest: 4.65 s (6.2%) — digest streams the full expanded string into SHA-256 on every miss instead of computing a cached O(|V|) structural DAG Merkle hash.
  5. Quadratic proof-string concatenation in Deriver.infer / _chk (f"(ty_app {fproof} ...)") and O(N) list scans in nat_name.
  4. Correctness Bug in De Bruijn Substitution/Lifting for Projections
  In Deriver._lift, Deriver._subst0, and Deriver.max_free, "proj" is treated as a closed leaf and returned unchanged instead of recursing into e["proj"]["struct"]. Any dependent structure projection referencing a
  bvar gets silently corrupted under lift/subst0.
  5. Progressive Generate → Verify → Discard & Disk-Space Defects in scripts/lean-proof-scan
  • 4× disk + 2× Python RAM during assembly (lines 890–893): reads entire *_typing.mm0/.mm1 into RAM via .read_text() to prepend prefix_text, keeping both *_typing.* and full.* simultaneously, then reads full.mm0
  and full.mm1 into RAM again at lines 947–949 for ledger_append.
  • Leaked full.mm0 on pass & multi-GB debris on fail: discard_proofs() leaves work/<decl>/full.mm0 on disk even though the stripped statement is already saved in memo/stmt/<sha12>/, and failures leave all 5 files
  (*_typing.mm0/.mm1, full.mm0/.mm1, multi-GB *_obligations.jsonl) behind (~17 GB currently in _build/conleche/model*).
  • Non-progressive failure logging & concurrent write race: --record-failures only flushes at process exit (lines 962–968), so an interrupted sweep loses failure rows; index.tsv appends are unlocked (fcntl.flock
  missing).
  • Minor driver bugs: directory name collision on short (line 844) and "sorry" in (c.stdout + c.stderr) false-positive if a declaration name contains "sorry" (lines 501, 908).
  ──────
  Implementation Plan (Designed for Step-by-Step Execution by a Smaller/Cheaper LLM)
  Global Invariants (DO NOT BREAK)
  1. Do not modify docs/cic.mm0 or docs/cic_typing.mm0. Keeping theory_sha unchanged preserves all 3,871 verified rows in _build/conleche/model.memo/index.tsv and adds zero axioms to the TCB.
  2. Keep the goal band (theorem thm_X: $ tty ... $;\n) byte-identical so statement_sha256 matches the existing ConLeche ledger across every optimization.
  3. Keep scripts/check-lean-typing-proof green (0 moved artifacts in scripts/lean-typing-proof.golden.tsv). Every format-changing optimization (--no-obligations, --compact, --emit-mmb) must be opt-in on lean-
  typing-proof and enabled by lean-proof-scan.
  4. Always run mm0-c as the verification oracle (SCAN_MM0_C=/home/leo/.local/bin/mm0-c). Never accept a proof without mm0-c exit code 0 and zero sorry.
  ──────
  Phase 1: Byte-Identical Emitter Speedups, proj Bugfix & Progressive Disk Hygiene
  Goal: Cut per-declaration emit time by ~3–5×, fix the proj de Bruijn bug, and bound lean-proof-scan disk usage to O(1 declaration), with zero changes to default golden output.

  1. scripts/lean2mm0:380-451 (Export.__init__):
      • Add parameter compute_sha: bool = True; only run file_sha256(path) when compute_sha is True (set self.sha256 = "" otherwise).
      • Replace off = fh.tell() inside the readline() loop with an integer accumulator off = 0; ...; off += len(raw) (exact in binary mode; removes 9.36M calls).
      • While processing self._pending_inductives in _register_inductive (or right after the loop), populate two dicts on Export:
	  • self.ind_decls: dict[str, dict] mapping each inductive type's qualified name self.name(t["name"]) to ind.
	  • self.ind_types: dict[str, int] mapping each inductive type, ctor, and rec qualified name to its "type" eid.
  2. scripts/lean-typing-proof:
      • Call l2m.Export(export_path, lazy=True, compute_sha=False) in main().
      • In Deriver.ind_decl (line 3008) and Deriver.ind_type (line 3033): read directly from self.ex.ind_decls / self.ex.ind_types if present (keep file-scan fallback only if attribute absent).
      • Add NATS_SET: set[str] = set() beside NATS in nat_name for O(1) dedup.
      • Replace Enc._ds_bytes(eid) with an iterative/memoized O(|V|) DAG byte-length DP over self._ds_size:
	  • Exact formulas matching _enc_tokens:
              • bvar k: len(f"(tvar {nat_name('idx' + str(k))})") if not succ_idx else 12 + 7 * k ("(tvar tzero)" is 12, each (tsucc ...) adds 7).
              • sort: len(self.sort(e)) (still registers level in self.levels!).
              • const: len(self.sym(self.ex.name(e["const"]["name"]))) (still registers sym in SYMS!).
              • natVal/strVal/sym: len(self.sym(...)).
              • mdata: size(expr).
              • app: 8 + size(fn) + size(arg).
              • forallE: 7 + size(type) + size(body).
              • lam: 7 + size(body).
              • letE: 15 + size(body) + size(value).
              • proj: 14 + 7 * idx + size(struct) ("(tproj " 7 + " " 1 + tzero 5 + ")" 1 = 14, plus 7 * idx).
              • tsubst: 10 + size(b) + size(x).
	  • Pitfall: Make sure symbol/level registration order stays identical to _enc_tokens! Before committing, verify check-lean-typing-proof is 100% byte-identical. If symbol registration order differs on any
	  shared node (because _ds_bytes used to walk children in _enc_tokens order even on repeated subtrees—which doesn't re-register already-registered symbols—post-order/pre-order DFS visiting children in the
          exact same order as _enc_tokens preserves registration order identically!).
      • Replace Enc.digest(eid) with a cached O(|V|) structural DAG Merkle hash (using hashlib.blake2b(digest_size=16) or sha256 per node, memoized in self._digest with no tree expansion):
	  • Strip mdata transparently.
	  • Hash letE(body, value) using the exact same tag and structure as app(lam(body), value) (H("app", H("lam", digest(body)), digest(value))), because _enc_tokens encodes both to identical text (tapp (tabs
	  body) value).
	  • Hash const, natVal, strVal, sym using their canonical symbol key (same string passed to self.sym(...)), and sort using str(e["sort"]) (so two nodes have equal Merkle digest iff their expanded MM0
	  encodings are byte-identical).
      • Fix proj handling in Deriver._lift, Deriver._subst0, and Deriver.max_free:
          • Remove "proj" from the leaf shortcut in _lift and _subst0; instead copy d = dict(e["proj"]), rewrite d["struct"] = self.lift/subst0(d["struct"], ...), and return self.mk({"proj": d}) (or eid if struct
	  unchanged).
	  • In max_free, handle "proj" by returning self.max_free(e["proj"]["struct"], depth).
      • Add --no-obligations CLI flag: when passed, pass a no-op ObligationSink (creates no file, begin/write/end_line/close do nothing, and emit_obligation returns immediately).
  3. scripts/lean-proof-scan:
      • Pass --no-obligations to EMITTER unless --keep-proofs is set.
      • Replace full0.write_text(prefix_text + stem.with_suffix(".mm0").read_text()) (and full1) with a streaming helper prepend_file(prefix_text, src, dst) using shutil.copyfileobj (1 MiB buffer), then
      immediately src.unlink(missing_ok=True) unless --keep-proofs.
      • Stream-hash full1 (1 MiB chunks) and stream-extract statement_digest / statement_of_text from full0 by seeking/scanning markers in chunks instead of .read_text() / .read_bytes().
      • On PASS (when ledger is enabled and not keep_proofs): after ledger_append writes memo/stmt/<sha12>/<name>.mm0, delete out (shutil.rmtree(out, ignore_errors=True)) so work/ stays near 0 bytes!
      • On FAIL (unless --keep-failed is passed): delete out after recording the failure reason so failed sweeps don't accumulate gigabytes.
      • Flush --record-failures rows immediately on each failure (instead of after the loop), and wrap index.tsv appends in fcntl.flock(fh, fcntl.LOCK_EX) for safe concurrent sharding.
      • Fix out dir naming collision (work / f"{slug(short)}_{hashlib.sha1(nm.encode()).hexdigest()[:8]}") and fix sorry detection to re.search(r"\b(contains|uses) sorry\b", c.stdout + c.stderr).

  Verification for Phase 1:
    python3 scripts/check-lean-typing-proof
    # Must print: OK — 12 artifact(s) across 4 modes match lean-typing-proof.golden.tsv

    # Test on 3 ConLeche decls (small, medium, declQuotPB_at) into a scratch workdir & ledger:
    printf '%s\n' \
      ConLeche.Model.no_proof_of_Empty_pure \
      ConLeche.Model.CtxOkP.extend \
      ConLeche.Model.declQuotPB_at > /tmp/p1_only.txt
    SCAN_MM0_C=/home/leo/.local/bin/mm0-c python3 scripts/lean-proof-scan \
      ../con-leche/_out/capstone.ndjson ConLeche.Model /tmp/p1_work \
      --only=/tmp/p1_only.txt --memo=/tmp/p1_memo
    # Expect: 3/3 verified closures, `du -sh /tmp/p1_work` < 100 KB,
    # and statement_sha256 for all 3 rows matches `_build/conleche/model.memo/index.tsv`!

  Re-plan if: scripts/check-lean-typing-proof reports any moved byte on Phase 1 (most likely cause: symbol registration order in _ds_bytes or letE Merkle tag mismatch in digest—debug before moving to Phase 2).
  ──────
  Phase 2: Fork-After-Index Execution (Eliminate 8 s Per-Declaration Startup)

  Goal: Index capstone.ndjson once per scan instead of 3,975 times (~9.4 CPU-hours → ~7 seconds).
  1. Extract a clean per-declaration entry point in scripts/lean-typing-proof:
      • Factor the body of main() after ex = l2m.Export(...) into run_one(ex, export_path, want, out_dir, *, spine=True, defer=False, digest=False, no_obligations=False, compact=False, emit_mmb=False) -> int.
      • Reset module-level NATS.clear(); NATS_SET.clear(); SYMS.clear() at the top of run_one().
  2. Use os.fork() per declaration in scripts/lean-proof-scan:
      • lean-proof-scan ALREADY loads ex = l2m.Export(export_path, lazy=True) at startup!
      • Close ex.exprs._fh before forking (if ex.exprs._fh: ex.exprs._fh.close(); ex.exprs._fh = None) so no child shares a file descriptor seek offset!
      • Implement run_emit_forked(ex, ...) (for both digest_of and emit):
	  • Redirect child stdout/stderr to small temp files in out, os.fork().
	  • In child: set resource.setrlimit(RLIMIT_AS, ...), call ltp.run_one(...), flush, os._exit(rc).
	  • In parent: poll os.waitpid(pid, os.WNOHANG) (or use pidfd_open + select.select) up to EMIT_TIMEOUT; on timeout send SIGKILL and reap.
      • Keep a --subprocess-emit fallback flag for debugging.


  Verification for Phase 2:

    # Audit statements across ALL 3,975 ConLeche declarations (used to take 75 min / 13 CPU-hours):
    python3 scripts/lean-proof-scan ../con-leche/_out/capstone.ndjson ConLeche.Model \
      /tmp/p2_audit --memo=_build/conleche/model.memo --audit-statements
    # Expect: finishes in < 2 minutes, `same statement: 3871/3975`, `statement differs: 0`,
    # `digest unavailable: 0`, matching `_build/conleche/audit-final/statements.tsv`!
  Re-plan if: Any child sees corrupted JSON lines (symptom of forgetting to close ex.exprs._fh before fork()).
  ──────
  Phase 3: --compact Mode (Axiom Dedup + Context def cxKs + Split defs + Linear Proof Serialization)

  Goal: Shrink full.mm0 by 5–15× (bringing blockTablesStage_of from 1,048 MB to <120 MB—well under mm0-c's 512 MiB INPUT_SIZE) and eliminate quadratic proof-string concatenation, while keeping the goal band byte-
  identical.
  1. **Add --compact flag to scripts/lean-typing-proof** (enabled by default in lean-proof-scan, off by default in standalone lean-typing-proof so golden gate stays untouched):
      • Axiom deduplication:
	  • deq_ax: add self.deq_ax_map: dict[tuple[bytes, bytes], str] = {} keyed on (enc.digest(got), enc.digest(want)); reuse existing ax_lev_* / ax_deq_* instead of emitting duplicates (cuts 99.8% of ax_lev
	  and 89.3% of ax_deq).
          • ax_type (_chk): add self.type_ax: dict[tuple[str, bytes, bytes], str] keyed on (ctx, enc.digest(eid), enc.digest(expected)).
          • ax_leaf (assume_leaf): add self.leaf_ax_map: dict[tuple[str, bytes, bytes], str] keyed on (ctx, enc.digest(eid), enc.digest(expected)).
      • Context-chain def cxKs:
	  • In Deriver.ctx_str(env) when compact=True: instead of formatting (tcons <dom> ... tnil) inline, intern each context prefix (dom_digest, parent_cx_name) into self.ctx_defs: list[tuple[str, int, str]]
	  ((cx_name, dom_eid, parent_name)), returning "tnil" for empty env and "cx{k}" otherwise!
	  • Write def cx{k}: Term = $ (tcons <enc(dom_eid)> <parent>) $; right after the sdK def block (before -- ── assumptions ──). Because def cxK is emitted in parent-before-child order after all sdKs, there
	  are zero forward references, and enc(dom_eid) reuses sdKs!
      • Split large def sdK bodies:
	  • When compact=True and LTP_DEF_BODY_MAX is unset, default enc.ds_body_max = 65536 (64 KiB) instead of 1 GiB.
      • Linear-time proof tree (no O(N²) string copying):
	  • Represent proofs in Deriver as lightweight tuples (e.g., ("app", fproof, aproof, ax), ("abs", inner), ("deq", inner, ax), ("proj", sproof, ax), ("var", k), or leaf str), and stream them directly to .
	  mm1 via an iterative stack writer (_write_proof(fh, proof))—zero quadratic copies, and the exact AST needed by Phase 4's .mmb emitter!
	  • (Note: keep len/stats tracking lightweight; check-lean-typing-proof verifies .mm1 bytes match verbatim!)

  Verification for Phase 3:
    python3 scripts/check-lean-typing-proof   # must still be 100% green (compact off by default)

    # Measure size reduction on renameCore and blockTablesStage_of:
    python3 scripts/lean-typing-proof ../con-leche/_out/capstone.ndjson \
      ConLeche.Model.EnvModelM.renameCore /tmp/p3_rc --spine --compact --no-obligations
    python3 scripts/lean-typing-proof ../con-leche/_out/capstone.ndjson \
      ConLeche.Model.blockTablesStage_of /tmp/p3_bt --spine --compact --no-obligations
    # Expect: renameCore .mm0 drops 243 MB -> ~35-50 MB; blockTablesStage_of .mm0 drops 1048 MB -> <150 MB!
    # Verify statement digest still matches ledger, and mm0-rs + mm0-c accept small/medium --compact artifacts!
  ──────
  Phase 4: Direct .mmb Synthesis (scripts/mmb_emit.py) Verified by Frozen mm0-c

  Goal: Bypass mm0-rs compile entirely during lean-proof-scan by emitting .mmb directly from the derivation AST + term DAG, so mm0-c verifies even the 25+ previously OOM-killed ConLeche declarations in seconds and
  <1 GiB RAM.

  Create a new, self-contained module scripts/mmb_emit.py (following the exact binary layout in ../mm0-rs/mm0-c/types.c, ../mm0-rs/mm0-c/verifier.c, and ../mm0-rs/mm0-c/parser.c):

  1. Binary Layout & Low-Level Helpers:
      • Command packer pack_cmd(op, data): 1-byte (data==0), 2-byte (<=0xff), 3-byte (<=0xffff), 5-byte (<=0xffffffff) using CMD_DATA_{0,8,16,32}.
      • Header (40 bytes + 3 sort bytes + 5 bytes padding to 8-byte alignment):
	  • Sorts: 0: Prop (SORT_PROVABLE=4), 1: Term (0), 2: nat (0).
      • Fixed theory prefix (docs/cic.mm0 + docs/cic_typing.mm0, verified against mm0-rs's .mmb on no_proof_of_Empty_pure):
	  • Terms 0..28 in exact declaration order (tvar, tapp, tabs, tint, tf64, ttrue, tfalse, tcon, tnil, tcons, tfold, tdup, tera, tprim, tentity, tsubst, teval, teq, timp, valid_iso, tpi, tsort, tsucc, tzero,
	  tty, def tdefeq, tcert, tproj, tfield, def tctx_deq).
	  • Axioms 0..20 in exact declaration order (teq_refl .. ty_ctx), with exact UThm unify streams.
	  • CRITICAL: In mm0-c, CMD_UNIFY_HYP pops hypotheses in reverse order of the .mm0 signature (parser.c:467 prepends to p_hyps). Write a small inspection script on mm0-rs-compiled cic_typing.mm0 first to
	  lock down the exact unify bytes for all 21 theory axioms!
      • Generated declarations (in the exact order written to full.mm0):
	  • term t_*: nat (num_args=0, sort=2).
	  • def sdK: Term = $ ... $ (num_args=0, sort=1|0x80), with both a proof stream (CMD_PROOF_TERM/TERM_SAVE/REF) and a unify stream (CMD_UNIFY_TERM/TERM_SAVE/REF).
	  • def cxK: Term = $ (tcons <dom> <parent>) $ (num_args=0, sort=1|0x80).
	  • axiom ax_*: $ ... $ (num_args=0), with proof stream building the statement expression and matching UThm unify stream.
	  • theorem thm_*: $ tty <goal_ctx> <goal_term> <goal_ty> $ (num_args=0), with proof stream + UThm unify stream.
      • Always append CMD_END (0x00) + 4 bytes zero padding (required by CMD_MAX_SIZE=5 check in verifier.c:704), and emit a minimal "Name" index table (index.c:41) so any mm0-c error names the exact
      sdK/ax_*/thm_*.
  2. Proof-Stream Builder (MmbProofBuilder):
      • Maintain a hash-consed expression table expr_heap: dict[node, int] mapping each (termid, arg_node_ids) to its mm0-c heap slot.
	  • When building a term node: if already in expr_heap, emit CMD_PROOF_REF(slot); otherwise build args, emit CMD_PROOF_TERM_SAVE(termid) (if heap_size < HEAP_LIMIT, e.g., 120_000, else CMD_PROOF_TERM
	  without saving) and record the slot.
	  • Note on HEAP_SIZE: mm0-c resets g_heap and g_store between every def/axiom/theorem. Only save nodes that appear ≥2 times (count uses first!) so g_heap_size stays tiny even on 1M-rule proofs (renameCore
	  had 87K heap entries in mm0-rs; saving only ≥2-use nodes cuts heap usage dramatically!).
      • Convertibility & def Unfolding (prove_conv(e_have, e_want)):
	  • e_have and e_want are internal DAG nodes (which may contain sdK / cxK defs on one side and expanded terms on the other).
	  • Cache proved conversions (e_have, e_want) -> heap_slot using CMD_PROOF_CONV_CUT + CMD_PROOF_CONV_SAVE so each sdK/cxK unfolding is emitted at most once per proof!
	  • Algorithm for e_have =?= e_want:
              1. If e_have == e_want: emit CMD_PROOF_REFL.
              2. If (e_have, e_want) in conv_heap: emit CMD_PROOF_REF(slot).
              3. If (e_want, e_have) in conv_heap: emit CMD_PROOF_SYMM, CMD_PROOF_REF(slot).
              4. If e_have is a def (sdK, cxK, tdefeq, tctx_deq): emit body_node, CMD_PROOF_UNFOLD, recurse on (body_node, e_want).
              5. If e_want is a def: emit CMD_PROOF_SYMM, body_node, CMD_PROOF_UNFOLD, CMD_PROOF_SYMM, recurse on (e_have, body_node).
              6. Same head termid: emit CMD_PROOF_CONG, recurse left-to-right on each pair of arguments (a_i, b_i)!
	  • Wrap each rule application (ty_var0, ty_varS, ty_abs, ty_app, ty_deq, ty_proj, ax_*):
              • Emit premise subproofs (converting any premise whose proven formula differs in sdK/cxK folding via CMD_PROOF_CONV + prove_conv), push the rule's term arguments, push the conclusion expression, emit
              CMD_PROOF_THM(rule_id).
              • At the top level of thm_*, apply one final CMD_PROOF_CONV + prove_conv from the derived tty formula (with sdK/cxK) to the theorem's expanded statement in full.mm0!


  3. Wire into lean-typing-proof --emit-mmb and lean-proof-scan:
      • When lean-proof-scan runs in direct-MMB mode (new default; keep --use-mm0-rs flag as fallback/crosscheck):
	  • Emit full.mm0 + full.mmb directly (no .mm1 written at all!).
	  • Run mm0-c full.mmb < full.mm0.
	  • On exit 0: record in ledger (statement_sha256, derivation_sha256 = sha256 of full.mmb, retained statement in memo/stmt/), then delete full.mm0 and full.mmb.



  Verification Ladder for Phase 4 (strictly in order; do not jump ahead until each step exits 0 in mm0-c):

  1. Step 4a (Theory prefix check): Emit an empty .mmb with just cic.mm0 + cic_typing.mm0 (no generated decls) and verify mm0-c prefix.mmb < prefix.mm0 exits 0!
  2. Step 4b (Capstone smoke test): ConLeche.Model.no_proof_of_Empty_pure (35 rules) → mm0-c full.mmb < full.mm0 exits 0! Also mutate 1 byte of the proof in a test and verify mm0-c exits non-zero!
  3. Step 4c (Medium ConLeche decls): CtxOkP.extend (18,602 rules), declQuotPB_at (10,570 rules), TgtNodePres.kit._proof_1 (65,334 rules) → all exit 0 in mm0-c!
  4. Step 4d (Previously 17.1 GB mm0-rs row): EnvModelM.renameCore → must exit 0 in mm0-c in < 20 s and < 1 GiB RAM!
  5. Step 4e (Previously >21 GiB OOM rows): genArgs_graded, admVal_kid, blockTablesStage_of, EnvModelM.swapP, EnvModelM.swapTyP → check mm0-c exit code and highwater counters (_build/mm0-c-store2x/mm0-c-store2x).
  6. Step 4f (All 104 previously failed ConLeche rows): Run lean-proof-scan with --only over the 104 missing rows from _build/conleche/model.memo, and measure new verified count out of 3,975, total wall clock, 
  peak RSS, and peak disk!

  Re-plan if:

  • On Step 4e (swapP/swapTyP/blockTablesStage_of), mm0-c itself hits HEAP_SIZE (1<<17), STORE_SIZE (1<<27), or INPUT_SIZE (1<<29). Measure the exact highwater counters with _build/mm0-c-store2x/mm0-c-store2x and
  decide whether to: (a) add local intermediate helper theorems (CMD_STMT_LOCAL_THM, which reset mm0-c's store & heap per chunked subproof—keeping mm0-c memory O(chunk) with zero changes to mm0-c!), or (b) add
  sliding-window buffer refill in mm0-c/parser.c. Note that (a) (CMD_STMT_LOCAL_THM) is supported natively by stock mm0-c!
  ──────
  Phase 5: Regression Tests & Documentation

  1. Run python3 scripts/check-lean-typing-proof (confirm 12/12 golden artifacts unchanged).
  2. Add a self-test (python3 scripts/mmb_emit.py --selftest) covering: theory prefix, valid proof passes mm0-c, corrupted proof rejected by mm0-c, def unfolding with sdK/cxK, and proj with free bvar under 
  lift/subst0.
  3. Run bash scripts/verify-invariants (inside nix-shell -p python3) to confirm no repo gate regressed.
  4. Append a short measured section to docs/lean4mm0.md recording the before/after ConLeche numbers (verified closures out of 3,975, wall time, peak RSS, disk usage).
  ──────
