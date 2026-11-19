# Agent Guidelines: australVM (Module Runtime)

australVM is the **module runtime** for the unfer modular probability kernel.
Austral cells (`.aui`/`.aum`) are linear-typechecked, lowered to CPS binary IR,
and JIT-compiled by the safestos cranelift bridge. Modules call the unfer kernel
in-process via native `uk_*` symbols registered in the JIT.

## Layout that matters

- `lib/` — the OCaml Austral compiler. `Compiler.ml` (`use_cps_jit` path),
  `Compiler_cps.ml` (Mtast → CPS IR), `CamlCompiler_rust_bridge.ml` (FFI to the
  Rust JIT). Build with `dune build lib/ bin/ test/`.
- `safestos/cranelift/` — the Rust JIT bridge (`austral_cranelift_bridge`).
  - `src/lib.rs` `cranelift_init()` registers `au_*` runtime symbols and, under
    feature `unfer-kernel`, all 18 `uk_*` + 5 `uz_*` kernel symbols from
    `unfer_ffi` (data-driven `UNFER_SYMBOLS`/`ZENODO_SYMBOLS` tables).
  - `src/auth.rs` — `AuthorizationEngine` trait, `ManifestAuthEngine` (TOML
    grants), `AllowAll`, and `safestos_load_auth_manifest()`. Cedar is an
    **optional** backend (`--features cedar`, on by default).
  - `src/cps.rs` `check_call_permission()` gates every call: `__`/`au_`/self are
    free; everything else (including `uk_*`) goes through `auth::check`.
  - `src/bin/modhost.rs` — loads a `module.toml` manifest and answers the
    kernel-call authorization question (UK-4001 enforcement point).
- `examples/kernel/UnferKernel.aui/.aum` — typed Austral bindings for `uk_*`.
  Byte buffers are `Address[Nat8]` + `Int64` length; handles are `Int64`.

## Build / feature notes

- `cargo build` (default features) → Cedar engine is the default authorizer.
- `cargo build --no-default-features --features unfer-kernel` → `AllowAll`
  default authorizer (used by `demo_module/run_demo.sh` for the live JIT
  execution demo); the manifest gate is then exercised explicitly via `modhost`.
- `crate-type` includes `rlib` so in-crate bins (`modhost`) link the bridge; the
  OCaml side still links the `cdylib`/`staticlib` at
  `safestos/cranelift/target/release/libaustral_cranelift_bridge.{a,so}` — so
  **rebuild that release artifact** after touching the bridge, or the OCaml
  `austral` binary will use a stale lib (and may miss `uk_*` symbols).
  Since A15 this is enforced, not just advised: `make all` and `make test`
  run `check-bridge-fresh`, which refuses to link when any bridge source
  (`src/*.rs`, `Cargo.toml`, `Cargo.lock`) is newer than the `.so` and tells
  you to run `make bridge`. Check it alone with `make check-bridge-fresh`.
- **Shell discipline (measured 2026-10-21).** Run every `dune` command
  inside `nix develop`, never a bare `nix-shell`. The two environments
  carry different glibcs (devshell 2.42, bare nix-shell 2.37 on this
  host), and dune relinks its test executables and ppx driver binaries
  to whichever environment is active — a `nix-shell … dune runtest`
  poisons `_build` with 2.37-linked exes that then cannot satisfy the
  bridge's `GLIBC_2.39`, and the failure surfaces as a loader error
  far from the command that caused it. Symptom: `version GLIBC_2.39 not
  found` (or a dead `ppx.exe` under `_build/default/.ppx/`). Remedy:
  re-run the command under `nix develop` — dune re-executes the link in
  the current environment and `_build` heals (stale ppx drivers are
  regenerable cache: `rm -rf _build/default/.ppx/<hash>` is safe).
  `../scripts/check-elf-interpreters` (workspace gate) catches the dead-
  loader half of this class across all repos in one scan.
  `scripts/toy-witness` compiles `scripts/toy-witness.rs` with `rustc` on every
  run (no cargo, no build directory) and exits 2 when `rustc` is missing, which
  `verify-invariants` reports as a `[gap]`.
- **When `nix develop` itself will not build (measured 2026-11-13, aarch64).**
  This channel's `alt-ergo` fails inside its own build (`ocamlopt: I/O error:
  as --debug-prefix-map …` — no assembler reachable in that sandbox), and since
  `devShells.default` carries the prover, the whole shell is unbuildable and no
  `dune` command can run at all. `nix develop .#ocaml` is the same nixpkgs and the
  same OCaml/rust/python inputs without why3/alt-ergo/z3; it is enough for
  `make all` and `make test` (measured: 161 tests, 18 suites, 0 failures on a
  clean `_build`) and **not** enough for anything that must *prove* —
  `LiquidWhy3.prove` then takes its structured-skip path. Use `.#ocaml` to build,
  `default` to prove. Address the flake as `git+file://$PWD#ocaml`: a plain path
  makes nix copy the whole worktree (14 GB of `refi/target`) into the store
  before it starts. `dune` itself (3.23.1 with OCaml 5.4.1, measured 2026-11-14)
  exists only inside that shell — the host has none — so every `make` target
  that needs it is reached through the shell:

  ```bash
  nix develop "git+file://$PWD#ocaml" -c make all     # or test / bridge
  ```

  `check-dune` (an order-only prerequisite of `$(BIN)` and `bridge`) refuses
  outside the shell and prints that command, because the alternative was a
  `dune: command not found` arriving after `make bridge` had already rebuilt
  cargo.

## Scripts & gates

Everything under `scripts/`; `python3` is not on `PATH` in a bare NixOS
shell, so the `nix-shell -p python3 --run …` wrapper is part of each command:

- `scripts/verify-invariants` — the repo's invariant gate (symbol census vs
  `SYMBOL_REGISTRY`, cross-repo sync, …). Runs in CI
  (`.github/workflows/build-and-test.yml`); a missing interpreter is reported
  as an explained gap, never as a false failure.
- `scripts/check-links` — relative markdown links resolve
  (CI: `.github/workflows/linkcheck.yml`).
