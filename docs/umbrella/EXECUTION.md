# Umbrella production execution record

Ledger for [`PLAN_umbrella_production.md`](../../PLAN_umbrella_production.md),
with the acceptance rows in
[`TEST_MATRIX.md`](TEST_MATRIX.md). Baseline findings are kept separate from
post-fix results: nothing here is marked passed because a test file exists, and
a blocker found in a *different* task's area is recorded as a gap rather than
repaired while executing U00.

Status vocabulary is the plan's §8 set: `pending`, `reproducing`,
`implemented with gaps`, `passed within stated profile`,
`experimental/unsupported`.

## U00 — harness and honest baseline

### What U00 added

| Path | Purpose |
|---|---|
| `scripts/test-umbrella` | lane runner: selection, per-case timeouts, unique scratch dirs, per-case evidence, setup-vs-case timing, required-gap accounting, NDJSON mode |
| `umbrella/tests/support/harness.py` | identity record (lock hashes, features, toolchain, target, native linker), prerequisite probe with stage identity, child-process capture |
| `umbrella/tests/support/fake_tool.py` | stand-in tool that records verbatim argv + `UMBRELLA_TEST_*` environment and writes an invocation marker |
| `umbrella/tests/support/__init__.py` | package marker |

The runner is stdlib-only Python 3 (uses `tomllib` when available, degrades to
an identity record without the features table on older interpreters). AGENTS.md
notes `python3` is absent from a bare NixOS shell, so invoke it the way the other
scripts are invoked:

```bash
nix-shell -p python3 --run "python3 scripts/test-umbrella --lane cli --strict --json"
```

### Runner contract

```
scripts/test-umbrella (--lane LANE | --case ID | --all) [--strict] [--json]
                       [--seed N] [--timeout SECONDS] [--scratch-root DIR]
                       [--keep] [--list]
```

- **Selection is explicit and exact**: only the selected cases run, in registry
  order. `--all` cannot be combined with `--lane`/`--case`. A run with no
  selector, an unknown lane, an unknown case, or a selection that resolves to
  zero cases exits **2** — an empty selection is an error, never an empty PASS.
- **Lanes**: `cli`, `build`, `abi`, `capability`, `e2e`, `robustness`,
  `package`, `isolated`, `oracle`. Lanes with no cases yet select zero and
  therefore also exit 2, which is how a missing lane suite stays visible.
- **Fixtures** (`SELFTEST_*`) exist to test the harness and run only when named
  with `--case`; lane and `--all` runs never include them.
- **Required inventory** is pinned in `REQUIRED_CASES`: a pinned case that is
  missing, in the wrong lane, or no longer marked required is an inventory
  error (exit 2), not a silent skip.
