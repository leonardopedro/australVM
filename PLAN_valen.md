# PLAN V — Valen-inspired direction: group borrowing and Rust interop, without growing the kernel

Additive plan, parallel to `PLAN_liquid_types.md` (Plan L) and
`PLAN_parallel_australvm.md` (Plan B). It rides the same two seams those plans
ride: the ghost-pass discipline of Plan L (checks that erase before CPS) and
the plugin boundary at `lib/Compiler_plugin.ml`.

**Status at handover (2026-10-16)** — this document and the two attribution
rows (`ATTRIBUTION.md` here, the workspace ledger at `../ATTRIBUTION.md`) are
the deliverables of P0. No compiler code is changed by this plan. Everything
from P1 on is unbuilt and unverified; each phase below names its own
verification step so the first agent to execute it starts from evidence, not
from prose. Build note that applies to all of it: `make bridge` first (the
root `libaustral_cranelift_bridge.so` staleness gate — AGENTS.md gotcha).

## §0 The inspiration, and its licence

**Valen** (`github.com/valen-lang/valen`, Apache-2.0 — LICENSE read firsthand
2026-10-16) is a prototype systems language by Evan Ovadia and collaborators.
Its two distinguishing claims, both from its README and the accompanying
articles:

1. **Group borrowing** — mutable aliasing *within a group* of objects, on top
   of a single-ownership foundation, with no runtime overhead. Explained in
   the article "Group Borrowing: Zero-Cost Memory Safety with Fewer
   Restrictions" (Ovadia, 2025-08-28, crediting Nick Smith's original
   formulation). The pitch: Rust's aliasing-xor-mutability rule is
   *conservative* — it rejects valid programs — and group borrowing is the
   point in the design space that keeps zero-cost safety while admitting the
   aliases a program actually makes.
2. **Ecosystem reach** — Valen calls into existing Rust libraries; the
   "Golden Spike" article documents the route (a patched `rustc` on branch
   `per-instance-mir` in `valen-lang/rust`).

Both are *design ideas*, and this repo takes them as ideas only, per the
workspace attribution policy (`../ATTRIBUTION.md` policy 1: adapt ideas, do
not copy source). Nothing from Valen's tree is vendored here. If that ever
changes, Apache-2.0 permits it with the licence notice retained — and the
ledger row must say so in the same commit (policy 4).

## §1 Why this lands on australVM specifically

Austral already has two of the three properties Valen optimises for:

- **memory safety through linearity** — cells are linear, ownership
  transfers are checked, and destruction is deterministic (Theseus-shaped
  capability tokens; see `ATTRIBUTION.md`);
- **speed without GC** — the CPS lowering and the cranelift JIT in
  `safestos/cranelift/` are lightweight by design: no runtime, no GC, no
  reflection. The `uk_*`/`uz_*` ABI is frozen.

What Valen's writing names as the remaining gap is exactly what this plan
pursues:

1. **Aliasing flexibility**: Austral's linearity gives *no* aliasing at all.
   Programs that update several related cells together must thread them
   through destructuring and reconstruction — safe, but the type system
   rejects a shape the programmer can see is fine.
2. **Ecosystem reach**: Austral modules reach the outside world through
   declared `uk_*` symbols and the `au_*` registration bridge. That is a
   deliberate trust boundary (every call is gated by
   `check_call_permission` in `safestos/cranelift/src/cps.rs`), but it means
   "call this ordinary Rust crate function" is not yet a thing an Austral
   module can say.

## §2 The architecture constraint (owner decision, 2026-10-16)

> australVM's compiler stays JIT and lightweight. The Rust compiler is
> integrated **either** as an optional plugin over the existing seam **or**
> as a *new* compiler that includes australVM and the Rust compiler as
> components/libraries/plugins — never by baking rustc into australVM.

Two admissible shapes, and this plan keeps both open until a phase has evidence:

- **Shape A — plugin over `lib/Compiler_plugin.ml`.** The same seam the WhyML
  gate uses (invoke, never link; see `../unfer/docs/WHYML_CYCLE.md` and Plan
  L's LGPL discipline). A Rust-integration plugin consumes declarations,
  emits lowered forms, and the kernel keeps its frozen surface. A plugin that
  is absent degrades exactly like the Why3 gate does: an *explained gap*,
  never a false green.
- **Shape B — an umbrella compiler.** A new workspace-level binary (outside
  this tree at first) that includes australVM — reachable as a library
  through the Rust bridge, whose crate already carries `crate-type` `rlib` —
  and the Rust toolchain as components. One CLI, two compilers underneath.
  The Rust side is invoked as a **subprocess** (`cargo build`), mirroring the
  Why3 rule: invoked, never linked. rustc-as-a-library would pin a nightly
  compiler into our dependency graph; that cost is not paid unless a phase
  proves the subprocess shape insufficient.

## §3 Group borrowing, mapped onto Austral's linear cells

The Valen/Nick Smith model, in one sentence: single ownership stays the
foundation, but a declared **group** of objects may alias mutably *inside*
the group — the group is the unit of exclusive access, not the object.

Mapped onto Austral, in the same spirit:

```
group g = (cell a: Foo, cell b: Bar);   // not syntax — illustrative
  let r = &mut g;                        // one mutable borrow of the group
  r.a.field = 1;  r.b.field = 2;         // aliases inside the group: fine
// region ends: borrows end, a and b are individually linear again
```

Design commitments, each chosen to keep the kernel untouched:

1. **Ghost, like Plan L.** The group region is a *checking* construct on the
   MTAST. It is erased before monomorphization/CPS lowering: generated code
   sees ordinary linear cells. The frozen `uk_*`/`uz_*` ABI, the grant
   vocabulary, and the JIT do not change. Plan L's `LiquidPragma` carrier in
   `lib/Common.ml` is the precedent for attaching checking-only annotations
   to terms the backend never sees.
2. **One external owner.** Entering a region *consumes* the group's cells
   into the region; there is no path from outside the region into its cells,
   and no path from inside to outside except by ending the region. That is
   what makes the aliases inside safe: the group, not the cell, is the
   linear unit for the region's duration.
3. **Regions are lexically scoped.** No higher-order "return a group borrow".
   First version: a region is a block; borrows cannot escape it (this is the
   check that falls out of Austral's existing liveness analysis rather than a
   new one).
4. **Composes with Plan L.** Refinements attach to group cells the same way
   they attach to ordinary ones; the passes run in either order because both
   erase.

What it buys concretely, in this repo's own code: patterns like the
`data`/`len` buffer pair of `examples/kernel/UnferKernel.aui`
(`Address[Nat8]` + `Int64` length, moved and updated together), and
multi-cell records in the module corpus that today must be destructured and
rebuilt across every update.

**Verification for P1** (the phase that implements this): the group pass gets
its own tests in `test-programs/` — positive programs that must compile,
negative programs that must be *rejected at the region boundary* (borrow
escapes, external alias during a region, use-after-region of a moved cell) —
run through the existing `test-programs/runner.py`. Nothing in the Why3 gate
or the JIT test suites should need to change; if a change there is required,
that is evidence the erasure boundary leaked, and the phase stops.

## §4 Rust interop, in tiers over the existing boundary

**Tier 0 — what exists.** `safestos/cranelift/src/lib.rs` `cranelift_init()`
registers `au_*` runtime symbols and, under feature `unfer-kernel`, the
`uk_*`/`uz_*` tables from `unfer_ffi`. Every gated call passes
`check_call_permission(caller, module, callee)` — the JIT authorization
principal is the **module** (A10), and `modhost` is the enforcement point
that answers the same question out-of-process.

**Tier 1 — `extern rust` declarations (plugin shape A).** An Austral module
declares foreign functions whose types are restricted to the buffer
protocol the kernel already speaks:

```
extern rust fn crate_fn(buf: Address[Nat8], len: Int64) -> Int64
  symbol = "crate_fn"  owner = borrowed;   // illustrative, not syntax
```

- **Lowering**: an `extern rust` call becomes a symbol lookup in the JIT,
  exactly like a `uk_*` call — the symbol comes from a Rust `cdylib` built
  by `cargo` and registered through the bridge's existing data-driven
  tables (`UNFER_SYMBOLS`-style). No new mechanism; a new *table*.
- **The linear boundary is the design work.** Each parameter and the return
  carry an ownership annotation — `borrowed` (the callee must not retain it;
  the caller's cell stays linear across the call) or `owned` (the cell is
  consumed; the callee's return becomes the caller's cell). Borrows cannot
  outlive the call. This is checked on the MTAST and erased before CPS, same
  discipline as §3. The Rust side's safety story is its own `unsafe` contract;
  the Austral side guarantees the *shape* of what crosses.
- **Authorization is inherited, not new.** A foreign Rust call is gated by
  `check_call_permission` under the calling module's principal, exactly like
  `uk_*`. Nothing becomes ambient. A manifest that grants `uk_push` does not
  thereby grant a tier-1 call.

**Tier 2 — the umbrella (shape B).** A workspace-level CLI that takes a
mixed module set, builds the Rust crates via `cargo` (subprocess), registers
their artifacts, and JITs the Austral half through the bridge. The umbrella
*includes* australVM as a component; australVM does not learn about rustc.
(Prototype shipped 2026-10-17 as `umbrella/` — from the same day until
2026-10-18 a git submodule inside this tree, since then a plain
subfolder — see the P3 status in §5.)

**Not adopted, stated plainly**: Valen's route — a patched `rustc` on branch
`per-instance-mir` — is not taken. A forked compiler is a maintenance sink
and a bootstrap-chain liability, and it would put a Rust compiler *inside*
australVM's world, which §2 forbids. If instance-level MIR access ever proves
necessary, it lives inside the umbrella (shape B) as its own component, and
the plugin seam (shape A) keeps working without it.

## §5 Phases

| phase | what | verification |
|---|---|---|
| P0 | this plan + attribution rows | `scripts/check-attribution`, `scripts/check-links` |
| P1 | group-region checking pass (MTAST, erased), `test-programs/` positive + negative cases | `test-programs/runner.py`; JIT test suites unchanged (erasure boundary held) |
| P2 | `extern rust` tier 1: declaration, MTAST ownership annotations, symbol table, one reference crate called end-to-end from a module | a `demo_module`-shaped end-to-end through `modhost`; `cps.rs::a10_live_grant_tests`-style denial test for an ungranted extern call |
| P3 | umbrella prototype (shape B) as a separate crate: `cargo` subprocess + australVM-as-library + one mixed build | the same end-to-end as P2, driven by the umbrella CLI instead of `run_demo.sh` |

Each phase is independently shippable; none of them touches the frozen ABI,
and P1 does not touch the JIT at all. The honesty rule from Plan L applies
here verbatim: a phase that cannot run is recorded as *not covered*, never
as passing.

### P2 status (2026-10-17): shipped, with the e2e relocated — recorded here

Shipped:

- **Declaration + checking + erasure.** `pragma Rust_Ownership` on
  `extern rust fn` declarations: parsed in `CstUtil`, validated in
  `TypingPass` (annotation required, owned/borrowed vocabulary only,
  `Address[T]`/`Int64` parameter shapes), and erased before CPS — the MTAST
  discipline of §3. `Compiler_plugin.foreign_externals_of` counts `rs_*`
  names in the grant mirror.
- **A new table, not a new mechanism.** `RS_FFI_SYMBOLS` in
  `safestos/cranelift/src/lib.rs` registers the reference crate's symbols
  under the `rs_` namespace (`feature tier1-rs`, on by default like
  `unfer-kernel`). The reference crate is `safestos/rsffi/` — the buffer
  pair (`rs_checksum`) and the scalar-only case (`rs_version`).
- **The gate.** `check_call_permission` in `cps.rs` gates `rs_*` against
  the manifest grant list, same principal resolution as `uk_*`.
- **Negative typing tests.** `test-programs/suites/019-extern-rust/`: six
  programs that must be rejected — `rs_*` foreign import with no
  annotation, `Rust_Ownership` on a non-`rs_` import, `Rust_Ownership` on
  an ordinary function, annotation/parameter arity mismatch, annotation
  outside the `borrowed`/`owned` vocabulary, and a parameter type outside
  the buffer protocol — each with recorded stderr, plus the whole existing
  suite unchanged.
- **Execution tests, both sides of the bridge.**
  `safestos/cranelift/tests/tier1_extern_rust.rs` (in-process: granted
  execution of `rs_checksum` over a live buffer checked against an
  independent FNV-1a in the test, scalar `rs_version`, uk-grant ≠ rs-grant
  denial with UK-4001 attributed to the module, DenyAll denial, symbol-table
  sync, and `modhost authorize` answering the rs_ question from a manifest
  file), the `a10_live_grant_tests` rs_ assertions in `cps.rs`, and
  `test/JitTest.ml` (denial without manifest, denial under a uk-only grant,
  granted execution through `CamlCompiler_rust_bridge` with a buffer
  allocated from OCaml).

Deviations and limits, stated plainly:

- **The e2e does not go through `modhost host`.** `spawn_jit_server` passes
  `--allow-all` by design (the host path bypasses manifests), so a
  modhost-spawned run proves nothing about grants. The granted/denied pair
  instead exercises the *same* `check_call_permission` gate and the same
  manifest engine that modhost and the CLI both drive; `modhost authorize`
  — the out-of-process answer — is covered directly for `rs_*` names.
- **The C backend is not a tier-1 path.** An `extern rust` declaration
  emits a C wrapper referencing the `rs_` symbol, which cannot link without
  the crate — the same status quo as `uk_*`. Tier 1 is a JIT feature; the
  positive path lives where the symbol exists.
- **CLI-level denial is advisory, unchanged.** `--use-cps-jit` treats a JIT
  compile error as advisory (it prints `UK-4001` and falls back to the C
  backend, exit 0); that behavior predates this plan and was not changed.
  The hard denial lives at the JIT gate and in `modhost authorize`.

### P3 status (2026-10-17): shipped — umbrella prototype (now an in-tree subfolder)

The prototype is `umbrella/` — a separate crate that depends on this
repo only through the bridge path-dependency. It started life as a
sibling checkout (`../umbrella`), was moved inside the tree on
2026-10-17 as a git submodule, and was integrated as a plain subfolder
on 2026-10-18 (`.gitmodules` entry and the gitlink removed; the old
standalone repository's history kept as a backup outside the tree);
the move changed nothing about §4's boundary — the crate
still contains no australVM code and australVM still does not learn
about rustc; only the path-dependency and default front-end paths
became in-tree relatives.

- **cargo subprocess.** `umbrella` runs `cargo build --release
  --message-format=json` on an arbitrary crate (`modules/rscrate`) and
  takes the cdylib path from cargo's JSON stream — no assumption about
  target layout.
- **australVM as a library.** The bridge is linked as a plain path
  dependency (default features, as this repo builds it); the umbrella
  calls `safestos_load_auth_manifest` (or installs an explicit `DenyAll`)
  and `compile_to_function_named` / `execute_function` in-process — the
  same gate + compile pair P2 tested, driven by another host.
- **australVM as a front-end.** The stock dune-built `./austral` binary
  is invoked as a subprocess (`compile --use-cps-jit --allow-all
  --target-type=tc --emit-cps=…`, the flag set `modhost`'s own
  `spawn_jit_server` uses) to serialize `modules/Demo.aum`; the umbrella
  supplies the child's `LD_LIBRARY_PATH`, so a bare caller needs no
  special shell.
- **Registration without a table.** The crate's `rs_demo_checksum` is in
  neither `RS_FFI_SYMBOLS` nor the CPS binary: it resolves only because
  the umbrella dlopens the cdylib `RTLD_GLOBAL` and the bridge's import
  resolver falls back to `dlsym(RTLD_DEFAULT)` — the artifact
  registration path §4 Tier 1 sketched as "a new *table*" turns out also
  to need no bridge change at all.

Verification (both paths run and recorded here):

- **granted** (`--manifest module.toml`): cargo → dlopen → front-end →
  gate → `RESULT -1781872834555808882` — FNV-1a of the `"au!"` buffer
  the Austral module constructed itself and passed across the `extern
  rust` boundary — exit 0, asserted with `--expect`;
- **denied** (no manifest → explicit DenyAll): JIT compile fails with
  `UK-4001 — 'anon' is not granted 'rs_demo_checksum'`, exit 1, no
  `RESULT` line: nothing executed;
- the demo crate's own `cargo test` holds the same `"au!"` vector as an
  independently computed constant.

Limits, stated plainly: one crate per invocation (no workspace-wide
mixed build yet), single modules only (the multi-module link path is
untested here), the crate is rebuilt every run (cargo incremental keeps
this cheap), and the expected value is asserted by the demo crate's own
FNV contract rather than a cross-language golden file. The phase table's
"driven by the umbrella CLI instead of `run_demo.sh`" is met in
prototype form: same end-to-end as P2, ownership of cargo and rustc
entirely in the umbrella crate (a plain subfolder of this tree since
2026-10-18; it keeps its own `Cargo.toml`/workspace and depends on
australVM only through the bridge path-dependency).

## §6 Considered and not used

- **Patched rustc / per-instance MIR** — §4.
- **LLVM AOT compilation** — Valen compiles AOT to LLVM; australVM's answer
  is the cranelift JIT, and any AOT story belongs to
  `PLAN_parallel_australvm.md`, not here. Two backends would double the
  surface this repo must keep honest.
- **Generational references and reference counting** — Valen's roadmap
  includes both as future flexibility. Austral's linear types already give
  deterministic destruction without a runtime; adding RC would introduce a
  second memory model beside linearity, which is the opposite of lightweight.
- **Vale/Valen source compatibility** — no `.valen` subset is targeted. The
  borrow is from the *published design ideas*, not the language.

## §7 Licence discipline

Valen's tree is Apache-2.0; its articles are copyrighted text by their
authors. This plan adapts described ideas in this project's own words — the
same relationship the ledger records for the Agensh and ClawSwarm rows. If
code is ever copied from Valen (currently: never), Apache-2.0 requires the
licence and any NOTICE to travel with it, and the ledger row must change in
the same commit, saying what was copied and where it landed.