- `scripts/au2mm0` — reflects an Austral kernel's obligations to integer
  polynomials and decides them twice (symbolic expansion + exact sampling),
  refusing `div`/`float` as a gap. `emit` writes
  `mm0/<stem>_certificate.{mm0,mm1}` over `docs/kernel_alg.mm0` and re-checks it
  with `mm0-rs` + `mm0-c` when they are installed; a false obligation is refused
  before any file is written.
- `scripts/mm0_ring.py` — the W2 prover behind `emit`: a proof-producing
  normalizer whose steps are ring-law applications from `docs/kernel_alg.mm0`
  and whose coefficient arithmetic is grounded as visible `cf_` facts. Its
  trust split is pinned in `BUDGETS` in `scripts/au2mm0`.
- `mm0/flux1d_certificate.{mm0,mm1}` — the first symbolic certificate (3
  obligations, `-- trust-split: structural=5 grounded=15 certificates=3`).
  Regenerated byte-identically by `verify-invariants`; its statements await
  approval (`mm0-comparator`) and are reported as a `[gap]` until then.
- `scripts/mm0-comparator` — approved MM0 statements, held out (approve /
  check / selftest). A generated statement is compared against
  `mm0/approved/<artifact>.stmts` plus a dependency-closure and theory hash;
  an artifact without an approval is reported as a `[gap]` by
  `verify-invariants`, never a silent pass.
- `scripts/stage-approvals` — stages the pending statement approvals under a
  scratch root (`--root`, default `$TMPDIR/australvm-approve-staging`) and prints
  the diff an owner would commit, without writing `mm0/approved/`: approval is an
  owner commit, so the diff *is* the decision and this only shows it. Takes a
  stem or `mm0/<stem>.mm0`; a name that is neither exits 64 before staging
  anything, because a comparator "refused" line is the message for a real stale
  artifact, not for a typo. Naming an artifact that is *already approved* is
  refused (exit 1) before anything is staged — the diff would be a re-approval
  and reads exactly like a first one — so re-approval is opt-in with
  `--reapprove`; the no-argument run never needs the flag, since it derives its
  artifacts from the approved root. An artifact whose staged copy is
  byte-identical to its approved copy is refused as "nothing to approve" (exit
  1) rather than printing an empty diff; `--approved-root` (default
  `mm0/approved`, read-only) names the copies to diff against. With no argument it stages every artifact in
  `artifacts_all=` that has **no** copy under `--approved-root`, so approval
  state is not listed here a second time: an approval is one edit — the
  artifact's `state` cell in the plan's §W3 table — and this script follows it.
  `verify-invariants` pins the 64, the named-approved refusal, the no-op
  refusal, the artifact list against §W3's rows, the staged set against §W3's
  pending rows, and the skip an approved copy produces, as `[pass] W3` lines.
- `scripts/clif2mm0` — emits/checks `mm0/clif.mm0`, the CLIF *shape* model
  (W9.2): `clif_dump` (a bridge binary) measures the instruction forms the CPS
  lowering really produces, and this generator writes one declaration per form
  with the arity in the argument list. It proves nothing (`-- proves: 0
  theorems`, pinned); a theorem there needs a proof in `mm0/clif.mm1`. The
  corpus it measures is `scripts/cps-corpus`, shared with `scripts/isa-census`
  so the opcode layer and the CLIF layer cannot measure different modules.
  Since 2026-11-14 the file also declares W9.3's step *vocabulary* — `sort St;`
  and `term clif_step (s: St) (o: op) (t: St): Prop;` — and states the
  obligation `clif_step s o t ⇒ isa_step (lower o) s t` with five pinned
  `-- cites:` digests and `-- certificate: [gap]`. It must contain no `axiom`
  and no `theorem`: `verify-invariants` fails if either appears, and also fails
  the `[gap]` line once `mm0/approved/isa.stmts` exists (the owner's approval
  is what turns a stated obligation into a cited one).
- `scripts/check-lean-typing-proof` — the emitter's byte-identity gate: FockPair
  in its three modes (expanded, shared, stopped) compared by size and sha256 with
  `scripts/lean-typing-proof.golden.tsv`, so "the artifacts do not move" is a
  check instead of a hand-run `cmp`. Needs the gitignored corpus under
  `mm0/exports/` and reports an explained gap without it. Re-record an intended
  byte change deliberately, in the same commit: `make check-typing-proof RECORD=1`.
- `scripts/check-todo-ledger` — code files (`*.rs`/`*.sh`/`*.py`/`*.toml`)
  carry no `TODO`/`FIXME`/`HACK`/`XXX` marker unless the file is reviewed in
  `docs/TODO_LEDGER.md`; a ledger entry whose file no longer carries a
  marker is reported `[stale]` so it leaves with the work. Markdown
  historical status prose is out of scan scope by policy.
- `scripts/cps-fingerprint <cps>` — the bridge's own sha256 of the bytes
  (`austral_cps_fingerprint`); `--object <cps>` gives the digest of the
  cranelift-emitted code capture plus `arch=`. Not JIT symbols: no manifest
  grant applies. The object digest is host-native and defined for the first
  compilation in a fresh process, so `verify-invariants` only compares it on a
  matching arch and reports `[gap]` otherwise.
- `scripts/isa-census <module.cps>` — diffs the JIT decoder's own opcode count
  (`crate::isa::note`, `ISA_REGISTRY` in `safestos/cranelift/src/isa.rs`), the
  `-- op 0xNN` table in `mm0/isa.mm0`, and what the module actually read; exits
  1 on any uncovered or overstated opcode. `--corpus` compiles the example
  modules whose CPS path produces a function and censuses the union (one module
  only measures one kernel), and requires the opcodes left unexercised to be
  exactly its `UNREACHABLE_OPCODES` pin. That pin is **empty since 2026-11-14**
  (23 of 23 emitted, 905 reads): `sub`/`mul` now come from
  `Compiler_cps.trapping_arith_op` turning the Pervasive
  `trappingSubtract`/`trappingMultiply` calls into 0x06/0x18 (the JIT keeps the
  trap-on-overflow contract via `emit_trapping_arith`), and `match` is one 0x0A
  instead of a `CmpEq`+`If` chain. An opcode that stops being emitted without a
  reason written in the pin fails the census, so coverage cannot silently
  regress. `scripts/isa-hypotheses <isa.mm0>` refuses a
  `-- gap-hypothesis:` whose `-- cites:` sha256 pin is stale or missing, and
  classifies each one by content: `content-checked` only when every term/sort
  the hypothesis mentions is declared by a cited artifact. `bounds_from_layout`
  is not (no cited artifact has `in_bounds`/`typed`), so it reports `unchecked`
  and its count is pinned as `-- unchecked-citations: N` in `mm0/isa.mm0` — an
  unchecked citation can be declared, but not added or re-labelled silently.
  `ISA_REGISTRY` is the instruction-side twin of `SYMBOL_REGISTRY`.