- **Verdicts**: `pass` (the case's own oracle held), `fail` (it did not),
  `gap` (a prerequisite this environment lacks). A child exiting 0 is never a
  pass by itself; cases check the marker/oracle the child was meant to produce.
- **Exit**: 0 all selected cases passed (gaps tolerated without `--strict`);
  1 any case failed, or `--strict` saw a required gap; 2 usage/inventory error.
- **Evidence**: one directory per run (`--scratch-root`, default `$TMPDIR`),
  containing `identity.json`, `summary.json`, and `cases/<ID>/evidence.json`.
  Passing runs clean it unless `--keep` is given; failures and gaps are always
  retained and the path is printed. In `--json` mode stdout carries only NDJSON
  events (`setup`, `case`, `summary`) and human diagnostics go to stderr.
- **Timing**: `setup_ns` (identity capture, before any case) is reported
  separately from per-case `duration_ns` and from `total_ns`.

### Case inventory

| Lane | Case | Kind | Oracle |
|---|---|---|---|
| cli | HAR01 | required | exactly the requested case IDs run; zero/unknown selection exits 2; a failing child is reported failing and exits 1 |
| cli | HAR03 | required | an absent required tool is an explained gap (exit 0 non-strict); `--strict` makes a required gap exit 1 |
| build | BLD08 | required | every prerequisite stage is reported individually; in a controlled-empty environment all are missing and each detail names its own stage plus a remedy |
| cli | SELFTEST_PASS | fixture | fake tool observes verbatim argv and the harness environment, and writes its marker |
| cli | SELFTEST_FAIL | fixture | a child that prints PASS and exits 0 without writing the marker must be reported `fail` |
| cli | SELFTEST_GAP | fixture, required | a required tool is absent, so `--strict` must fail the lane |

### Environment identity (U00 baseline, measured 2026-11-17)

Captured by `harness.record_identity` in the `setup` event; this is the identity
the baseline results below belong to, not a claim about future runs.

| Item | Value |
|---|---|
| git rev | `7f094b5a2457146a113926515295f837abce2d46` (+ uncommitted U00 files) |
| umbrella | `umbrella 0.1.0`, manifest sha256 `86ff19e46d50f7e9ef3e3e745af3d6e29fa40f9bad717be2acb70b88a4fd55c0` (U02 added the `serde` derive; U00 identity was `fdb950bf…`) |
| umbrella → bridge dep | path dependency, default features, no extra features |
| bridge | `austral_cranelift_bridge 0.1.0`; features `arctic-auth`, `capstd`, `cedar`, `ecmascript`, `federation`, `sandbox`, `test-stubs`, `tidepool`, `tier1-rs`, `unfer-kernel`, `zenodo-store`; default `cedar`, `unfer-kernel`, `tier1-rs` |
| lock: bridge | `safestos/cranelift/Cargo.lock` sha256 `30997b79c88dd0b3e195914fffbb0c3e3ec23ca8debdafaebdc392d8735f5f19` (refreshed 2026-11-17; the U00 baseline ran against the stale `1dc20a6a…` lock, see below) |
| lock: umbrella | `umbrella/Cargo.lock` sha256 `01095f2a8e585d0211fc0ecbf4e7e215af4d4114b3472d8a02d1b6132d61eed5` (refreshed by resolution only for U02: a `serde` edge from umbrella plus the same stale `sha2` edges the bridge lock carried; no version changed) |
| lock: rscrate | `umbrella/modules/rscrate/Cargo.lock` sha256 `463162dfcffe747b3f3d92cab434748a144638e9d282d68dd72353c7158cfebf` |
| toolchain | cargo 1.99.0, rustc 1.99.0, python 3.12.13, host `dune` 3.22.0; the front-end was built through `nix develop "git+file://$PWD#ocaml" -c make bridge` per AGENTS.md |
| target | `x86_64-unknown-linux-gnu` |
| native linker | `/usr/bin/cc` → `x86_64-linux-gnu`; `GNU ld (GNU Binutils for Deepin) 2.41` |

### Existing-suite inventory (actual assertions)

These are what the named suites assert, so "the file exists" is never the
evidence. None of them was modified.

| Suite | Tests | What they assert |
|---|---|---|
| `umbrella/modules/rscrate` (`cargo test`) | 2 | `au_vector` exercises the crate's vector helper; `empty_is_zero` checks the empty-input checksum path. Passed 2/2. |
| `safestos/cranelift/tests/tier1_extern_rust.rs` | 6 | `rs_*` namespace registration; granted vs. denied manifest execution of the reference checksum/`rs_version`; a `uk_*` grant does **not** grant `rs_*`; no manifest denies; `modhost` authorizes `rs_*`. |
| `safestos/cranelift/tests/capstd_tests.rs` | 4 run | manifest parses the cap-std archetype; fs/net grants default empty; hotswap refuses fs-grant escalation; cap-std blocks traversal. The file defines five `#[test]`s, but `capstd_stub_returns_error` is `#[cfg(not(feature = "capstd"))]`, so the capstd-enabled run executes the other four. |
| `safestos/cranelift/tests/static_seam.rs` | 3 | static seam resolves, executes and is still gated; the C-ABI registrar reaches the registry; late registration is refused. |

### Baseline results

| Command | Exit | Result / gap |
|---|---|---|
| `python3 scripts/test-umbrella --lane cli --strict --json` | 0 | HAR01 pass (≈0.69 s), HAR03 pass (≈0.47 s); setup ≈0.19 s. Evidence cleaned on pass. |
| `python3 scripts/test-umbrella --lane build --strict --json` | 1 → 0 | At U00, BLD08 was a **gap**: `frontend` missing — no stock `austral` binary at `<repo>/austral`. After `make bridge` built it (below), `--case BLD08` passes (238 ms) and `--lane build --strict` exits 0. The controlled-empty control still reports all five stages missing, each naming its stage and remedy. |
| `cd umbrella/modules/rscrate && cargo test --locked --offline` | 0 | 2 passed, 0 failed. |
| `bash examples/modules/rust_kv/run_demo.sh` | 3 | Honest after U01: capability execution unimplemented, uncovered path named. Before U01 it printed `PASS`/`ALL TESTS PASSED` and exited 0 without performing any capability operation. |
| bridge `--test tier1_extern_rust --features test-stubs` (`--locked`) | 101 → 0 | At U00 it was blocked before compiling: the committed lock was stale against `Cargo.toml` (`sha2` promoted to a direct dependency; a dropped `unfer_protocol` entry). After the lock refresh below: 6 passed, 0 failed. |
| bridge `--test static_seam --features test-stubs` (`--locked --offline`) | 101 → 0 | Same stale-lock blocker at U00; after the refresh, 3 passed, 0 failed. |
| bridge `--test capstd_tests --features test-stubs,capstd` (`--locked --offline`) | 101 → 0 | Same stale-lock blocker at U00; after the refresh, 4 passed, 0 failed (see the `cfg(not(capstd))` note above). |
| `test-programs/runner.py 019-extern-rust` | 0 | After the front-end build: 6/6 programs pass (`002-missing-ownership` … `007-ownership-linear-shape`), `All tests passed`. It was recorded as a gap at U00 because the front-end and a fresh bridge were missing, not attempted-and-passed. |
| umbrella demo (`cargo run -- --manifest module.toml modules/Demo.aum`) | not run | Needs the stock `austral` front-end at `<repo>/austral`; not run so the baseline does not imply it was. |

The bridge release `.so` was stale (dated Sep 29, older than its sources) and
the front-end binary was absent, so U00 recorded both rather than rebuilding
them. The build task that needed them then ran
`nix develop "git+file://$PWD#ocaml" -c make bridge CARGO_FLAGS="--offline -j 2"`:
it rebuilt the release bridge (`libaustral_cranelift_bridge.so` 16:15:21), ran
`dune build lib/ bin/`, and copied `_build/default/bin/austral.exe` to
`./austral` (16:15:28, `austral 0.2.0`). `make bridge` was used rather than
`make all` because the `.so` was older than the bridge sources (AGENTS.md
`check-bridge-fresh`); the target rebuilds the bridge and then the front-end,
which is the same artifact `make all` would produce. The binary needs
`LD_LIBRARY_PATH=safestos/cranelift/target/release` (as the Makefile's own
targets set); the umbrella driver prepends it automatically when the layout is
in-tree.

The lockfile diagnosis was taken on a backup/restore basis: `cargo metadata
--offline` was allowed to rewrite a copy, the diff (`+sha2`, `-unfer_protocol`)
inspected, and the tracked lockfile restored byte-identically.

The lock was then refreshed for real, and only by resolution: `cargo metadata
--offline` rewrote it with exactly that two-line diff (a `sha2` edge added to
the bridge package, a stale `unfer_protocol` edge removed from `unfer_ffi`) and
no version changed. The local crate cache was missing crates the resolution
needs (`arrayvec 0.5.2`, via the `pretty` test dependency), so `cargo fetch`
populated the cache; after that `cargo metadata --locked --offline` exits 0, and
every suite above runs under `--locked --offline`. `Cargo.toml` was not
touched, no feature was weakened and no dependency was removed to make the
build pass.

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U00 harness + baseline | HAR01, HAR03 | implemented with gaps | git `7f094b5a2457`; python 3.12.13; stdlib-only runner; bridge `0.1.0` identity recorded but not rebuilt | `python3 scripts/test-umbrella --lane cli --strict --json` — exits 0; harness self-cases HAR01/HAR03 pass, evidence cleaned on pass | exactly-selected execution, empty/unknown selection exit 2, failing child → fail/exit 1 (HAR01); explained gap + strict exit 1 (HAR03) | evidence for passing runs is cleaned by default (`--keep` retains it); `--seed` is recorded but no randomized semantics yet (U24); lanes that currently have no cases (abi, robustness, package, isolated, oracle) exit 2; cli carries HAR01/HAR03, build carries BLD08 + the SUITE_* cases, capability carries HAR02 |
| U00 prerequisite reporting | BLD08 | implemented with gaps | x86_64-unknown-linux-gnu; cc/ld present; cargo/rustc 1.99.0; bridge `.so` present; front-end absent | `python3 scripts/test-umbrella --lane build --strict --json` — exits 1 with a `frontend` gap; `prerequisites.json` names every stage | five stages reported once each; controlled-empty env reports all missing, each detail naming its stage and remedy (BLD08); real env gap names `frontend` | version *mismatch* detection (not just absence) is U25; the front-end still has to be built for BLD08 to pass |
| Baseline: existing suites | SUITE_TIER1, SUITE_CAPSTD, SUITE_STATIC_SEAM, SUITE_RSCRATE (lane `build`) | implemented git `7f094b5a2457`; cargo/rustc 1.99.0; refreshed bridge lock `30997b79…`; release profile reused | `python3 scripts/test-umbrella --case SUITE_TIER1 --case SUITE_CAPSTD --case SUITE_STATIC_SEAM --case SUITE_RSCRATE --strict --json` — exits 0, 4 selected / 4 pass / 0 fail / 0 gap (CAPSTD 497 s, STATIC_SEAM 510 s: those feature sets relink the bridge; RSCRATE 0.2 s, TIER1 149 s); each case runs `cargo test --locked --offline --release` on its own manifest/features | each case requires exit 0, at least one `test result: ok.` line, zero failed, and the baseline's pinned passing count (6/4/3/2); a suite that loses a test or reports a failure is a red case, not a smaller pass | the cases reuse the release profile (the plan's commands differ only there); the first run after a bridge edit relinks, so cases carry a 30-minute timeout and cargo is bounded to 2 jobs (`UMBRELLA_CARGO_JOBS`) — unbounded linking OOM-killed this 15 GB host; each passing case now reports build setup and test runtime separately (`build 0.0 s, tests 0.00s, 2 passed`), parsed from cargo's `Finished \`release\` profile … in <t>s` and the suite's `test result: … finished in <t>s` lines; the front-end is built, so BLD08 and `019-extern-rust` are no longer gaps |

Both U00 blockers are now cleared. The bridge lock is refreshed (digest above)
and all four baseline suites run and pass under `--locked --offline` as the
`SUITE_*` lane cases. The stock front-end is built (above), so BLD08 passes and
`test-programs/runner.py 019-extern-rust` runs and passes 6/6. The umbrella
demo itself (`cargo run -- --manifest …`) was still not run: it links the
default-feature bridge and is not needed to establish BLD08 or 019.

The `SUITE_*` cases keep the baseline honest in a second way: each pins the
passing count it measured (6 / 4 / 3 / 2), so a suite that silently loses a test
fails the lane instead of passing smaller. Measured negative control: changing
`SUITE_RSCRATE`'s pin to 3 makes that case `fail` and the run exit 1
(`ran 2 tests, pinned 3`), and the runner was restored byte-identically
(sha256 re-checked).

## U01 — remove false capability demo success

### What changed