- `scripts/isa-encdec [--write]` — regenerates the `encdec` table and its 23
  proofs plus the three `step_refines` proofs in `mm0/isa.mm0`, and derives
  `mm0/isa.mm1` (the MM0 proof file) from it. MM0 splits the two:
  `mm0-rs compile mm0/isa.mm1 <mmb>` (output is positional, not `-o`) then
  `mm0-c <mmb> < mm0/isa.mm0`. Editing `isa.mm0` by hand requires `--write`
  afterwards, or the gate reports the proof file stale. A `theorem` in the
  `.mm0` with no entry in the script's proof table is a hard error, never a
  silent `sorry`.
  - MM0 has no lambda in proof terms and proof-position arguments are
    *hypotheses*, not terms: `(req_trans run_add_def (req_symm ir_add_sem))`
    works, `(req_trans (run_add_def s dst a b) ...)` fails with "too many
    arguments" — term arguments are inferred from the goal.
  - A `.mm0` statement's premises (`$ A $ > $ B $`) become named binder
    hypotheses designed by the script (`h1`, `h2`, …) in the `.mm1`, which is
    how a proof stays a plain term instead of needing a lambda.
- The new arithmetic lowering is regression-tested in
  `safestos/cranelift/tests/cps_tests.rs`: `sub_opcode_computes_the_difference`
  and `mul_opcode_computes_the_product` check the value (operand order
  included), and `sub_opcode_traps_on_overflow` / `mul_opcode_traps_on_overflow`
  check the trap. The trap is `std::process::abort`, uncatchable in-process, so
  those two re-exec the test binary as a child (`--exact arith_trap_child_exit`,
  env `AVM_ARITH_TRAP`) and assert SIGABRT plus the `Overflow in
  trappingSubtract/Multiply (Int64)` message. Without `emit_trapping_arith`'s
  `brif` both would return a wrapped i64 and exit 0.
- `lib/Cps_codec.ml` is the OCaml opcode table; `CpsGen` writes every opcode
  through it, and `test/CpsCodecTest.ml` holds it to `mm0/isa.mm0`.
- `mm0/slot_layout.mm0`/`.mm1` — the `St`-level layout model `bounds_from_layout`
  cites: `in_bounds` is *defined* as membership in the slot list a state carries
  (`def in_bounds … = $ mem i (slots s) $`), and `in_bounds_alloc` proves
  allocation never drops a slot. `isa.mm0` repeats the same definition and
  `isa-hypotheses` refuses the two copies if they differ. The one unprovable
  premise left is about the compiler: that the layout pass lists every slot its
  code later dereferences.
- Memory instructions are emitted from source now: `MDeref` lowers to `Deref`
  (0x20) and `MAssign` on a pointer-like target lowers to `Store` (0x30). Before
  that, deref was a call to the `__deref` pseudo-function and a store through a
  reference was an assignment to a never-read `__assign_target` (silently
  dropped). `examples/memory-ops` is the runnable module that keeps both paths
  exercised, and it is in `isa-census --corpus` so they stay measured.
- `examples/kernel/TestKernel.aum` is the smallest module that calls the kernel
  (`uk_version`); `isa-census --corpus` includes it. The corpus compiles with
  `--target-type=c` (no host linker, so `uk_*` need not exist in the `.so`) and
  passes `--allow-all` to both the compiler and the census — the census measures
  bytes in a capture module and never executes them, so the grant question stays
  with `modhost`.
- Lean 4 is not on the host either; it comes from the flake's `lean` shell
  (`nix develop "git+file://$PWD#lean" -c …`): Lean 4.30.0, Lake 5.0.0 and a
  locally built `lean4export` (tag v4.30.0, NDJSON format 3.1.0). Keep lean's
  version and the exporter's tag equal — the pin is `pkgs.lean4.version` from
  this input's nixpkgs, not the registry's newer release. `lean4export <Mod>`
  writes NDJSON to stdout (`LEAN_PATH=<dir> lean4export <Mod> > out.ndjson`);
  giving it a second positional argument panics, because argv after the module
  names is a *constant* list, not an output path.
- `scripts/rv64-model-mm0 [check]` — the W9.1 conversion, end to end, over two
  pinned models:
  - `$RISCV_MODEL` (`packages.riscv-lean`: opencompl/riscv-lean at `ccdfd676`,
    only its import-free `RISCV/Instructions.lean`) → `mm0/statements/
    riscv_instructions.{mm0,mm1,roots}` — 83 `RV64.*` declarations, 3 sorts /
    30 term-def / 94 ax-thm, `mm0-c` exit 0;
  - `$RISCV_MEMORY_MODEL` (`packages.sail-riscv-lean`: the repo's own
    `lake-manifest` lock, `Lean_RV64D` at `11d8fa2` + `lean-sail` at `0794631`,
    built to oleans by the derivation — ~28 min, cached in the store) →
    `mm0/statements/rv64_memory.{mm0,mm1,roots}` for
    `LeanRV64D.Functions.execute_LOAD`/`execute_STORE`, 3 sorts / 30 term-def /
    13 ax-thm, `mm0-c` exit 0. Unset and unbuildable → that stem is skipped and
    `deref`/`store` are reported NOT COVERED, never silently assumed.
  Both go through `lean` → `lean4export` → `lean2mm0-stmts emit --only` →
  `mm0-rs compile` → `mm0-c < .mm1`. The script then prints the `ISA_REGISTRY`
  diff: **15 opcodes map to a converted declaration, 8 are structural, 0 are
  uncovered**. The table is closed (an `isa.rs` opcode with no entry fails it),
  and "mapped" is checked against the artifact, so a missing declaration reads
  `NOT IN ARTIFACT`, not as covered.  `check` re-converts and diffs both stems.
  `make w9` runs this `check` together with `scripts/mem-refines` and asserts
  each one's acceptance line (`check: 2 stem(s) match`, `0 not covered`,
  `REFUTED as stated`), so a regressed converter or a lowering that grew the
  model's signature fails a command instead of reading as a `[gap]`.
  Runs under `nix develop "git+file://$PWD#lean"`: that shell carries lean,
  lean4export, `mm0`, python3 and **both** model paths (`$RISCV_MODEL`,
  `$RISCV_MEMORY_MODEL`) on purpose, so entering it may trigger the one-time
  28-minute Sail build. `.github/workflows/w9-isa-model.yml` runs `check`
  weekly, because a PR should not wait for that build.