| Path | Change |
|---|---|
| `examples/modules/rust_kv/run_demo.sh` | no longer prints `PASS`/`ALL TESTS PASSED`; checks the manifest declarations it actually inspects, reports the capability stage as `UNSUPPORTED`, names the uncovered path, and exits 3. The optional `modhost` build was removed rather than kept as a stand-in for a capability operation. |
| `examples/modules/rust_kv/fixtures/printed_success.txt` | the previous transcript, kept as the fixture the guard must reject |
| `examples/modules/rust_kv/README.md` | what the demo reports, which existing unit tests it merely references, the guard, and what U22 replaces it with |
| `scripts/test-umbrella` | `HAR02` case, lane `capability`, pinned in `REQUIRED_CASES` |

`HAR02` has three parts, and each can fail independently: the real demo must
exit 3 with no success claim and must name the uncovered path (with the unit
tests marked as unexecuted references); the printed-success fixture must be
rejected by the same textual oracle; and a mutated copy of the script whose
`uncovered_capability_execution` function is replaced by
`echo "PASS: capability execution"` must be caught. If the mutation applies but
produces no success claim the case fails, so the guard cannot pass vacuously;
if the mutation does not apply at all, the case fails too.

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U01 demo honesty | HAR02 | implemented with gaps | git `7f094b5a2457`; `bash` 5.x; no Rust implementation in `examples/modules/rust_kv`, so no new dependency or feature was needed | `bash examples/modules/rust_kv/run_demo.sh` — exits 3, prints `UNSUPPORTED: capability execution is not implemented` and the uncovered path; `python3 scripts/test-umbrella --lane capability --strict --json` — exits 0, HAR02 passes in ≈16 ms | demo exit is 3 with zero `PASS`/`ALL TESTS PASSED` lines (HAR02); the printed-success fixture and a temporary `echo PASS` regression both make the case fail/exit 1 (measured, script restored byte-identically by sha256) | the capability path is still unimplemented: the demo has no success path until U22 supplies the real pipeline; `capstd_tests.rs` still needs its feature enabled and stronger isolation (U19–U22) |

## U02 — strict profiles and policy preflight

### What changed

| Path | Change |
|---|---|
| `umbrella/src/project.rs` | **new**: versioned (`schema: 1`) project document — profile, module, build permission, load permission, call grants. Every struct is `deny_unknown_fields`, so a misspelled `allowedd` is a named diagnostic rather than a silent default. Validation names unsupported schema versions, an empty module, `build.allowed` without a crate, `load.allowed` without a provider, and a grant whose symbol is not `rs_`/`uk_`/`uz_`. |
| `umbrella/src/policy.rs` | **new**: three separate decisions (build / load / call) computed before any stage, a `report()` that prints each state with its reason plus the scope sentence, `run_stages` which invokes only authorized stages in order (and never calls after a failed or denied earlier stage), and `dlopen` (moved out of `main.rs` so the test maps the fixture through the same code). |
| `umbrella/src/lib.rs` | **new**: exposes `project` and `policy` so the tests do not need the whole pipeline (and therefore the front-end). |
| `umbrella/src/main.rs` | `--project <json>`: load, validate and preflight before cargo, dlopen or the front-end. Build-denied projects use a declared prebuilt provider instead of compiling; load-denied projects never dlopen; a call-denied project still runs the authorized build/load, prints the scope line, and exits 2 before invoking the entry. Modules may come from the project when no positional source is given. |
| `umbrella/Cargo.toml` / `Cargo.lock` | `serde` derive added (already in the lock through the bridge); the lock changed by resolution only, see the identity table. |
| `umbrella/tests/policy.rs`, `umbrella/tests/fixtures/stageprobe/` | the U02 oracles below, with a real build script and a real `.init_array` library initializer writing one line each to `stages.log`; the fixture's target dir lives in the test's scratch dir. |