-  `scripts/mem-refines` (see `make w9`) — refutes the tempting shortcut "the JIT's memory op
  lowers to the model's `execute_LOAD`/`execute_STORE`", for **both** opcodes.
  It reads the *committed* artifacts (`mm0/clif.mm0`'s `load__i64__r_i64`: 1
  operand, 1 result; `store__*`: 2 operands, 0 results) and the *pinned*
  `LeanRV64D/InstsEnd.lean` signatures (`execute_LOAD`: 5 arguments;
  `execute_STORE`: 4; both return `SailM ExecutionResult`, check `width ≤
  xlen_bytes` and can fault in `vmem_read`/`vmem_write`), so the verdict is
  measured instead of retyped. Exit 0 = refuted (the expected state), 1 = the
  shapes now agree (re-argue the correspondence), 2 = evidence missing.
  `verify-invariants` runs it and pins the refutation as `[pass]`.
- `scripts/mm0-comparator` — W3's held-out statements. Both modes take
  `--approved-root DIR`, and `approve --dry-run` prints what would be approved
  and writes nothing: the flow for an unapproved artifact (`isa`, `slot_layout`,
  `store_projection`, `store_projection_toy`) is *approve into a scratch root,
  `check` against it, read the statements, then let the owner commit
  `mm0/approved/<artifact>.stmts`*. `isa`'s emitter is registered
  (`scripts/isa-encdec`), so its approval is anchored to a current artifact; the
  hand-written specifications have no emitter and the comparator says so.
- `mm0/store_projection.mm0` — the claims that *survive* that refutation, for
  both memory opcodes, stated and cited but not asserted: `aligned8 a ⇒
  in_linear_space a ⇒ store_projects s a v` and `… ⇒ load_projects s a` over
  uninterpreted state transformers (`jit_store_i64`/`isa_store_w8`,
  `jit_load_i64`/`isa_load_w8`), with `-- certificate: [gap]`, three pinned
  citations and **no** `axiom`/`theorem`. `mm0-c` accepting it means "the
  obligation is well-formed", never "it holds". It declares its own vocabulary
  (`-- theory: self-declared vocabulary`), which `verify-invariants` accepts in
  place of a `used=` coverage list; membership is still a W3 `[gap]` until the
  owner approves it. The file is named for the obligation it was created for
  and now covers load as well — the refutation leaving only a projection is the
  same argument for both.- `mm0/store_projection_toy.mm0` + `.mm1` — the projection *proved* on a toy
  model: a **three-cell** memory whose capacity is characterised by one axiom per
  case — a refresh (`at_put_held_other`, with `put_held`/`put_held_other`: a
  write to an address the memory already holds replaces that cell and leaves the
  other one; `at_put_held_keeps` + `at_put_held_no_new`: the *set* of held
  addresses is unchanged, which is how the toy proves "a refresh never evicts"
  without arithmetic — membership in both directions instead of a count), a free cell (`at_put_other`, guarded by `notfull m`: a write
  leaves other addresses alone), full and evicting (`at_drop`: four
  pairwise-distinct stores and the earliest address reads `unset` — four because
  a three-cell memory evicts on the fourth, so the depth is in the axiom), full and
  keeping (`at_keep_prev`: the oldest of the three survivors still reads its own
  value). Two guards are
  load-bearing: an *unconditional* preservation axiom would describe unbounded
  memory and contradict `at_drop`, and `put_held_other` needs `notfull` because a
  write to a fresh address in a full memory evicts the oldest cell. `notfull` is
  **not** closed under `put` — `put_init_notfull` and `put2_init_notfull` say only
  that the states after one and after two stores on `init` have a free cell, and
  the closure stops there because the third store fills the memory; an earlier
  closure axiom (`put_notfull`) was removed when the refresh case showed it cannot
  hold with `at_drop`, and `put2_init_notfull` is the counting step that keeps the
  third store's proof from needing arithmetic.
  Twenty cited axioms all told, `3 sorts, 18 term/def, 48 ax/thm`, no `sorry`.
  **Toy theorem count: 28**
  (keep the number on this line) — pinned: `verify-invariants` compares the
  number of `theorem`s in the `.mm0`, the number of `pub theorem`s proved in the
  `.mm1`, and this line and the matching one in the plan, so a new theorem with
  stale prose is a red gate. **Toy axiom count: 20** — pinned the same way, from
  the `axiom` declarations on both sides. **Toy shape: 3 sorts, 18 term/def, 48 ax/thm**
  (keep it on one line) — the figures `mm0-rs` prints, which is the authority
  here: `provable sort Prop` counts as a sort and a `grep '^sort '` would say 2.
  This is required at every site that quotes the shape, not just here: both
  documents' bullets and their backticked prose, plus the plan's transcript
  `[detail]`/`[pass]` lines — so a figure that grew (12 → 17 when the width-4
  definitions landed) cannot survive in one paragraph after another is fixed. Both
  sides are defined as the same write/read. The store-then-load-*elsewhere* pair
  (`toy_jit_store_load_other`, `toy_isa_store_load_other`) returns the value
  from before the store;  `toy_jit_two_cells_preserved` shows a store at `a` then
  a distinct `b` leave `a` readable (axioms composed through `veq_trans` with a
  `notfull` fact from `put_init_notfull`); `toy_jit_refresh_keeps_other_cell`
  shows a third store at `a` refreshing it in place while `b` stays readable,
  `toy_jit_refresh_keeps_held_cells`/`toy_jit_refresh_no_new_held` show `b` stays
  held and nothing new becomes held, and `toy_jit_repeated_refresh_never_evicts`
  shows two refreshes at `a` still leave `b`'s value readable.
  `toy_keep_store_invisible`/`toy_keep_refresh_misses_the_write` exist to show
  where the projection *fails*: `isa_store_w8_keep` is a second width-8 store
  with the other refresh order (first write wins), so on the same refresh trace
  it reads the value from before the refresh while the JIT side reads the new
  one. Both sides of every projection theorem are the same `put`, which is the
  assumption this pair makes visible.
  `toy_jit_refresh_visible`/`toy_isa_refresh_visible` close the same case from the
  other side — the *refreshed* address reads the new value back — and carry **no
  `vne` guard**, which is the asymmetry worth knowing: what a write returns at its
  own address does not depend on the rest of the memory, whether the other cell
  survives does. A second width
  (`jit_store_i32`/`isa_store_w4`, `jit_load_i32`/`isa_load_w4`, with an
  uninterpreted `narrow4` carrying only idempotence and congruence) proves the
  same shape at width 4, plus `toy_width4_load_after_wide_store` and
  `toy_width4_store_load_roundtrip` — there the narrowing is in the *proof*,
  since the width-4 loads narrow on the way out and `veq` alone cannot push
  through a function without `narrow4_cong`;
  `toy_jit_third_store_kept` reads the first of three stores back (the counting
  step `put2_init_notfull` exists for); `toy_jit_fourth_store_drops_oldest` and
  `toy_jit_fifth_store_drops_next` fire the *same* `at_drop` axiom on a fresh and
  then a one-store memory — it is stated for an arbitrary `m` so it fires at any
  depth, which is what makes the fifth store a theorem rather than a new
  assumption; and `toy_jit_recent_cells_readable` says what the fourth store kept
  (`at_keep_prev`, reading the oldest of the three survivors).
  Dropping `vne a b` or `notfull m` makes the different-address theorems
  unprovable, so both side conditions are load-bearing. The axioms are
  consistent by a witness that `scripts/toy-witness` **executes** (memory = a
  list of at most three `(address, value)` pairs, `put` = cons on the list with
  that address's pair removed and the result truncated to three, `get` = first
  match else `unset`, `held` = the address is one of the pairs, `notfull` =
  fewer than three): every axiom is checked over all memories of at most three
  cells on a four-atom domain (four, because `at_drop` needs four distinct
  addresses and a smaller domain would make it vacuous), and the six rejected
  candidates — `put_notfull`, the unguarded
  `at_put_other`/`put_held_other`, `at_drop_at_two_cells`,
  `keep_store_visible`, and
  `narrow4_is_identity` — must each *fail*
  there with a counterexample, so the guards (and the width-4 narrowing) are
  checked rather than assumed. Its `[hold]` rows are exhaustive over that finite
  domain — a check of the model's code, not a proof over arbitrary `Val`; its
  `[reject]` rows are domain-free, because a counterexample memory embeds in any
  atom type with at least that many atoms and a `[model]` row checks that the
  operations commute with the injective renaming the embedding uses. It is
  `[gap]`, never a pass, where `rustc` is absent. The same axiom set is
  *proved* a second time, in `mm0/toy_witness/ToyWitness.lean`: each axiom is a
  theorem about an arbitrary atom type with decidable equality (no finite
  enumeration, no atom-level threshold — `narrow4`'s two are at `Fin n` for
  every `n ≥ 2`), with the capacity-3 carrier in the model's type, and the
  proofs structural on the list that models a memory. The six rejected
  candidates are theorems about their negations, exhibited at `Fin 4` (a
  refutation needs one domain), and a degenerate `veq := ⊤` model is noted as a
  consistency trap (content comes from the instance proofs).
  It proves the toy's **28 theorems** as well as its 20 axioms, each as a chain of
  axiom instances (a missing axiom would leave the theorem that needs it open),
  and `verify-invariants` runs `lean` on it in the `lean` shell and requires the
  printed axiom, theorem and rejected-candidate lists to agree with the `.mm0`
  and the Rust witness. It also mutates five model definitions (`take 3`,
  `drop`, `notfull`, `narrow4`, `keep`) and requires elaboration to fail, since a
  proof's own failure mode is a definition it no longer depends on, audits
  `#print axioms` for all 48 declarations (nothing outside
  `propext`/`Classical.choice`/`Quot.sound`), and scans the comment-stripped code
  for `sorry`/`admit`/`native_decide`/`ofReduceBool` — the audit cannot see a
  `sorry` in a declaration it does not name. It proves the shape and says so in its header; it does not connect Cranelift or Sail to `put`/`get`,
  and `verify-invariants` pins that it cites the *current* `store_projection.mm0`
  and uses the same four transformer names, so a proof cannot drift onto a
  neighbouring (easier) statement — and that pin has its own control: the gate
  re-runs the digest comparison against a copy of the claim with `store_projects`
  renamed and requires it to report the pin stale, so a comparison that could not
  fail is a red gate rather than a green one; and it checks that the witness holds exactly
  the axioms the artifact declares, so a new axiom cannot slip past the model.
  It also mutation-tests the file thirteen ways,
  one per substantive axiom: rewriting `at_put`'s conclusion to "read
  unchanged", `at_put_other`'s preserved value to `unset`, `veq_trans`'s
  conclusion to reflexive, `at_drop`'s `unset` to the written value,
  `at_keep_prev`'s `vy` to `vx`, `at_put_held_other`'s preserved value to
  `unset`, `put_held`'s held *address* to the held *value*,
  `put_init_notfull`'s one-store state to `init`,
  `put2_init_notfull`'s two-store state to `init`, `at_put_held_keeps`'s kept
  address to the written one, `at_put_held_no_new`'s `held m y` to
  `held init y`, `narrow4_cong`'s conclusion and `narrow4_idem`'s inner
  `narrow4` must make `mm0-rs` refuse at
  `toy_jit_store_visible`, `toy_jit_store_load_other`,
  `toy_jit_two_cells_preserved`, `toy_jit_fourth_store_drops_oldest`,
  `toy_jit_recent_cells_readable`, `toy_jit_refresh_keeps_other_cell` (twice),
  `toy_jit_two_cells_preserved`, `toy_jit_third_store_kept`,
  `toy_jit_refresh_keeps_held_cells`, `toy_jit_refresh_no_new_held`,
  `toy_width4_load_after_wide_store` and `toy_width4_store_load_roundtrip`
  respectively — a proof that survived its own
  axiom being falsified would be deriving the claim from something else, and is
  reported as a failure. A mutation that fails to apply is a failure too. The
  control was itself tested by making the mutations no-ops (patterns that match
  nothing, patterns that change nothing, patterns that break the file
  elsewhere): each scratch run reports `1 failed`, exit 1.