The three permissions are deliberately not derived from one another. A project
may authorize a build without authorizing a call (BLD06's "call grants do not
claim containment"): the run then builds, loads and finally reports
`call=denied … a denied call does not mean the build or library initializer was
contained`. A project authorizing neither build nor load is refused in
`preflight`, before a `Preflight` value exists for `run_stages` to act on
(AUT01). `untrusted-native` is recognized by name and refused as U26's work, so
the default untrusted profile cannot be selected by accident.

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U02 schema + preflight | AUT01, BLD06 | implemented | git `7f094b5a2457`; cargo/rustc 1.99.0; umbrella manifest `86ff19e4…`, lock `01095f2a…`; bridge lock `30997b79…` | `cd umbrella && cargo test --offline --lib --test policy` — 5 schema tests + 6 policy tests, 0 failed; `cargo check --offline --bin umbrella` exits 0 (the driver wiring compiles) | an unknown field is named with its section (`build.allowedd`); schema 2, a malformed grant and a missing `build.crate` are each refused by name; a build/load-denied project is refused at preflight and writes no marker; a call-denied run leaves `build`+`load` markers and the report says those stages ran | the project's call grants are preflight-declared only; the authoritative UK-4001 gate is still the bridge manifest (`--manifest`), and wiring project grants into the Cedar engine is U09/U18's seam. Feature *availability* is checked for the profiles this build implements; comparing grants against the bridge's compiled symbol table is U25. There is no `scripts/test-umbrella` case for this suite yet — the cargo integration test is the oracle, and the runner's lane inventory stays U00's |
| U02 stage ordering | BLD06, RUN06 | implemented | same; fixture `stageprobe` builds with the host cargo offline, target dir in scratch | `cargo test --offline --test policy` — `authorized_stages_run_in_order_and_leave_markers` maps the cdylib with `policy::dlopen` and reads `stages.log` = `build\nload\ncall\n` (the first `build` is the test's own pre-build, needed to have a cdylib for the load closure) | the build script's marker and the library initializer's marker are separate lines, and a denied call still leaves the two earlier lines; a denied project leaves an empty log | RUN06's *isolated worker* and its deadline are U23's: the process here is the umbrella itself, so a blocking initializer is bounded only by the caller's timeout. The fixture proves ordering and authorization, not worker isolation |

## U03 — CLI grammar, diagnostics and exit contracts

### What changed

| Path | Change |
|---|---|
| `umbrella/src/cli.rs` | **new**: exact option matching over `OsString` args. `--name`/`--name=value` only (no prefix match), `--` ends options, a value is never taken from a token in option position, `--expect -7` is the one documented negative value, paths stay `PathBuf` with no UTF-8 round trip, repeating a single-valued option is an error, `--help`/`--version` win anywhere before `--`. `USAGE` is one constant. |
| `umbrella/src/main.rs` | parsing replaced by `cli::parse` + `with_defaults` (checkout-relative front-end/crate defaults). Usage errors print the diagnostic and usage to stderr and exit **2** before any stage; operational failures exit **1**. `--json` emits one typed event per stdout line (`{"event":"done","result":n}` / `{"event":"error","message":…}`) with all human text on stderr; human mode still prints `RESULT <n>`. |
| `umbrella/tests/cli.rs` | **new**: real-binary tests; a fake `cargo` and a recording `--austral` prove malformed invocations spawn nothing. |

The measured UR12 defects map one-to-one: `--help`/`--version` now exit 0; `--manifest-garbage` is an unknown option, not `--manifest`; `--entry --expect 7` is a missing value for `--entry`; `--entry=` is refused; `-x` is not a module path (use `--`). A new rule makes `--project` and `--manifest` mutually exclusive, since they are different policy sources; the legacy `--manifest module.toml --entry compute modules/Demo.aum` form still parses and is covered by `the_legacy_demo_invocation_is_not_a_usage_error`.

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U03 grammar + exits | CLI01, CLI02, CLI03, CLI05 | implemented | git `7f094b5a2457`; cargo/rustc 1.99.0; umbrella manifest `86ff19e4…` (unchanged by U03) | `cd umbrella && cargo test --offline --lib --test cli` — 12 cli parser tests + 6 binary tests, 0 failed | help/version exit 0 with no stage marker; unknown/prefix option, missing/empty value, malformed i64, duplicate option and `-x` exit 2 with the flag named and no `cargo` marker; `--k=v` equals separated form; legacy demo parses and its later failure is exit 1 | exit-2 coverage is parser-level plus binary-level for the measured defects; there is no `scripts/test-umbrella` case for this suite yet. The `-` (single dash) token is treated as a path, not stdin |
| U03 OS paths + JSON | CLI04, CLI06 | implemented | same; fixture `stageprobe` cdylib for the load-only project | `cargo test --offline --test cli` — `a_non_utf8_path_reaches_the_front_end_unchanged` reads the recording front-end's NUL-separated argv and finds the exact non-UTF-8 bytes; `json_mode_prints_one_typed_event_and_diagnostics_on_stderr` gets exactly one `error` event with a non-empty stderr | a path that is not valid UTF-8 survives parse and reaches the tool byte-for-byte; JSON stdout carries no `RESULT` and no human text, and a usage error writes no protocol at all | JSON mode currently covers the run's start and end events (done/error), not a per-stage stream; U15 owns runtime-refusal event detail. Paths in *diagnostics* are printed lossily (`display()`), which is a message concern, not an argument one |

## U04 — strict entry admission (bounded patch)

This patch implements U04 in two halves: the compiler rejects interop
signatures the baseline transport cannot carry, and an entry may only be
invoked when the bridge can call it correctly.

### What changed

| Path | Change |
|---|---|
| `safestos/cranelift/src/cps.rs` | `CpsModule` keeps each function's **declared** signature (parameter count and return-type byte) from the CPS header, which used to be read and discarded. The CLIF signature is always `(i64*) -> i64`, so this is the only place the declared types survive. |
| `safestos/cranelift/src/lib.rs` | **new** `compile_entry_i64(ir, len, name, name_len, expected_params)`: refuses a non-64-bit host (the documented transport is host-native 64-bit words in, one i64 out), an empty name (no `run`/`main`/first-function fallback), a missing export, a declared parameter count different from the caller's, and any return type other than `Int64` — naming the type found. Every refusal returns null with the reason on the last-error channel, so the caller never receives a pointer to miscall. The legacy named/unnamed APIs are unchanged. |
| `umbrella/src/main.rs` | the run's compile step uses `compile_entry_i64(..., 0)`: `execute_function` invokes the pointer as `fn() -> i64`, so the caller's contract is zero parameters and an Int64 result, and a module declaring anything else is refused instead of run. |
| `lib/TypingPass.ml` | the strict extern-rust interop (every `rs_*` import, which already requires `Rust_Ownership`) now also enforces the baseline transport: parameters must be `Int64` or a byte-buffer address/pointer, and the result must be `Int64`. `Int32`/`Bool`/`Float64`/`SingleFloat` pass the older buffer-protocol shape check but are refused here — a float travels in a vector register, and a narrower integer arrives as a truncated word. Aggregates keep their existing `rust_ownership_shape` message. |
| `lib/TypeErrors.ml` | new `rust_interop_transport` declaration error naming the type and why the word transport cannot carry it. |
| `test-programs/suites/019-extern-rust/008-transport-narrow-parameter` | `rs_narrow(width: Int32): Int64` — compile fails with the transport error (golden `austral-stderr.txt`). |
| `test-programs/suites/019-extern-rust/009-transport-float-result` | `rs_gain(scale: Int64): Float64` — compile fails with the transport error (golden `austral-stderr.txt`). |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U04 strict entry (this patch) | ABI01, ABI02, BLD05 | implemented (bridge + umbrella) | git `7f094b5a2457`; cargo/rustc 1.99.0; bridge crate `safestos/cranelift`; umbrella manifest `86ff19e4…` | `cd safestos/cranelift && cargo test --release --locked --offline -j 2` — `strict_entry_*` (7 tests: 10 i64 boundary values, matching signature, empty name, missing name, wrong arity + legacy control, five non-i64 return bytes, caller-arg mismatch); `cd umbrella && cargo test --offline --test entry` — 4 tests with a scripted front-end emitting hand-built CPS | boundary values round-trip exactly; a 1-parameter or non-i64-returning `compute` yields a null pointer, the diagnostic names the declared parameter count/return type, and stdout has no `RESULT` (the invocation marker); the same 1-parameter buffer compiles through the legacy named path, so the refusal is the signature gate, not the fixture; umbrella's live demo still runs | `compile_entry_i64` admits only `Int64` returns (Unit/Bool/I32/String/F64 are refused by name); the compiler-side strict type rejection (`lib/TypingPass.ml`, U04 step 1) is **not** implemented yet, so a module can still *emit* a non-writable parameter type — the bridge refuses to *invoke* it, which is the half that matters here. Parameter types are not in the CPS header (only names), so the bridge cannot check them; the compiler half enforces the baseline on the declarations instead (row below). The 32-bit-host refusal is not exercised on this host (aarch64/x86_64) |
| U04 strict interop, compiler side | ABI01, ABI02, UR02 | implemented | git `7f094b5a2457`; dune 3.23.1 / OCaml 5.4.1 via `nix develop "git+file://$PWD#ocaml"`; front-end `./austral` rebuilt by `make all` | `nix develop … -c python3 test-programs/runner.py 019` → 8/8 PASS (six pre-existing ownership fixtures plus 008 and 009); positive control: a module declaring `rs_checksum(buf: Address[Nat8], len: Int64): Int64` compiles with exit 0; `make test` → 19 suites OK | a narrower-than-64-bit parameter and a float result are refused at typechecking with the type named, before any CPS or invocation; a byte-buffer address plus i64 length (the kernel/checksum shape) still compiles, so the check is not "reject everything" | the check covers the strict `rs_*` interop tier only: legacy C imports (`putchar(Int32)`, `Span[Nat8]` for `puts`, libm's `Float64` in `examples/haversine`) keep their documented behaviour, because rejecting those today would break existing examples rather than fix a strict profile. `examples/haversine` is still an unchecked float-register call and is the fixture U11 must replace. Parameter types are erased before CPS, so this is a compile-time refusal only — the bridge cannot re-check them later. |

## U05 — exact Cargo artifact selection (bounded patch)

This patch completes U05: the build stream is read as a set of identities, the
artifact umbrella loads must be the one the crate directory declares, a fixture
workspace pins the real stream shapes that make that necessary, and everything
that varies the build is stated on the command line and recorded.

### What changed

| Path | Change |
|---|---|
| `umbrella/src/cargo.rs` | **new.** `wanted_from_metadata` reads `cargo metadata --no-deps` and requires exactly one package at the crate directory with exactly one `cdylib` target — otherwise it refuses, naming what it found. `select_shared_artifact` scans the build stream, matches only `compiler-artifact` payloads whose `package_id` names that package and whose `target.name` is that target, and requires exactly one shared-library filename. Zero matches, several matches, a match with no shared filename, and an unparseable stream line are all refusals that name the evidence; a dependency's cdylib or a proc-macro `.so` can no longer win by appearing first. |
| `umbrella/src/main.rs` | `cargo_build` asks for the identity first (`cargo metadata`, logged as ``cargo will build package `…`, target `…`, profile `…` ``), builds with the explicit argv, then selects with that identity. It also logs each applicable `.cargo/config{,.toml}` and every set build-affecting variable, and refuses `RUSTC_WRAPPER`/`RUSTC_WORKSPACE_WRAPPER`. |
| `umbrella/src/cargo.rs` (bullet 2) | `BuildRequest` (crate dir, package, target dir, `Profile::{Dev,Release}`, features, locked, offline) and `build_args`, which keeps every value one `OsString` (a feature list containing a space stays one argument; no shell). `build_environment` records `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_TARGET`, `CARGO_BUILD_RUSTFLAGS`, `RUSTC` and refuses wrappers; `cargo_config_files` lists merged configs from the crate dir upwards. `wanted_from_metadata_for` selects a workspace member by `--cargo-package`. |
| `umbrella/src/cli.rs` | new options with the U03 grammar: `--cargo-package <name>`, `--target-dir <dir>`, `--profile dev|release` (default `release`, the old behaviour), repeatable `--features <list>`, and the flags `--locked` / `--offline`; duplicates, empty values, unknown profiles and `--flag=value` on a flag are usage errors (exit 2). |
| `umbrella/src/lib.rs` | `pub mod cargo;` so the selector is unit-testable without spawning cargo. |
| `umbrella/tests/fixtures/cargo-workspace/` | **new** fixture workspace: `fixture-macros` (proc-macro), `fixture-shared` (transitive lib), `fixture-alpha` and `fixture-beta` (cdylibs depending on the lib, alpha also on the macro). No external dependencies, so `cargo build --offline` works. |
| `umbrella/tests/cargo_select.rs` | **new** 4 tests over the *real* stream: preconditions (proc-macro `.so`, transitive `.rlib`, both cdylibs present), per-identity selection into a custom `--target-dir`, a mixed package-id/target pair matching nothing, and the transitive library refused as not shared. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U05 exact artifact | BLD01, BLD02, BLD03 | implemented (bullets 1–3) | git `7f094b5a2457`; cargo 1.99.0; `umbrella/Cargo.toml` `86ff19e4…` | `cd umbrella && cargo test --offline --lib` → 27 passed, 9 of them `cargo::tests`; live path: `./target/debug/umbrella --manifest module.toml modules/Demo.aum` prints `cargo will build package `rscrate`, target `rscrate``, builds, loads and prints `RESULT -1781872834555808882` (unchanged from before this patch) | a stream containing a proc-macro `.so`, a dependency cdylib and the wanted cdylib selects the wanted one; absent target, two libraries for one target, a non-shared artifact and a malformed line each refuse with the evidence named; `cargo metadata` identity refuses a package with no cdylib or two cdylibs; **measured**: this cargo's path-dependency id is `path+file:///…/rscrate#0.1.0` with **no package name in it**, so matching extracts nothing from the id — the id from `cargo metadata` is compared verbatim, and a fixture pins that name-less shape | cargo config files and `RUSTFLAGS`/`CARGO_BUILD_TARGET`/`RUSTC` are **recorded, not restricted**: the log names every applicable `.cargo/config{,.toml}` and every set build-affecting variable, but only compiler wrappers are refused. A config that changes the linker or target still influences the build. `modhost`/manifest refusal of a foreign-native artifact is U10's descriptor work, not this selector. |
| U05 fixture workspace | BLD01, BLD03, PKG07 | implemented | git `7f094b5a2457`; cargo 1.99.0; `umbrella/tests/fixtures/cargo-workspace` (workspace: `fixture-macros` proc-macro, `fixture-shared` lib, `fixture-alpha`/`fixture-beta` cdylibs, no external deps) | `cd umbrella && cargo test --offline --test cargo_select` → 4 passed; harness: `scripts/test-umbrella --case SUITE_UMBRELLA_CARGO` → 1 pass, exit 0 (pinned at 4 tests) | preconditions assert the real stream carries a proc-macro `.so`, a transitive `.rlib` with no shared artifact, and both `libfixture_{alpha,beta}.so` — otherwise the selection tests would pass for the wrong reason; each cdylib is selected from its own package id into the custom `--target-dir`, a package id mixed with the other package's target matches nothing, and the transitive library is refused as “no shared-library filename” | the fixture runs `cargo build --offline` into a scratch target dir on every test run, so it is a build-shape pin, not an ABI check; it does not try to `dlopen` the fixture cdylibs or execute them |
| U05 explicit build inputs | BLD01, BLD02, BLD04, BLD09 | implemented | git `7f094b5a2457`; cargo 1.99.0 | `cd umbrella && cargo test --offline` — lib 34 (7 new: argv shape, feature boundaries, explicit fields, wrapper refusal + recording, package-by-name identity; plus 4 new CLI grammar tests); `cargo test --test cargo_select` — `explicit_package_profile_and_policy_reach_the_real_build` builds the fixture workspace with `--package fixture-beta --target-dir <scratch> --locked --offline` and dev profile | the real build's stream contains `fixture_beta` and **not** `fixture_alpha` (so `--package` really scopes it), the selected artifact is `<scratch>/debug/libfixture_beta.so` (so profile and target dir really reached cargo), and the identity came from `--cargo-package`; wrapper refusal is unit-tested for both wrapper variables with the variable and value named | `RUSTFLAGS`, `CARGO_BUILD_TARGET`, `RUSTC` and cargo config files are recorded in the log but not refused, so a config that rewrites the linker/target can still change the artifact. `--profile` maps only `dev`/`release` (custom profile names are not accepted). Feature *declarations* are passed but not cross-checked against the built features (bullet 2's "requested features match evidence" is not measured yet). |

## Harness pins for the U03/U04 suites

`scripts/test-umbrella` now wires the two umbrella test suites into the `build`
lane as required cases, so neither can silently shrink:

| Case | Suite | Pinned `test result` total |
|---|---|---|
| `SUITE_UMBRELLA_CLI` | `cargo test --test cli` (U03 grammar, exit contracts, JSON, non-UTF-8 path) | 6 |
| `SUITE_UMBRELLA_ENTRY` | `cargo test --test entry` (U04 strict entry from the umbrella side) | 4 |
| `SUITE_UMBRELLA_CARGO` | `cargo test --test cargo_select` (U05 selection and explicit build options over a real fixture-workspace stream) | 5 |
| `SUITE_ADAPTERS` | `cargo test --lib adapters --release` (U11 scalar word encoding) | 9 |
| `SUITE_SCALAR_ABI` | `cargo test --test scalar_abi --release` (U11 scalar encoding across the C ABI) | 6 |
| `SUITE_UMBRELLA_INTERFACE` | `cargo test --test interface` (U10 descriptor + generated wrapper typecheck) | 5 |
| `SUITE_PROCESS` | `cargo test --lib process::tests` (U06 bounded child runner) | 6 |

`SUITES` entries now carry a fourth field saying whether the run uses
`--release`. The bridge suites keep `--release` (they reuse existing release
artifacts); the umbrella suites run the debug profile, because their assertions
are exit codes, messages and the absence of the `RESULT` marker — nothing they
check depends on optimization, and a release build of those test binaries is a
second full artifact set. The pinned totals are the counts `cargo test` prints,
so a new test that is not recorded here fails `SUITE_UMBRELLA_CLI`/`_ENTRY` with
the measured and pinned numbers named.

Evidence: `nix develop "git+file://$PWD#ocaml" -c python3 scripts/test-umbrella
--case SUITE_UMBRELLA_CLI --case SUITE_UMBRELLA_ENTRY` → `2 selected, 2 pass,
0 fail, 0 gap`, exit 0; `--case SUITE_UMBRELLA_CARGO` → `1 pass`, exit 0 (first run also builds the umbrella debug test binaries;
a dependency's `mathhook-core` build script takes ~7 min of CPU once).

## U10 — versioned adapter descriptor and generated wrapper (bounded start)

U10 depends on U07/U08, which do not exist, so this patch is the part that
stands on its own: the descriptor schema, its canonical text form, and a
wrapper generator whose output is actually compiled. Admission side-effects
(provider digests, modhost checks) are not in this slice.

### What changed

| Path | Change |
|---|---|
| `umbrella/src/interface.rs` | **new.** `DESCRIPTOR_SCHEMA = 1`, `ADAPTER_ABI_VERSION = 1`, `Scalar` (the eight scalar classes), `Adapter { schema, abi_version, symbol, target, params, returns }`, `validate`, canonical `descriptor_text`/`parse_descriptor`, and `render_wrapper`, which emits a `#[no_mangle] extern "C" fn <symbol>(word0: u64, …) -> u64` that decodes each argument with the bridge adapters, calls the native function with its real typed signature, and encodes the result. Unsupported types (`Span`, records, references) are refused with guidance; `Unit` in either position is refused because it has no word. |
| `umbrella/src/lib.rs` | `pub mod interface;` |
| `umbrella/tests/fixtures/adapters/rs_scale.{descriptor,wrapper.rs}` | golden descriptor and wrapper, regenerable with `UPDATE_GOLDEN=1`. |
| `umbrella/tests/interface.rs` | **new** 5 tests: the descriptor's encoding version equals the bridge's, the golden descriptor roundtrips byte-identically, the generated wrapper **typechecks with `rustc` against the bridge rlib**, a wrapper whose call drops an argument **must fail** to compile (the control), and a mismatched encoding version cannot render. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U10 descriptor + generator | ABI03, API03 | started (schema + generator + typecheck) | git `7f094b5a2457`; rustc 1.99.0; umbrella `86ff19e4…` | `cd umbrella && cargo test --offline --test interface` → 5 passed (the typecheck runs `rustc --emit=metadata` against the bridge rlib); `cargo test --offline --lib interface` → 7 passed; harness case `SUITE_UMBRELLA_INTERFACE` pins the 5 | the descriptor text is canonical and roundtrips byte-identically; the generated wrapper is compiled by rustc, not merely compared as text; the deliberately broken wrapper (one argument instead of two) is rejected with E0061, so the typecheck can fail; an `abi_version` of 99 is refused with the version named; `Span` is refused with "only scalars cross a descriptor … need a buffer or a versioned layout" | no compiler emission (CPS/`--emit-cps` does not carry a descriptor), no provider digest, no `modhost` admission check, no generation of Austral declarations, and no runtime wiring: the wrapper exists as generated source plus goldens. Lifetimes, variadics, aggregates and non-scalar returns are refused rather than supported. The typecheck depends on `umbrella/target/debug` artifacts, so it is a build-shape check in the dev profile only. |

## U11 — versioned scalar adapters (bounded start)

U11 depends on U10, which does not exist yet, so this patch is the part that
can stand alone: the **definition** of how Int32, Bool and Float64 become
words and back, versioned and tested against independently compiled Rust
calls. The compiler still refuses those signatures, because the native call
they would produce is exactly the guess U04 banned.

### What changed

| Path | Change |
|---|---|
| `safestos/cranelift/src/adapters.rs` | **new.** `SCALAR_ABI_VERSION = 1`, `ScalarType::{Unit,Bool,I32,U32,I64,U64,F32,F64}`, `ScalarValue`, `encode`/`decode`, checked `narrow_i64`, and `AdapterError` that names the type and the offending word. Integers are extended on the way out (sign for I32, zero for U32) and **checked** on the way in; `Bool` is a tag and only 0/1 are valid; floats are **bit-preserving** (`to_bits`/`from_bits`, never an `as i64` cast) so `-0.0`, infinities and NaN payloads survive; `Unit` has no word at all. |
| `safestos/cranelift/src/adapters.rs` (C ABI) | `au_scalar_abi_version`, `au_scalar_encode`, `au_scalar_decode`, `au_scalar_type_tag` — the encoding across the bridge's C surface: numeric type tags 0–7, bit-pattern payloads, refusals via the last-error channel that never write their output pointer. Generated wrappers may call these instead of linking Rust enums. |
| `safestos/cranelift/src/lib.rs` | `pub mod adapters;` |
| `safestos/cranelift/tests/scalar_abi.rs` | **new** 6 tests of the exported surface: version, tag roundtrip, integer extension rules, bool tags, float bit patterns and garbage refusal, Unit and null-output refusal. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U11 scalar codec | ABI04, ABI05 | implemented (definition + tests only) | git `7f094b5a2457`; rustc/cargo 1.99.0; bridge crate `safestos/cranelift` | `cd safestos/cranelift && cargo test --offline --lib adapters` → 9 passed; `cargo test --offline --test scalar_abi` → 6 passed; harness cases `SUITE_ADAPTERS` (9) and `SUITE_SCALAR_ABI` (6) pin both | each scalar roundtrips through `encode`/`decode` **and** agrees with a direct call to a rustc-compiled `extern "C"` function (`i32*3`, `!bool`, `f64/2`, `mixed(i32,f64)->f64`); `-0.0` encodes as `0x8000…`, a NaN payload survives bit-for-bit and is explicitly asserted **not** equal to `x as i64`; bool tag 2, Int32 with garbage in the upper half, Nat32 with a bit above 31, Float32 with high bits and Unit-with-a-word are all refused with the type named; `narrow_i64` refuses out-of-range values instead of truncating | **not wired to any call path**: `lib/TypingPass.ml` still rejects Int32/Bool/Float64 at `rs_*` imports, and the bridge's entry path still admits only Int64, because a direct native call to a float-register ABI is the bug. Re-admission needs U10's versioned descriptor plus a generated wrapper that uses these adapters (U11 step 1's "generated adapter" half). No CPS opcode, `Cps_codec`, `ISA_REGISTRY` or corpus change is involved, so U11 step 3 is vacuously satisfied here. F32 arithmetic is only tested through the bit roundtrip, not a native `f32` call. The adapter is exposed across the C ABI (`au_scalar_*`) but the compiler and entry admission still do not use it. |

## U06 — bounded child processes (bounded start)

This patch is bullet 2's execution half and the deadline the plan asks for; the
private staging directory (bullet 1) is not in this slice.

### What changed

| Path | Change |
|---|---|
| `umbrella/src/process.rs` | **new.** `run(Command, Duration) -> Outcome { exit, stdout, stderr, timed_out }`: the child is spawned in its own process group (`process_group(0)`), stdout and stderr are drained by two reader threads so a noisy child cannot deadlock a pipe, `try_wait` is polled against the deadline, and on expiry the **group** is killed (`kill(-pgid, SIGKILL)`) and the leader reaped. `Exit::Code` and `Exit::Signal` are distinct outcomes; spawn and I/O failures are separate `ChildError` variants. |
| `umbrella/src/main.rs` | the front-end call goes through `process::run` with `FRONT_END_DEADLINE` (300 s); a timeout reports "exceeded its deadline and was killed with its process group" and the captured streams, and a non-zero/signal exit is reported with its `Exit` value. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U06 bounded child | RUN01, RUN03, BLD07 | started (deadline + group + reaping) | git `d2225612`; umbrella `86ff19e4…` | `cd umbrella && cargo test --offline --lib process` → 6 passed; harness case `SUITE_PROCESS` pins those 6; live demo still prints `RESULT -1781872834555808882` | a child writing 1 MiB to each stream is captured in full (no deadlock, nothing truncated); a hanging child is killed at the deadline with `Signal(SIGKILL)` and the call returns in well under 10 s; a helper that spawns a `sleep 60` grandchild leaves **no live grandchild** after the group kill (checked via `/proc/<pid>/stat`: gone or a zombie); exit 3 and `SIGABRT` are different outcomes; a missing program is `ChildError::Spawn` not a fake exit | no private per-build staging directory yet (bullet 1), no deadline on `cargo build`/`cargo metadata` (a build is legitimately slow), no environment allowlist policy in this module, and no readiness-event protocol — the tests use a bounded wait on `/proc`, not sleeps. `verify-invariants` still shows the W9.3 Lean/MM0 failures noted earlier, untouched here. |

## W9.3 gate run in the lean shell — blocked, results recorded

Attempted exactly as documented:

```bash
nix develop "git+file://$PWD#lean" -c python3 --version && \
nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants
```

**The shell does not build on this host (2026-11-17).** Nix fails while
realising `lean4export-4.30.0`'s pinned source:

```
error: hash mismatch in fixed-output derivation '…-source.drv':
         specified: sha256-LysjGOikBaaOdZdwxA+RwzfnQweTaU3kpFuWc6LxmXc=
            got:    sha256-kg0XW1HTJplBHznI4FM9KxSZIuQ/5P0IILp3FjDRXJA=
error: Cannot build '…-lean4export-4.30.0.drv'  (1 dependency failed)
error: Cannot build '…-nix-shell-env.drv'
```

So no shell command runs: there is no `lean` 4.30.0, no `lean4export`, and no
`mm0` from the flake. Fixing it means re-pinning the exporter's source hash in
`flake.nix` — an owner decision about the proof toolchain, not a test fix, so it
is recorded here instead of changed.

**Bare-shell fallback (what this host can measure).** `lean` resolves to host
elan **4.33.1**, `lean4export` is absent, `mm0-c`/`mm0-rs` are absent.
`bash scripts/verify-invariants` gives **112 passed, 17 explained gaps, 8
failed**. Nineteen W9.3 checks pass, including the toy model's counts (28
theorems / 20 axioms), the citation digests, the witness agreement and the
mutation controls. The 8 failures are:

| W9.3 check | Why it fails here |
|---|---|
| load/store are refuted as the model's `execute_LOAD`/`execute_STORE` | needs the pinned `$RISCV_MEMORY_MODEL` (flake lean shell, Sail build) |
| the memory projection is stated as vocabulary, accepted by `mm0-c` | `mm0-c` absent |
| the toy projection's theorems check without `sorry` | `mm0-rs` absent |
| the toy's proofs depend on its axioms (mutating one refuses them) | `mm0-rs` absent |
| a Lean development proves the same axioms | `ToyWitness.lean:628:70: error: failed to synthesize` — elaboration under host Lean **4.33.1**, not the pinned 4.30.0 |
| every Lean declaration rests on Lean's standard foundation only | axiom audit reports `[(0 (1 0) 1) ]` outside `{propext, Classical.choice, Quot.sound}` under 4.33.1 |
| the no-sorry scan refuses an injected `sorry` that Lean itself accepts | depends on the Lean run above |
| the no-sorry scan does not count banned words inside comments | depends on the Lean run above |

**Conclusion recorded:** the W9.3 proof-of-record results are **not** produced by
this host. The 8 failures are consistent with the missing pinned toolchain
(4.30.0 + `lean4export`, `mm0-c`, `mm0-rs`, Sail model) rather than with the
artifacts, and the earlier W1/W9 failures were separately traced to a stale
root `libaustral_cranelift_bridge.so` (July) and fixed. W9.3 must be re-run in
CI or after the `lean4export` pin is refreshed; nothing here claims it passes.

## U11 admission — scalar imports now call the generated adapter

The U04 transport restriction is replaced by adapter-mediated admission.

| Path | Change |
|---|---|
| `lib/TypingPass.ml` | `is_transport_word_parameter` / `is_transport_i64_result` and the `rust_interop_transport` rejection are gone: `Bool`, `Int32`/`U32`, `F32`/`F64` parameters and results are admitted. The buffer-protocol shape check (`rust_ownership_shape`) still refuses aggregates. |
| `lib/Compiler_cps.ml` | `foreign_needs_adapter` decides per `MForeignFunction` signature: if every parameter is `Int64`/`Nat64`/address/pointer and the result is `Int64`/`Nat64`, the call keeps the declared external symbol (unchanged word-native path); otherwise the lowered symbol is `<ext_name>__adapter`. Hence a float-register or narrow-integer call can only ever name the wrapper. |

**Evidence (measured, emitted CPS).** A module declaring and calling
`rs_narrow(width: Int32): Int64` compiles and emits CPS containing exactly
`rs_narrow__adapter` (no `rs_narrow`):

```
$ ./austral compile M.aum --entrypoint=M:main --use-cps-jit --allow-all \
      --target-type=c --emit-cps=M.cps --output=M.c
CPS JIT: Generated 1 functions (126 bytes)
panicked: can't resolve symbol rs_narrow__adapter
$ strings M.cps | grep -o 'rs_narrow[a-z_]*' | sort -u
rs_narrow__adapter
```

That is the fail-closed admission contract: the module typechecks, and it
**cannot** be loaded unless the generated wrapper is present, because the raw
symbol is never requested. With `umbrella/tests/fixtures/adapters/rs_scale`'s
wrapper compiled in, the same shape executes through `au_scalar_*`.

**Test-suite fallout (recorded honestly).** The two U04 fixtures
`019-extern-rust/008-transport-narrow-parameter` and `009-transport-float-result`
asserted the *rejection* this patch removes; with scalars admitted they compiled
and linked against a nonexistent `rs_narrow`, so they were deleted rather than
kept as unconditional passes. `test-programs/runner.py 019` is now **6/6 PASS**
(002–007). The runner has no per-fixture C stub support, so a success fixture
for a foreign symbol is not expressible there; the admission evidence above is a
compiler-level check, and an executing adapter fixture belongs with the U10/U11
integration work.

**Checks run:** `nix develop "git+file://$PWD#ocaml" -c make all` exit 0;
`LD_LIBRARY_PATH=$PWD:$PWD/safestos/cranelift/target/release python3
test-programs/runner.py 019` 6/6 PASS (the runner execs `./austral`, so the
bridge `.so` must be on the loader path; without it every fixture fails on the
loader, which is an environment fault and not a compiler result).

`make test` first failed two cases and both were fixed at the cause, in the
tree rather than in the assertions: `GpuWgslTest`/`GpuFrontendTest`/
`GpuSubsetTest` read `examples/gpu/pointwise/*` by repository-relative path,
which dune never stages — the stanza now lists them in `(deps …)` and the tests
resolve them through `TestUtil.fixture`, which walks up to the staged copy; and
`CliEngine:5` still expected the pre-GPU target sentence `One of \`exe\`, \`tc\`,
\`c\``, while the help — correctly — advertises the shipped `wgsl` as well, so
the expectation was updated to the parser's real vocabulary. `make test` now
exits 0.

## U11 — the CPS lowering emits the adapter descriptor (*pinned golden*)

The descriptor is now produced by the compiler, not typed by hand: the CPS
lowering knows exactly which foreign calls need a wrapper and what their
scalars are, so it is the only side that can state the contract.

| Path | Change |
|---|---|
| `lib/Compiler_cps.ml` | while lowering, each `rs_*` signature that is not word-native is recorded (`required_adapters`) and mapped to `<ext_name>__adapter`. `adapter_descriptors_text` renders them in the canonical form of `umbrella/src/interface.rs`: header comment, then one block per adapter (`schema 1`, `abi scalar 1`, `symbol`, `target`, `params A,B`, `returns R`), **sorted by symbol**, blocks separated by a blank line. `write_adapter_descriptors` writes it. A signature the schema cannot express (e.g. a buffer parameter beside a scalar result) is an error, never a guessed layout. |
| `lib/Compiler.ml`, `lib/CliParser.ml{,i}`, `lib/CliEngine.ml` | new `--emit-adapters=<path>`; refused with `--target-type=wgsl` like `--emit-cps`, and listed in the compile help. |
| `test-programs/adapters/rs_scalars.aum` | golden fixture: `rs_scale(Float64, Int64) -> Float64`, `rs_narrow(Int32) -> Int64`, `rs_toggle(Bool) -> Bool`, plus `rs_word_native(Int64) -> Int64` which must produce **no** descriptor. |
| `test-programs/adapters/adapters.golden` | the pinned text. |
| `scripts/check-adapter-descriptors` | re-emits and diffs byte for byte; exit 0 exact match, 1 diff, 2 cannot measure (no compiler binary / fixture / compile failure). Wired into `scripts/verify-invariants` as `U11: the CPS lowering's adapter descriptors match the golden`, with 2 reported as a gap. |
| `umbrella/tests/interface.rs` | `the_compilers_emitted_descriptors_parse_here` reads the same golden and parses every block with `Adapter::parse_descriptor`, asserting the symbol order, `<target>__adapter` naming, schema/abi versions and `rs_scale`'s `(F64, I64) -> F64`. The golden is therefore pinned from both languages: the compiler must keep emitting it, and the generator must keep being able to read it. |

**Measured.** `./scripts/check-adapter-descriptors` → `[pass] 3 descriptor(s)` on
two consecutive runs (byte-stable). `cargo test --offline --test interface` →
**6 passed** (pin raised from 5 in `scripts/test-umbrella`).
`verify-invariants` → **113 passed, 17 explained gaps, 8 failed**; the 8 are the
known W9.3 toolchain failures (no `mm0-c`/`mm0-rs`, host Lean 4.33.1, no Sail
model), unchanged by this patch. The gate's file had arrived from an upstream
merge carrying conflict markers (`<<<<<<< HEAD` around the W3 `_plan_file`/
`_appr_root` declarations, which are already defined above that block); the
stale hunk was dropped and the script parses again — same 113/17/8 figures
before and after that repair.

**Regression found and fixed while measuring.** The first version redirected
*every* non-word-native foreign import to `__adapter`, which broke
`examples/ffi`'s C import `putchar(Int32)`: no generated wrapper exists for a C
ABI, so the module stopped compiling (the gate reported `cps-corpus: [gap]
examples/ffi compiled no function`). Adapter mediation is now scoped to `rs_*`
imports — the Rust-interop contract — and the gap disappeared.

### U11.2 — the wrapper's *values*, measured against direct Rust calls

`umbrella/tests/scalar_roundtrip.rs` turns the descriptor into a running pair: it
writes a Rust library that exports the five declared targets (`#[export_name]`,
because the wrapper declares the same names in `extern "C"`), appends the wrapper
`render_wrapper` generates for each descriptor (one module per wrapper — the
generator writes a file per adapter, and each starts with the same `use`),
compiles the lot into a `cdylib`, loads it with the platform loader, and calls
both sides: the target as Rust declares it (`f64`, `i64`, `bool`) and the
wrapper with `u64` words built by the bridge's own `encode`.

Every result is compared **by bits**, and the cases are chosen so a plausible
wrong encoder fails: `-0.0` (a numeric cast drops the sign), `rs_sub` called with
both orders (a commuted pair returns a different value), `0.0/0.0` and
`1.0/0.0` (NaN and infinity survive the word), `rs_narrow(-7)` (a negative
`Int32` sign-extends), and `rs_toggle(true)` (a `Bool` word is 0 or 1). NaN is
compared to the direct call's bits rather than a constant, because the payload is
the implementation's and the claim is only that the word carries it.

The second test is the failure oracle: a wrapper whose result leaves through
`result as i64 as f64` still compiles and still returns `-10.0` for
`rs_scale(-2.5, 4)`, and loses only the sign of zero — so the bit assertion is
what has to catch it, and the test pins that the mutation really does return
`+0.0` (otherwise the control has stopped discriminating).

**Measured.** `scripts/test-umbrella --case SUITE_UMBRELLA_SCALARS --case
SUITE_UMBRELLA_INTERFACE` → 2/2 pass (2 and 6 tests); the harness counts are
pinned in the same `SUITES` table.

### U11.3 — the build consumes the descriptor

`umbrella` no longer asks a second time what the compiler already knows. The
front-end is invoked with `--emit-adapters=<tmpfile>`, and
`Adapter::parse_descriptors` reads the whole document: blocks are blank-line
separated, `#` lines are comments, a duplicate symbol is refused, and a
document with no adapters is a real state — the module needs none. The parse is
the same code the golden test uses, so the emitter's output has exactly one
reader in the tree.

Stage 3.5 then does the work the descriptor names: `build_adapters` renders one
wrapper per adapter with `render_wrapper`, writes them into a scratch crate,
links it against the bridge **rlib** found beside the umbrella executable, and
`dlopen`s the result (`RTLD_NOW|RTLD_GLOBAL`) *after* the module's own cdylib,
so the targets are already resolvable when the wrapper's relocations are
processed. An empty descriptor prints `the module needs no scalar adapters`, so
the no-adapter case is stated rather than silently skipped — `Demo.aum`'s live
run reports exactly that and still returns its documented `RESULT`.

**Measured.** `cargo test --offline --test interface` → 7 passed (new
`a_document_with_a_repeated_symbol_is_refused`; pin raised 6 → 7);
`./target/debug/umbrella --manifest module.toml modules/Demo.aum` →
`umbrella: the module needs no scalar adapters` then
`RESULT -1781872834555808882`.

### U11.4 — the live JIT path, and the float word

`scalar_roundtrip.rs`'s third test drives the real `umbrella` binary over
`modules/AdapterLive.aum`, whose six externs are `Int32`, `Bool`, two `Float64`
and two `Float32` signatures — none word-native, so all go through
`<target>__adapter`. The test builds the target crate, grants the six adapter
symbols in a manifest, and asserts the `RESULT`, the `built and loaded 6 adapter
wrapper(s)` line naming each symbol, and that a wrong `--expect` fails (`compute`
returns `0` unless every mediated result is right, so the wrong-expectation run
is the oracle that `RESULT` is really the entry's). It needs the dune-built
`austral` front-end at the repo root and **fails** with that instruction when it
is absent, rather than skipping.

**Measured.** `cargo test --offline --test scalar_roundtrip` → 3 passed;
`scripts/test-umbrella --case SUITE_UMBRELLA_SCALARS` → pass, 3 tests (pin
raised 2 → 3).

**What it exposed, and the fix: `Float64` crossed as a rounded integer.** A first
version of the module also called `rs_scale_bits(2.5, 4)` and returned the bits
of the product. It returned `12`. That was not a wrapper bug: the CPS binary
carries one 64-bit literal and no float type, so `CpsGen.FloatLit` wrote
`Int64.of_float (Float.round f)` — `2.5` became the word `3` — and the adapter,
which correctly decodes its `u64` parameter as an `f64`, read
`f64::from_bits(3)`; `4 * that` is a denormal whose bits are exactly `12`.

The word a float needs in a one-word value model is its IEEE-754 bit pattern, so
`FloatLit` now writes `Int64.bits_of_float f`. Every `f64` has bits (NaN and
infinity included), which is why the out-of-range compile error that rounding
required is gone with it. The JIT needed no change: every import is declared
`(i64, …) -> i64` and every value is one word, so a float parameter was always
passed verbatim — only the literal was lying about the value. `AdapterLive.aum`
now takes a `Float64` parameter *and* returns one (`rs_scale` then `rs_bits`),
and the expected `0x4024000000000000` covers the literal, the `F64` parameter
decode and the `F64` result encode in one number that the old encoding could not
produce.

**`Float32` needed the literal narrowed, not a new opcode.** A Float32 word is
the f32 bits zero-extended (the ABI's `F32` decode refuses a word with bits above
bit 31), but `Mtast.MFloatConstant` is a string with no width and the typechecker
wraps a narrower literal in a cast, so every literal reached the CPS as an f64
bit pattern and the wrapper aborted with `0x3ff8000000000000 is not a valid
Float32 word`. `CpsGen.FloatLit32` writes `Int32.bits_of_float` under the same
`Int_lit` opcode (masked to 32 bits, since `Int64.of_int32` sign-extends), and
`Compiler_cps` emits it where the destination's declared type is
`MonoSingleFloat`: a call argument (the callee's parameter table), a local
(`MLet`/`MLetTmp` record their types), and a `return` (the enclosing function's
return type). The literal is looked for through casts, and only a literal is
narrowed — a Float32 returned by a foreign call is already the target's word.
`AdapterLive.aum` checks `rs_f32_bits(1.5)` = `0x3FC00000` (a literal written
straight into an argument) and `rs_f32_bits(rs_f32_scale(2.5, 1))` =
`0x40200000` (a literal in a Float32 local, then the parameter decode and the
result encode), both inside the module, before the Float64 leg returns.

**Still not carried:** float *arithmetic* — the JIT's `add`/`sub`/`mul` are
integer operations, so a computed float still needs a foreign call to do its
math, at either width. Only literals and mediated calls carry a float; a
Float32 that goes through a slot or a record field is still one opaque word, so
nothing in the JIT checks the width for you.