- `scripts/l11a-ordering-drift-control` — the L11a negative control: it inserts
  `dump m;` above the index-closure audit in a scratch copy of
  `lib/liquid/LiquidCheck.ml` and requires the gate (reading that copy through
  `AUSTRALVM_LIQUID_CHECK`) to exit 1 with exactly `[FAIL] L11a: the liquid pass
  runs the index-closure audit before emitting`, the other two L11a pins still
  passing; the unmutated copy must pass all three first. Writing it exposed a
  weak pin, which now compares against the *first* `dump m;`. The gate pins this
  control's wiring (`L11a: the ordering pin has a negative control`, `… moves the
  copy through the gate's own hook`) because the gate cannot run the control —
  the control runs the gate.
- `scripts/l11b-conversion-drift-control` — the L11b negative control: it
  rewrites `LiquidIndex.ml`'s single `convert` decision line twice in place
  (restored by a trap, re-hashed, suite re-run) and requires the real suite to
  fail in opposite directions — `if true then Convertible` trips the four
  refusal cases while the acceptances still pass, `if false then Convertible`
  trips the four acceptance cases while the refusals still pass. Each of the
  eight is required **by name in both directions**: the four of the affected kind
  must print their `FAIL` line and the other four their `ok` line (the suite
  spells the two verdicts differently — `FAIL m + n must not convert to n + m`
  against `ok   m + n is not convertible to n + m` — so both names are listed),
  and the run must report exactly four failures. Requiring only the failures
  would let a mutation that also broke a case it should leave alone pass; the
  swapped-expectation copy of this control (expecting the permissive run's
  acceptances to `FAIL`) exits 1 naming all four, so the per-case assertion is
  load-bearing. It mutates in
  place because `LiquidTypes` needs `Stages` from `austral_core`, so a separate
  link of the module is not possible; the restore is asserted, not assumed. It
  needs `dune`, so run it under `nix develop "git+file://$PWD#ocaml"`. Its own
  control: making both mutations no-ops reports `the mutation did not apply`
  and exits 1. The gate pins its wiring too (`L11b: the conversion pin has a
  negative control`, `… mutates the rule LiquidIndex declares`, `… re-checks the
  tree after restoring`), and the anchor pin is the load-bearing one: rename the
  rule in `LiquidIndex.ml` without updating the control and the control exits
  early on its own "expected exactly one conversion rule" check, which nothing
  on the default path would run — the gate now says so instead.
- `scripts/w3-emitter-drift-control` — the §W3 negative control that goes through
  the gate itself: it rewrites `isa`'s emitter cell in a copy of the plan to name
  a command `EMITTERS` does not hold for that row and requires
  `verify-invariants` (via `AUSTRALVM_PLAN`) to exit 1 with `[FAIL] W3: every §W3
  emitter cell names a real command`, the row named in the detail, and every §W3
  line that does not read that row still passing. One full gate run.
- `scripts/w93-composition-drift-control` — the same negative control for the
  §W9.3 composition table: it rewrites `store_projection_toy`'s `20 axioms` cell
  to `21` in a copy of the plan (the `70` stmts cell untouched, so only the
  composition column can fire) and requires the gate to exit 1 with
  `[FAIL] W9.3: the composition table's counts match the files it records`, the
  detail `store_projection_toy(says 21 axioms, staging 20)`, and the closure,
  approval-control and §W3 lines unchanged. One full gate run. The gate also
  re-reads the four cells and requires a literal comparison for every counted
  word they name (`… every field word a composition cell names has a literal
  comparison (8 word(s))`), so a field added to a cell cannot go uncounted.
- `scripts/w93-closure-drift-control` — the §W9.3 negative control for the
  `closure-sha256` column, which the count drift cannot exercise: it flips the
  first hex digit of `store_projection_toy`'s digest cell in a copy of the plan
  (every count cell untouched) and requires the gate to exit 1 with `[FAIL]
  W9.3: the composition table's closure digests match the files it records`, the
  detail `store_projection_toy(says '<row12>…', staging '<real12>…')`, the counts
  line and the approval control still passing. One full gate run.
- `scripts/w93-certification-drift-control` — the §W9.3 control for the
  refinement's certificate line: it copies `mm0/clif.mm0`, rewrites
  `-- certificate: [gap]` to claim `mm0/approved/isa.stmts` (which does not
  exist), and runs the gate with `AUSTRALVM_CLIF` at the copy, requiring exit 1
  with `[FAIL] W9.3: certification state matches mm0/approved/`, the detail
  `no isa approval, yet clif.mm0 does not say [gap]`, and the `clif_step`
  vocabulary, pinned citations and §W3 lines unchanged. One full gate run.
- `scripts/w3-approval-state-drift-control` — the §W3 control for the state
  cell: it rewrites `isa`'s cell in a copy of the plan to `` `[pass]`, approved in
  `mm0/approved/` `` with no `mm0/approved/isa.stmts` behind it, and requires
  `verify-invariants` (via `AUSTRALVM_PLAN`) to exit 1 with `[FAIL] W3: the
  plan's §W3 stmts column matches what each artifact holds`, the detail
  `isa(says approved, but …/isa.stmts does not exist)`, exactly the three other
  lines that read the same cell red (digest column, approved-row emitter check,
  staging count) and nothing else, and the shape check plus the emitter column
  still green. One full gate run.
- `scripts/w3-drift-controls` — one case for each of the thirteen checks the
  census classified `copy`, plus the certification run with an approval present
  (`<case>`, `--list`, `all`, `-j N`); 14 cases: Phase D's two
  trust splits and its four coverage/vocabulary claims, the W8 hypothesis pin,
  the `clif_step` vocabulary and pinned citations, the witness's axiom table, the
  memsafe edge census, the Liquid golden, and the refinement's certificate with an
  approval present. Each builds a scratch copy, drifts one claim in it, runs the
  whole gate with the read-only override that points at the copy
  (`AUSTRALVM_MM0_DIR`, `AUSTRALVM_LIQUID_GOLDEN`, or the existing
  `AUSTRALVM_CLIF` / `AUSTRALVM_APPROVED_ROOT`), and requires exit 1 with the
  expected `[FAIL]` line. One full gate run per case; logs kept under
  `$W3_DRIFT_LOGS`. A case is one gate run and nothing shorter, so the only
  speed-up is `-j N`: cases are independent (own scratch tree, own `TMPDIR`, the
  repository only read), and measured 2026-11-18 all 14 cases take 62m43s at
  `-j 1` and 44m9s at `-j 2` — 18m34s (30%) off, 1.42x and not the 2x the two
  cores suggest, because a gate run does not itself parallelise. The last
  `copy` row, `W3: $_stem matches its approved statements`, is
  `w3-approved-row`: it copies the whole approved root into scratch and weakens
  one approved axiom there, so the check runs without the tree holding any new
  approval. Each run appends its own wall clock to
  `$W3_DRIFT_LOGS/timings.tsv` (and to `$W3_DRIFT_RECORD` when set); the rows
  kept in the tree are `docs/data/w3/drift-timings.tsv`, which the plan quotes
  instead of a retyped number. `liquid-golden` is the one case conditional on
  the shell: the gate guards that check on `command -v why3` and reports it as
  an explained `[gap]` when the prover is absent, so the drifted copy has no
  `[FAIL]` to produce. The case then prints `[gap] liquid-golden: why3 not on
  PATH …` and the summary reads `13 case(s) held, 1 not covered in this shell`
  — never a silent pass, which is what lets `make test` run past
  `drift-controls` under `nix develop .#ocaml` (where the `default` shell that
  carries why3 does not build).
  `make drift-controls` runs it at `DRIFT_JOBS` (default `nproc`) plus the
  seven read-only scripts listed in `READ_ONLY_CONTROLS` — the four §W3/§W9.3
  cell controls, `w3-approval-state-drift-control`, and the two L11 controls
  (`l11a-ordering`, `l11b-conversion`) — then `obj-drift-control` alone, because
  that one edits the tree and rebuilds. `make test` depends on it. The two
  parallel halves together measure 1h25m27s serial against 1h01m31s at two jobs
  (2026-11-18, before the two L11 controls joined the loop; the ~30% saving is
  the point, not the wall clock). It never
  writes `mm0/approved/` — the approval a case needs lives in a scratch root — so
  no control can approve.
- `docs/data/w3/probes.tsv` — the measured probe evidence (retired emitter,
  stale `.mm1`, §W3's emitter and state cells, §W9.3's composition count and
  closure-digest cells, the refinement's certificate line in both directions, the
  model's `proves: 0 theorems` pin, and one row per `scripts/w3-drift-controls`
  case):
  artifact, perturbation, the check that fired, its message, Never edited by the gate; re-read every run, which fails when a row cites
  a `gate_line` the script no longer emits, an `expected_message` skeleton whose
  literal chunks (`…` marks the interpolated parts) are no longer in the script, or
  a `gate_exit`/`gate_total` pair of the wrong shape (a recorded run must have
  exited non-zero; `-` means the probe is exercised in-gate, not by a gate run).
- `scripts/check-census` — the counter behind the plan's "which checks have no
  measured failure" paragraph, so those numbers are re-measured, not trusted:
  it censuses every `check "…"` in `scripts/verify-invariants` as statements,
  distinct texts (a trailing count parenthetical dropped — it reports a number,
  not a different claim), the texts the plan's census table names, the texts a
  `docs/data/w3/probes.tsv` row records, the residue with neither, and the
  `class` totals (`control` / `derived` / `copy` / `tool` / `none`). It exits 1
  when the plan no longer quotes the measured counts — including the `copy`
  count, which is the gap — so a table that grows without the paragraph moving
  is caught here, and `make check-census` runs it. Kept out of
  `verify-invariants` because it reads the plan's prose rather than an artifact,
  which also means nothing runs it for you: adding a `check` to
  `scripts/verify-invariants` makes it exit 1 until the plan quotes the new
  numbers. Last measured 2026-11-19: `223 statements, 136 distinct, 22 probe
  rows (20 matched), 0 neither, copy 0`.
- `scripts/lib/arch.sh` — the one recorded-arch reader for the W5 object
  digest (`avm_recorded_arch`, `avm_host_arch`, `avm_arch_gap`), sourced by
  `verify-invariants`, `obj-drift-control` and `w5-arch-gap-control` so all
  three word a host mismatch the same way. The committed certificate records
  `arch=x86_64` (2026-11-19), so the obj comparison is `[pass]` there and the
  documented `[gap]` elsewhere; `scripts/obj-drift-control` expects the obj
  line on a matching host and the cps line on a mismatched one, and exit 2
  stays an explained gap in `make test`.
- `scripts/w5-arch-gap-control` — the arch-gap path itself: on this host the
  matching-arch run never takes it, so the control sets
  `AUSTRALVM_HOST_ARCH` to a label that is neither the recorded nor the real
  arch and requires the gate to exit 0 with `W5: object digest not comparable
  (recorded arch=…, this host is …)`, no obj `[pass]`, and the cps `[pass]`
  still there. Read-only, one full gate run; in `READ_ONLY_CONTROLS`.
- W5's failed comparisons carry a staleness line. The certificate's `cps=`/`obj=`
  digests describe one `austral` and one bridge build, so when a comparison
  fails the gate prints `staleness: — austral is newer (…T14:08:25) than
  mm0/flux1d_certificate.mm0 (…T13:57:34), so the recorded digests may describe
  an older compiler build; re-emit it: …`: the mid-rebuild case reads as an
  artifact that predates the compiler, not as silently drifted code. It is a
  diagnosis and never a skip — the two digest `[FAIL]`s stay red. mtime alone
  cannot refuse the comparison: a rebuild of identical sources leaves the
  digests equal and the tree green (measured 2026-11-19 with `austral` 34
  minutes newer than the certificate), and CI writes the artifact at checkout
  before `make bridge` writes the binary, so an mtime-only refusal would be red
  on every run. The note is itself a comparison with a control:
  `[pass] W5: the staleness note is silent when the artifact is newest, and
  names the newer binary` stamps a scratch pair both ways.
- `scripts/obj-drift-control` — the W5 negative control: a semantics-preserving
  emitter edit moves `code.obj`, the gate refuses the certificate, and the tree
  is  restored. It rebuilds the compiler twice, so it is not in
  `verify-invariants`; run it under `nix develop` (needs `dune`). It resolves the
  bridge the way the gate, `scripts/cps-corpus` and
  `scripts/check-adapter-descriptors` do (repo root, else `target/release`), so a
  clean `make bridge` layout is exercised instead of reported as `[gap]`;
  measured there 2026-11-19: held, `obj=26bb3ab3…` → `524f7432…` with the
  program's exit status unchanged, gate `[FAIL] W5 (checked): the certificate's
  obj digest … (x86_64)`, pinned digests back after the restore. The
  `nix-pkg` CI job also runs `verify-invariants` inside `.#ocaml` after the
  OCaml suite, against the built `austral`/bridge and the sibling `unfer`
  checkout, so the checks that need artifacts are measured there instead of
  reported as gaps. The standalone `verify-invariants` job was upgraded to the
  same shape: it installs nix and Rust, runs `make bridge`, and then runs the
  gate without staging a root `.so` — the gate, `scripts/cps-corpus` and
  `scripts/check-adapter-descriptors` resolve the bridge at the repo root when
  it is there and under `safestos/cranelift/target/release` otherwise, which
  is the layout `make bridge` leaves and CI has. Measured from a clean tree
  with no root copy: `157 passed, 8 explained gap(s), 0 failed` in 197 s.
- Entry points outside `scripts/`: the `Makefile` (`make all` / `make test` /
  `make bridge`, all guarded by `check-bridge-fresh`; `make w9` for the two
  lean-shell checks below), `run-examples.sh`,
  `test-programs/runner.py`, `demo_module/run_demo.sh`.

```bash
nix-shell -p python3 --run "bash scripts/verify-invariants"
nix-shell -p python3 --run "python3 scripts/check-links"
```

Unapproved artifacts are `[gap] W3: <artifact> has no approved statements` lines.
The gate prints two measured-size lines: `w3-pending:` (what each unapproved
artifact would hold out) and `w3-approved:` (what each approved one holds now, so
a shrunk artifact is visible even though its statement diff is the enforcement).
The plan's §W3 `stmts` column is checked on every run against the numbers the
row's *state* selects: today's staging for a pending row, `mm0/approved/<stem>.stmts`
for an approved one. A cell that disagrees fails `verify-invariants` with the row
named, as does half an approval (a committed copy whose row still says pending —
`flip this row's state cell` — or an approved row whose copy is missing or holds
a different number of statements), so edit the table only by reading that line.
A pending row also has to *re-derive* on every run — its artifact must stage,
because the row is the owner's next decision and a stale `.mm1` or a broken
generator should read as a finding about the artifact, not as "not measurable
here" — and the failure quotes the comparator's own line. `check` and `approve`
both default to `AUSTRALVM_APPROVED_ROOT` when it is set (the flag still wins),
so the comparator and the table cannot be pointed at different roots.
The table's `emitter that owns it` column is checked too: a cell
naming a command (`` `scripts/isa-encdec check` ``) must name a file that exists
and is executable with the script *and* mode `python3 scripts/mm0-comparator
emitters` reports for that stem, a cell saying `none` must be genuinely
unregistered, and any other wording fails — the registry in `EMITTERS` is the
single source of truth, so add an emitter there first and then update the row.
The `state` column is the approval record, so it has exactly two shapes:
`` `[gap]`, awaiting owner commit `` for a pending row and
`` `[pass]`, approved in `mm0/approved/` `` for an approved one, each optionally
followed by `; ` and an artifact-specific reason. `pending`, `TBD`, a bare
`[gap]` or `[pass]`, an approved cell that does not name `mm0/approved/`, a
trailing `;` with nothing after it, or a different reason (`blocked on X`) all
fail — every unapproval here waits on the same thing, an owner commit into
`mm0/approved/`. Flipping that one cell *is* the approval: the row stays, the
counts and the staging set are derived from the two states, and
`scripts/stage-approvals` stops staging the artifact once its copy exists. For
an approved row the emitter cell is re-run, not just read: the gate approves the
row again into a scratch root and requires the result to be byte-identical to
the committed copy, which is what keeps `emitter that owns it` true after the
`stmts`/digest columns start reading a fixed file. That check carries its own
negative control: the same code, given a registry with `isa` retired, must report
the retirement for the row that names the emitter, and report nothing with the
real registry over an identical staging.

## Gotchas

- Cross-module foreign calls resolve to the *interface* decl_id, not the body's
  `External_Name`. `Compiler_cps.ml` keys foreign functions by **both** decl_id
  and source name so imported `uk_*` calls lower to the external symbol
  (e.g. `uk_version`) instead of a qualified Austral name the JIT can't resolve.
- The JIT authorization principal is the **module** name (A10): manifest load
  records `[module] name` as the deployment principal and
  `check_call_permission` attributes each gated `uk_*`/`uz_*` call to it
  (fallback: CPS header's module name, then the caller for legacy binaries).
  `modhost` checks the manifest decision directly, and the live gate itself
  is tested in `cps.rs::a10_live_grant_tests` (see `unfer/docs/MODULES.md` §6).

## Changelog

`CHANGELOG.md` is **generated** from conventional commits (git-cliff,
config in `cliff.toml`, pattern adapted from `emthin/cliff.toml`).
The repo's pre-conventional/vendored history is excluded by design
(`filter_unconventional`), so a large skip count in git-cliff's warning
is expected. Refresh with:

```bash
nix run nixpkgs#git-cliff -- -o CHANGELOG.md
```
