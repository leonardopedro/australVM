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

## U07 — fail-closed compile-only CPS artifact (bounded patch)

This patch adds the additive emit-only compiler path and the host-side artifact
contract. The prototype path (`--use-cps-jit` without `--emit-cps-only`) is
unchanged, including its `Jit_compile_failed` refusal.

### What changed

| Path | Change |
|---|---|
| `lib/Compiler.ml`, `lib/Compiler.mli`, `lib/Compiler_cps.ml` | `emit_cps_only` ref and a separate branch in `compile_mod`: it runs `Compiler_cps.compile_module_cps`, writes the descriptor and CPS artifacts, and returns **before** `CamlCompiler_rust_bridge.initialize()` / `compile_binary`. An empty function list, a missing `--emit-cps`, an adapter write error and any lowering exception raise a compiler error; there is no C-backend fallback and no advisory environment in this branch. The normal path still prints `CPS JIT: Generated …` / `Emitted …` and still refuses an unresolved JIT symbol. `Compiler_cps.typed_exports` derives the functions/constants the lowering emits with their **wire** parameter/return types from the monomorphic module; the emit-only branch prints one tab-separated `CPS JIT: export\t<name>\t<params>\t<return>` line per export, so names with spaces/parentheses (monomorphs) remain unambiguous. |
| `lib/CliParser.ml`, `lib/CliParser.mli`, `lib/CliEngine.ml` | `--emit-cps-only` (bare bool). It is refused without `--emit-cps` and without `--use-cps-jit`, and is advertised in `compile --help`. `test/CliParserTest.ml`'s expected record carries the new field. |
| `umbrella/src/staging.rs` | **new.** A fresh private directory per run (`create_dir`, never reuse an existing path), a `fresh_path` refusal for pre-existing outputs, and removal on drop unless `retain()` is called. `cargo` staging is not moved here yet (U06 bullet 1 remains partly open). |
| `umbrella/src/frontend.rs` | **new.** `compile(austral, modules, liquid_required)`: runs `--emit-cps-only`, removes `CPS_JIT_ADVISORY` from the child environment, sets `AUSTRAL_LIQUID_REQUIRED=1` only when required (and removes an ambient value otherwise), and passes **no** `--allow-all`. It takes the **last** `CPS JIT: Generated <n> functions (<m> bytes)` line (a prefix compiles each module, and the last one is what survives the overwrite) and requires `n > 0`; requires every `Emitted CPS binary to <path>` line to name the staged path; and requires an existing non-empty file whose length equals `m`, plus an adapter document. Typed exports are parsed from the tab-separated lines into `Export { name, params, returns }`, require known CPS wire types, deduplicate identical declarations and refuse one name with two signatures. Failures are refusals, not warnings. `Artifact` carries source modules, module names, typed exports, CPS bytes, its sha256, the function count, the adapter document, the liquid mode and any `liquid:` notes. `digest` hashes all module CPS length-prefixed. Unit tests cover a complete acceptance, absence of `--allow-all`/advisory, liquid threading, missing/empty/zero-function/short-write/non-zero/foreign-path children, a missing export list and a same-name signature collision. |
| `umbrella/src/main.rs`, `umbrella/src/cli.rs`, `umbrella/src/lib.rs` | Stage 3 loops prefixes through `frontend::compile` instead of a `$TMPDIR/umbrella-<pid>.cps` file and the advisory opt-out. A new `--liquid-required` umbrella flag threads the requirement. The artifact digest, function count, module names and liquid mode are printed and emitted as one JSON `artifact` event. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U07 steps 1–3 (compile-only artifact + strict front-end) | CPS01/CPS02/CPS05–CPS07 | implemented with gaps | git `1bd6fee3` + this dirty patch; OCaml 5.5.2/dune 3.23.1 in `nix develop "git+file://$PWD#ocaml"`; umbrella is `umbrella/src/{frontend,staging,main,cli}.rs` | `dune build lib/ bin/ test/` → exit 0. Real fixture: `./_build/default/bin/austral.exe compile test-programs/suites/019-extern-rust/010-jit-unresolved-symbol/Test.aum --use-cps-jit --emit-cps-only --target-type=tc --emit-cps=_build/u07/out.cps --emit-adapters=_build/u07/out.adapters` → exit 0, `CPS JIT: module Test`, `Generated 1 functions (182 bytes)`, `CPS JIT: export<TAB>main<TAB><TAB>Int64`, `Emitted CPS binary to _build/u07/out.cps`; files `out.cps` 182 B and `out.adapters` 241 B; the stderr has **no** `functions compiled` and **no** `Compilation error`. `examples/memory-ops` additionally reports `export<TAB>monomorph_(Id.MonoId 15)<TAB>Int64<TAB>Int64`, proving a space-bearing monomorph name survives the tab format. Same module on the unchanged path (`--use-cps-jit --allow-all --emit-cps=…`) → exit 255 with the unresolved `rs_absent_word` error. `--emit-cps-only` without `--emit-cps` / without `--use-cps-jit` refuses with the named message. `CliParserTest.exe` → 4 tests OK. Rust: with the bridge's kernel feature temporarily disabled (and restored), `cargo test --offline --lib` → **98 passed, 0 failed** (including 11 frontend + 2 staging tests) and umbrella entry/cli/policy/linking → **5 + 6 + 6 + 1 passed**. | A lowering that the JIT cannot resolve still produces a CPS artifact in emit-only mode, because the bridge is never initialized or asked to compile it; the same bytes on the runnable path still fail closed. No `--allow-all` is passed by the umbrella, and the child environment proves the advisory opt-out is absent. The artifact now names every typed export the lowering actually produces, with wire types and no first-wins on a name collision. | The **default-feature** Rust build/test is still blocked in the sibling `../unfer` checkout, which is mid-merge with conflict markers in `unfer_consensus/src/lib.rs` (pulled in by `unfer_ffi` → bridge default `unfer-kernel`); the 98-test run above proves the new code typechecks and its tests pass, but not the production feature set. `front_end` still takes the adapter document from the last prefix; `cargo build` does not use private staging; `CliEngineTest` still fails its pre-existing mlir help expectation (`One of exe, tc, c, wgsl` vs the shipped `mlir`), unrelated to this patch. |

**Full-suite note.** `dune runtest test` on this host reports 9 `LiquidVCTest` failures with `compiling … failed (exit 127)` — the `.#ocaml` shell ships no prover/checker for that suite. That is the documented shell limitation, not a U07 result. `make all` is blocked before dune by `check-bridge-fresh` (`safestos/cranelift/src/adapters.rs` is newer than the existing `.so`), so the direct `dune build` above is the measured compile.

## U08 — explicit provider binding and loaded-library ownership

This patch replaces the project path's `dlopen` + `dlsym(RTLD_DEFAULT)` symbol
lookup with an explicit admission step. The no-project DenyAll path keeps the
prototype's global mapping, because it has no declared provider identity to
bind.

### What changed

| Path | Change |
|---|---|
| `umbrella/src/provider.rs` (new) | `ProviderSet::load(paths, declared)` rejects an empty provider/export set and repeated paths (textual and canonical, so a symlink is not a second provider); resolves each declared name with `dlsym` scoped to **that** mapped image; requires exactly one definition; rejects reserved `au_`/`__`/`uk_`/`uz_` names; checks the bridge's existing static registry for a same-name/different-address collision; and only then registers the selected addresses via `register_static_symbol`. A separate `select_from` makes the duplicate/missing/load-order rule testable without shared libraries. Handles are retained in `ProviderSet` and never `dlclose`d: JIT code bakes the addresses, so unmapping would invalidate live pointers/handles/callbacks. |
| `umbrella/src/main.rs` | Stage 2 uses `ProviderSet::load` for a project run. The built cdylib (or the single declared prebuilt provider) is the admitted image; the declared set is `trusted.exports` plus any granted `rs_*` symbol the library actually exports (adapter/bridge grants are left to their own resolution). Refuses more than one provider library (the project schema pins one artifact identity); refuses an empty declaration. Keeps the set alive to the end of `main` and reports the admitted export names. The no-project path still maps globally. |
| `umbrella/tests/entry.rs` | The U07 fake front-end now emits the completion/emitted-path lines the strict frontend requires. Adds `two_provider_libraries_are_refused_before_mapping`. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U08 steps 1–3 (explicit provider + lifetime) | LNK01–LNK04, AUT05 | implemented with gaps | git `1bd6fee3` + this dirty patch; `umbrella/src/provider.rs`; bridge seam `register_static_symbol` / `austral_register_static_symbol` (already covered by `safestos/cranelift/tests/static_seam.rs::late_registration_is_refused`) | Reduced-feature Rust config (kernel feature off because the sibling `unfer` checkout is mid-merge), restored afterwards: `cargo test --offline --lib` → **95 passed, 0 failed** (9 provider tests); `cargo test --offline --test entry --test cli --test policy` → **5 + 6 + 6 passed**; `cargo check --offline --bin umbrella` → 0. The real-library test compiles two C shared objects both exporting `rs_umbrella_pick`, preloads the first `RTLD_GLOBAL`, admits the second, and observes `2` (the exact handle's address, not load order); admitting both refuses `defined by more than one admitted library`. Entry test: two providers in `load.providers` → exit 1 with `declares 2 provider libraries`, no `RESULT`. Provider unit tests cover reserved names, missing exports, duplicate declarations/paths and a same-name/different-address static-registry collision. | A granted `rs_*` export is resolved from the exact admitted image before JIT initialization, so a preloaded same-named export cannot win that JIT import; the name-based call gate is unchanged, and registration alone grants nothing. | The provider is now mapped `RTLD_LOCAL` and the adapter crate links it explicitly (U10 follow-on), so the previous global-scope adapter caveat no longer applies. The multi-provider in-process profile is refused rather than bound (needs per-provider ABI records). The default-feature Rust build still cannot run here because of the external `unfer` merge conflict; the test run is reduced-feature. |

## U09 — per-instance authorization context (bounded start)

D08 already gave the bridge immutable instance identities and a caller stack,
but `check_instance_call` still asked the process-global **merged** manifest. A
reload that named the same module merged its grants into the old set, so a
narrowed policy retained the earlier permissions. This patch snapshots the
policy into the instance.

### What changed

| Path | Change |
|---|---|
| `safestos/cranelift/src/auth.rs` | `InstanceContext.policy: Option<Arc<ManifestAuthEngine>>`; `bind_instance_with_policy` and `bind_instance_with_identity` bind a context with a parsed manifest snapshot and the delegating **user**; `replace_policy` builds the same identity with a different snapshot for a reload's compile scope; `rebind_instance_with_policy` installs the replacement (and the old `rebind_instance` preserves the existing snapshot/delegator). `check_instance_call` consults the instance snapshot when present and falls back to the global engine only for legacy contexts (`policy: None`). `ManifestAuthEngine::primary_principal` lets a host bind the manifest's module when it has no project document. Tests: A cannot use B's grant, a narrowed rebind refuses the old grant with `UK-4001`, and a user delegator is recorded without becoming the grant principal. |
| `umbrella/src/project.rs` | Optional `identity.user`. `deny_unknown_fields` accepts only `identity.user`, and an empty/whitespace user is a named validation error. |
| `safestos/cranelift/src/module.rs` | `load` binds with `bind_instance_with_policy(..., engine.clone())` before merging the engine globally. A hot-swap compiles under `replace_policy(current, new_engine)` — so a narrowing cannot have a call admitted by the old grants — and on success rebinds with the new snapshot. The existing failure paths still roll the global engine back and revoke the uncommitted instance. |
| `umbrella/src/main.rs` | `install_auth` returns the parsed `ManifestAuthEngine`; every run binds one instance with that snapshot (empty policy for an explicit DenyAll run), derives the module name from the project or manifest, records `identity.user` as the delegator, and enters the caller scope **before** JIT compilation. The capabilities block now installs into that same instance instead of binding a second one. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U09 step 1–3 (module/user/instance identity + effective grants) | AUT02–AUT04, LNK03 | started | git `1bd6fee3` + this dirty patch; reduced bridge features `cedar,tier1-rs` because the sibling `unfer` checkout is mid-merge | `cd safestos/cranelift && cargo test --offline --lib --no-default-features --features cedar,tier1-rs auth::tests` → **13 passed** (three new); `cargo check --offline --lib --no-default-features --features cedar,tier1-rs` → 0; umbrella `cargo test --offline --lib project::tests` → **8 passed** (identity parse + empty-user refusal); `cargo check --offline --lib --bin umbrella` → 0; `cargo test --offline --test entry --test cli --test policy` → **5 + 6 + 6 passed** | Two contexts bound with different snapshots each see only their own grant; a narrowed `rebind_instance_with_policy` refuses the removed grant with `UK-4001` while the replacement's remaining grant still works; the replaced instance is not live. A delegator (`did:unfer:alice`) is recorded and shown in the context label, but the grant lookup still uses the module, so the user neither gains nor moves a permission. Compilation runs inside the caller scope, so the gate resolves the instance's module rather than whichever manifest loaded last. | The per-instance policy is a grant snapshot, not a separate authorization engine/worker: a process that can call `rebind_instance_with_policy` is trusted to name the module/user. `capstd` operation authorization still uses its own `InstanceCaps` registry, and cross-instance **handle** owner tags are U13/U18. The global merged engine still exists for legacy callers and is merged on load, so it is not a containment boundary. No concurrent A/B calls or one-worker-per-generation invariant are exercised here. Default-feature Rust tests remain blocked by the external `unfer` merge conflict. |

## U10 follow-on — adapter/provider linkage, local scope

The U08 mapping was `RTLD_LOCAL`-safe for JIT imports but the adapter cdylibs
still had ordinary undefined `rs_*` references, so they needed the provider in
the global scope. This patch closes that gap for the driver path.

### What changed

| Path | Change |
|---|---|
| `umbrella/src/adapters.rs` | `AdapterCrate` records its exported symbols. `build_at`/`build_next_against` pass the admitted provider's **canonical absolute path** to `rustc` as a link input, so the crate gets `DT_NEEDED=/abs/path/libprovider.so` and the loader reuses the already-mapped image. `load_for_jit` maps the adapter `RTLD_LOCAL`, resolves each export through the adapter's own handle and registers it by address (refusing a same-name/different-address collision). `build_next`/`load` remain as the legacy global-scope path used by the reload tests. |
| `umbrella/src/policy.rs` | `dlopen_local` (RTLD_LOCAL) beside the existing global `dlopen`; both share one error-reporting path. |
| `umbrella/src/provider.rs` | Provider mapping uses `dlopen_local`; its docs now state the adapter-linkage half. |
| `umbrella/src/main.rs` | `build_adapters(adapters, provider)` uses `build_next_against(..., Some(provider))` and `load_for_jit`; the admitted `cdylib` is the link target. Module/README docs updated. |
| `umbrella/tests/linking.rs` | **new.** Builds a C provider exporting `rs_link_probe`, admits it locally, generates a scalar adapter, links it against that provider, maps it locally, and calls the adapter's exported symbol: it returns the provider's `7`. Without the link, the local adapter `dlopen` fails. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U10 linkage (RTLD_LOCAL provider/adapter) | LNK01/LNK02, ABI03 | implemented for the driver path | git `1bd6fee3` + this dirty patch; reduced bridge features `cedar,tier1-rs` because the sibling `unfer` checkout is mid-merge | `cargo test --offline --test linking` → **1 passed**; `cargo test --offline --lib` → **96 passed**; `cargo test --offline --test entry --test cli --test policy` → **5 + 6 + 6 passed**. The test's provider C source and adapter both map locally; the adapter's `rs_link_probe__adapter` returns `7`. `readelf -d` of the built adapter records the absolute provider path as `NEEDED`, and a probe host confirmed the loader reuses the existing mapping (one `libp.so` object) rather than loading a second copy. | The admitted provider's address, the adapter's calls and the JIT-visible adapter symbol all come from named handles/registrations, not `RTLD_DEFAULT`. A preloaded same-named export cannot answer for the provider call. | The legacy `build_next`/`load` path (used by the existing reload tests) still maps globally and intentionally leaves the references unresolved; only the driver uses the linked-local path. The adapter crate links the provider by absolute path, so moving the provider file after the build would break the adapter load. Reload/bind tests have not been rerun through the local path. Default-feature builds remain blocked by the external `unfer` conflict. |

## U10 — typed artifact pins and adapter data in the event

The link half above bound the provider and adapter images. This half binds what
flows through them: the module's exact typed signatures and the compiler-emitted
adapter descriptor document are now pins in the same `trusted` record, checked
before any descriptor-driven native work.

### What changed

| Path | Change |
|---|---|
| `umbrella/src/project.rs` | `TrustedArtifact` gains `signatures` (`[{name, params, returns}]`) and an optional `descriptor_sha256`. Both are optional for existing documents; when present, signatures are exact, names are unique, and the digest must be 64 hex digits. Nested unknown fields are named as `trusted.signatures[i].<field>`. |
| `umbrella/src/trust.rs` | `sha256_bytes`; `verify_signatures` compares the artifact's typed exports to the pin as an exact set (missing, extra, renamed, retyped or changed-arity all refuse with the name and both signatures); `verify_descriptor` hashes the emitted document; `verify_artifact` runs both. An absent pin is not a claim, so it is not a refusal. |
| `umbrella/src/main.rs` | The merged typed exports and the last prefix's descriptor document are verified against the project pin **before `ProviderSet::load`**: the provider identity checks still run before the front-end, but the mapping itself waits for the typed gate, so a refused contract never runs a provider initializer. After a successful artifact, `--emit-trust-record <path>` writes the pins `trust::trust_record` derives (signatures sorted by name, plus `descriptor_sha256`) as a fragment for the project's `trusted` block. The `artifact` JSON event carries `exports`, `adapters` (`kind` and `abi_version` as separate fields beside `symbol` and the canonical `descriptor` text) and `adapters_sha256`, alongside the CPS digest. |
| `umbrella/src/cli.rs` | `--emit-trust-record <json>`: single-valued, non-empty path, refused twice like any other single-valued option. |
| `umbrella/tests/entry.rs` | Five new cases: a pinned signature with a different arity is refused before `RESULT`; a wrong `descriptor_sha256` is refused before native work; both refusal cases also assert the stageprobe's `stages.log` holds no `load` marker (the probe appends it only when the cdylib's initializer runs); the artifact event's `exports`, empty `adapters`, and `adapters_sha256` are asserted; and a successful run's `--emit-trust-record` file carries the admitted `compute() -> Int64` signature and `sha256("")` descriptor pin. |

### Ledger

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U10 typed signatures + descriptor/provider digests, generated trust record, contract-before-mapping | ABI03, ABI06, API03 | implemented for the driver path | this dirty patch; `umbrella/src/{project,trust,main,cli}.rs`; `umbrella/tests/entry.rs`; reduced bridge features `cedar,tier1-rs` because the sibling `unfer` checkout is mid-merge | Reduced-feature `cargo test --offline --lib` → **105 passed, 0 failed** (new trust/project/cli cases included); `cargo test --offline --test entry --test cli` → **9 + 6 passed**. The pin tests use a real stageprobe cdylib and the real `umbrella` binary: a signature pinned as `compute(Int64) -> Int64` against an artifact exporting `compute() -> Int64` exits 1 with `trusted signatures refused` and no `RESULT`; a `descriptor_sha256` of all zeroes exits 1 with `trusted descriptor refused` and no `RESULT`; the success event reports `exports[0].name == compute`, `adapters == []`, and `adapters_sha256 == sha256("")`; a successful `--emit-trust-record` run writes a JSON fragment whose signature and digest pins parse back as the project's `trusted` fields; and both refusal tests leave the provider fixture's `stages.log` without `load`, which is the regression that the mapping used to print before the pins were checked. | A caller can pin an exact export name, parameter list and return type, and the compiler's adapter contract, before the artifact is used, and can generate those pins from an admitted run instead of copying them by hand. The provider bytes are still checked by D09's `library_sha256` before `dlopen`; the descriptor is checked after the front-end but before any wrapper is built or loaded, because the descriptor only exists then. Provider mapping now follows that check, so a tampered contract cannot run a provider initializer. | Signature pins cover CPS wire types, not source-level Austral types; the descriptor pin is exact bytes of the last prefix's document, so a change in meta-comment text also refuses. Provider and descriptor pins are not signatures over the provider's behaviour. Default-feature Rust verification remains blocked by the external `unfer` conflict. |

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
| `lib/TypingPass.ml` | `is_transport_word_parameter` / `is_transport_i64_result` and the `rust_interop_transport` rejection are gone: `Bool`, `Int32`/`U32`, `F32`/`F64` parameters and results are admitted. The buffer-protocol shape check (`rust_ownership_shape`) still refuses aggregates, and `is_adapter_protocol_type` refuses the scalars the adapter ABI has no encoding for (`Int8`/`Int16`/`Nat8`/`Nat16`/`Index`/`ByteSize`) with `rust_adapter_unsupported` — those used to typecheck and then lower to an `<name>__adapter` symbol no wrapper could implement, so the failure surfaced as a JIT `can't resolve symbol` (and, because a JIT error is only printed while the C output is still produced, with exit 0). Fixture: `test-programs/suites/019-extern-rust/008-adapter-unsupported-scalar`. The same reasoning refuses a byte buffer in an *adapter-mediated* signature (`rust_adapter_buffer_unsupported`): the descriptor has no length/capacity/provenance layout yet, so `Compiler_cps` could not describe it. The word-native shape is unchanged — `rs_checksum(buf: Address[Nat8], len: Int64): Int64` still typechecks, emits exactly `rs_checksum` and yields an adapter document with no descriptors. Fixture: `test-programs/suites/019-extern-rust/009-buffer-needs-adapter`. **Superseded by U12's mediation** (see that section): a byte buffer now always emits `<name>__adapter` plus a descriptor, whatever the word widths. |
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
`modules/AdapterLive.aum`, whose eight externs are `Int32`, `Bool`, two `Float64`,
two `Float32` and (since the unsigned leg) `Nat32`/`Nat64` signatures — none
word-native, so all go through `<target>__adapter`. The test builds the target
crate, grants the eight adapter symbols in a manifest, and asserts the `RESULT`,
the `built and loaded 8 adapter wrapper(s)` line naming each symbol, and that a wrong `--expect` fails (`compute`
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

**`Float64` arithmetic now lowers to bridge primitives.** The JIT's
`add`/`sub`/`mul` are integer operations and `trapping_arith_op` had already
turned `trappingSubtract`/`trappingMultiply` into the integer `Sub`/`Mul`
opcodes — on a float word that is arithmetic on two IEEE bit patterns, and the
multiply traps as integer overflow for perfectly ordinary values. `Compiler_cps`
now intercepts the four `TrappingArithmetic(Float64)` calls *before* that
fallback (the call node carries its result type, `MonoDoubleFloat`) and emits an
`App` to `au_f64_add`/`au_f64_subtract`/`au_f64_multiply`/`au_f64_divide`, the
same shape Int64 `+`/`/` already had. The four take and return the f64 bits in
the one-word model, are registered by `register_float_symbols` on every builder
(`cranelift_init`, the hotswap builder, `compile_cps_binary`), and are grant-free
because `au_` symbols are (`check_call_permission`). Divide aborts on `rhs ==
0.0` with `Division by zero in trappingDivide (Float64)`, mirroring
`BuiltInModules.ml`'s Int64/Float64 `TrappingArithmetic`; otherwise it is IEEE.
No opcode, `Cps_codec`, `ISA_REGISTRY` or census change is involved, so U11 step
3 stays satisfied. `AdapterLive.aum` now computes
`((rs_scale(2.5, 4) + 2.5) - 0.5) * 2.0 / 4.0` and returns it only when its bits
are `6.0` = `0x4018000000000000`, so a wrong operand order, a widened literal or
an op still on the integer path is a different number; `test/JitTest.ml` pins
each op separately (`f64_add`/`f64_subtract`/`f64_multiply`/`f64_divide`, 14
cases in the suite).

**Float32 arithmetic is not carried, and the reason is in the typechecker.**
The bridge already keeps both conversions (`au_f32_to_f64`, `au_f64_to_f32`) and
`test/JitTest.ml` exercises the whole shape by hand in four `f32_*` cases (29 in
the suite): widen both words, run the matching `au_f64_*` op, narrow back, so
`(((2.5 + 1.5) * 4.0) - 2.0) / 2.0` reads back `7.0` = `0x40E00000`. A
prototype of the source-level half was verified end-to-end (`adapter live`
lane, exit 0, that number accepted) and then reverted: `Float32` has no
`TrappingArithmetic` instance — `The typeclass TrappingArithmetic has no instance
which matches for Float32` — and the typechecker refuses the program before
lowering, so the lowering alone changes nothing.

Adding the instance is a two-line change to `lib/builtin/Pervasive.aui`/`.aum`
(`concat_builtins.py` embeds them into the generated, git-ignored
`lib/BuiltInModules.ml`), and that is where the cost is: every Austral
declaration there consumes ids from the *global* declaration/temp counter, which
the builtin module increments before any user module, so one `instance` line
renames every LiquidVC goal (`LiquidConstraints.goal_name` hashes the declaration
id) and every temporary in the compiled output. Measured on this tree, that one
line moved five LiquidVC goldens, `mm0/clif.mm0`'s use counts, both memsafe
ledgers and certificates, the flux1d certificate's `cps=`/`obj=` binding (and its
`arch`), and the `-- cites:` digests in `mm0/isa.mm0` and
`mm0/store_projection.mm0` that quote those artifacts — and from there the plan's
`§W3` digest rows and the toy-Lean pin. Each is regenerable, but they are
owner-committed trust artifacts, so re-pinning them belongs in its own change
(the alternative is making goal names and certificates independent of builtin
declaration order first).

**`Float64`/`Float32` comparisons now lower to bridge primitives too.**
`MComparison` used to fall through to the integer `CmpLt`/`CmpGt`/… opcodes,
which `icmp` the bit words as *signed* integers: right for positive values,
reversed between two negatives (`-2.0` = 0xC000… sorts above `-1.0` = 0xBFF0…),
`Eq` calling `-0.0` unequal to `0.0`, and a NaN equal to itself. `Compiler_cps`
now reads the operand type (`comparison_type`: a `MonoSingleFloat` operand makes
the comparison single-precision, otherwise any `MonoDoubleFloat` makes it
double) and emits `au_f64_lt`/`le`/`gt`/`ge`/`eq`/`ne` — the same 0/1 word the
`Cmp*` opcodes produce, so `If` and the boolean ops are untouched, and again no
opcode, `Cps_codec`, `ISA_REGISTRY` or census change. `Eq`/`NotEqual` go through
the primitives as well, because IEEE equality is not word equality. Float32
widens both words with `au_f32_to_f64` first (exact: every f32 is an f64, order
and NaN-ness preserved), mirroring the arithmetic chain; integer operands keep
the `Cmp*` opcodes. `AdapterLive.aum` now also requires `-2.0 < -1.0` and
`0.0 = -0.0` and `scaled32 = factor` (the widening path) before returning its
bits, so a comparison still on the opcodes returns 0 with every other leg
passing; `test/JitTest.ml` pins each primitive (11 cases: negative ordering both
ways, the `<=`/`>=` boundaries, `0.0 = -0.0`, `0.0 ≠ -0.0`, NaN `=`/`≠`/`<`,
and two Float32 widen cases).

**A `Nat64` literal above 2^63-1 compiled to `0`.** Both lowering paths decoded
integer literals with `try Int64.of_string s with _ -> 0L`, and `Int64.of_string`
rejects every unsigned literal above 2^63-1 — so a legal `Nat64` such as
`9223372036854775809` became the constant `0` in the CPS, silently. The literal's
text is the value: in the one-word model it is its two's-complement word
(`0x8000_0000_0000_0001`), so `Util.int_literal_word` now decodes the whole
unsigned 64-bit range exactly (leading zeros stripped, hex/octal/binary and
signed forms still through `Int64.of_string`) and *refuses* a literal outside
the range instead of wrapping it. Measured by diffing the emitted CPS: before,
the `Int_lit` operand was `00 00 00 00 00 00 00 00`; after, `01 00 00 00 00 00
00 80`. Pinned by `test/CpsCodecTest.ml`'s `integer literal words cover the
unsigned range` (2^63, 2^63+1, 2^64-1, sign and hex forms, and the two
out-of-range refusals), which is the same table the `Int_lit` opcode lives in.

**Unsigned widths, live.** `AdapterLive.aum` now carries the `Nat32`/`Nat64`
half of U11 step 1 through the real ABI: `rs_u32_low(0xFFFF_FFFF): Nat32` returns
`0xFFFF` (the `U32` decode refuses a word above `u32::MAX`, so a truncated or
sign-extended read is a different number), and `rs_u64_high(2^63+1): Nat32`
returns `0x8000_0000` — the full 64-bit word whose sign bit is set, narrowed by
the wrapper's `U32` result encode. Both are checked inside the module before it
returns its RESULT, so a `U64` parameter narrowed to 32 bits or sign-extended
returns 0 with every other leg passing. The lane's wrapper count rose 6 → 8 and
the grants name the two new `__adapter` symbols.

**A failed CPS JIT compile is now a hard error, not a printed warning.** A module
whose JIT compile fails has no runnable function, yet the compile used to print
`CPS JIT: Compilation error: …` and exit 0 (the C backend output is still
produced, which is why the old path looked successful). Measured on
`AdapterLive.aum` compiled without the wrappers loaded: exit 0 before, exit 255
now, with a diagnostic naming the unresolved symbol and the three ways a symbol
can be missing (`rs_*` adapter, `uk_*`/`uz_*` kernel, `au_*` primitive).
`CPS_JIT_ADVISORY=1` is the one opt-out, and only the umbrella's front-end run
sets it: that stage wants the CPS artifact and the descriptor, cannot see the
adapters it will build next, and compiles the CPS again itself once they are
loaded (stage 3.5). Its stderr still carries the compiler's error line, so the
advisory run reports what it tolerated instead of hiding it.

### U12 (bounded start) — the buffer descriptor's conventions

`umbrella/src/interface.rs` reads and validates an `abi buffer` block (version 1
and version 2), so the buffer layout is a checked contract. What it fixes, and
refuses anything else about:

* `params <n>` — total parameter count; every index is a parameter index.
* `lengths Nat64|Int64` — the length is a **separate 64-bit parameter**, named
  by index in each `buffers` entry. A 32-bit length is refused (a buffer above
  4 GiB would wrap), and no length is ever inferred from a pointer or an
  allocation.
* `buffers mode:buffer:length,…` — `mode` is `read`, `write` or `readwrite`;
  the buffer and length indices must be in range, distinct, and each buffer
  appears once. Capacity is the length: a `write` buffer's length is what the
  caller granted.
* `empty null-len0` — the only empty convention defined: length 0 means empty
  and only then may the pointer be null; length > 0 requires a non-null pointer
  covering at least that many bytes. A non-null pointer alone is never enough.
  A different convention needs its own ABI version, and an absent `empty` line
  is an error rather than a default.
* `returns` — a scalar, not `Unit`.
* `scalars index:Type,…` — version 2 only: 64-bit integer words (`Int64` or
  `Nat64`) that sit beside a buffer and are neither it nor its length. They are
  carried unchanged (`w<i> as i64`), never read as a length. In version 2 every
  parameter must be exactly one of a buffer, a buffer's length or a declared
  scalar — an unexplained parameter has no type the wrapper could give it, so it
  is refused. Version 1 keeps its published meaning (every non-buffer parameter
  is a length) and is not tightened under its old number.

`abi scalar` blocks in the same document are skipped, so one descriptor file
can describe both kinds.

**The writer now exists, for the shape version 1 can describe.** A parameter
that needs an adapter *and* carries a byte buffer is admitted only when every
buffer is an `Address[Nat8]` (crosses `readwrite`) or a `Pointer[Nat8]`
(crosses `read`) with its length as the immediately following `Nat64`/`Int64`
parameter, the lengths share one type, and the result is a scalar. The direction
comes from the pointer type, not from the `Rust_Ownership` annotation —
`Monomorphize` drops the pragma before CPS lowering, and an address is mutable
whether or not the callee says it only reads. `Compiler_cps.adapter_block`
writes the block; `TypingPass.check_rust_ownership` applies the identical rule,
so an admitted signature always has a descriptor and a refused one names no
`<name>__adapter`. Version 2 extends that shape with word scalars, so a
64-bit integer that measures no buffer is carried as a declared scalar instead
of being refused: `stdlib/gpu/Jobs`'s
`(context, buffer, offset, Address[Nat8], len)` calls compile as `abi buffer 2`
with `scalars 0:Int64,1:Int64,2:Int64`. Anything else is refused there with a
reason: a buffer with no length after it, a narrower integer, Bool, float or
non-Nat8 pointer beside a buffer, mismatched length types, or a buffer result. No parameter type escapes that
check: a byte buffer is always mediated, so `Address[Nat8]` + `Int64` → `Int64`
— the shape that used to stay word-native — now emits a block and lowers to
`<name>__adapter` as well.

`umbrella::interface::render_buffer_wrapper` implements the block: the shim
takes the same words as parameters, binds a length word once even when two
buffers share it, refuses a length above `i64::MAX`, maps length 0 to a null
pointer and panics on a non-zero length with a null pointer, then calls the
target with `*const u8`/`*mut u8` and the declared length type. Buffer and
scalar wrappers are rendered into one crate, because the JIT resolves
`<name>__adapter` without knowing which descriptor produced it.

Evidence: `test-programs/adapters/adapters.golden` is emitted from
`rs_scalars.aum` and pinned byte-for-byte by `scripts/check-adapter-descriptors`
(6 descriptors: `rs_count` read/Nat64→Nat32, `rs_fill` readwrite/Int64→Int32,
`rs_stamp` readwrite/Int64→Int64 with `scalars 2:Int64` — the version 2 shape —
and the three scalar ones), `umbrella/tests/interface.rs` parses and renders
those blocks from the same document, and the unit tests here cover the
round-trip, the mixed document, the wrapper's bindings, and the refusals (mode,
length type, index/duplicate rules, empty convention, ABI version).

**The wrapper now transports bytes, measured.**
`umbrella/tests/buffer_roundtrip.rs` compiles the targets and the rendered
wrappers into one `cdylib` and calls both through the real ABI (3 passed):

* a 5-byte fill into an 8-byte allocation leaves the three bytes past the
declared length at their canary, so the wrapper is shown to pass the *declared*
extent rather than the allocation, and its result and bytes match the direct
call;
* a checksum over `[1, 0, 2, 0, 3]` equals the direct call's and differs from
the checksum of `[1, 2, 3]`, so the NULs crossed as data (a `CString`-style
path would stop at the first one);
* the probe protocol is the empty convention in use: length 0 is passed as a
null pointer and the target still answers the needed size, writing nothing;
* a negative length and a non-zero length with a null pointer each abort
(`SIGABRT`, message checked) in a re-executed child of the test binary — the
wrapper refuses before the target is called, and the abort is why the check
lives in a child.

**A buffer call now crosses the JIT.** `umbrella/modules/BufferLive.aum` is a
compiled Austral module that allocates a Nat8 buffer itself (`allocateBuffer`
is the `au_calloc` intrinsic, a registered JIT symbol), passes it with a `Nat64`
length to three `rs_*` targets and returns 170 only if the second call reads
back exactly the four bytes the first wrote and the third returns that sum as an
`Int64`. `the_jit_executes_a_buffer_call_from_a_compiled_module` runs it through
the umbrella binary (`RESULT 170`, `--expect` checked) and requires the driver's
"built and loaded 3 adapter wrapper(s)" line naming all three wrappers, so the
chain is measured end to end: front-end names `rs_live_fill__adapter`, the
driver generates and loads it, the JIT resolves the mediated call, and the
wrapper marshals a pointer the *module* allocated.

The oracle is inside that run rather than in a second one: the allocation is
zeroed and the declared extent is 4 of 8, so `rs_live_fill` returns -1 when the
byte past the declared extent is no longer zero. A wrapper that passed the
allocation size would therefore produce RESULT 0, not 170.

**The word-native bypass is closed.** A buffer signature whose words are all 64
bits wide used to be lowered as a direct call: measured, `(Address[Nat8],
Nat64) -> Int64` produced a CPS naming `rs_wide` and an adapter document with no
block for it (consistent — the document only describes adapters the lowering
will call — but the call then dereferenced a pointer the wrapper's guards had
never seen; a null pointer with a non-zero length reached the target).
`Compiler_cps.foreign_needs_adapter` and `TypingPass`'s mirrored admission rule
now force mediation whenever a parameter is a buffer, so that shape emits
`abi buffer 1` … `returns Int64` and the CPS names `<target>__adapter`. Two
consequences are pinned rather than asserted: `Demo.aum`'s `rs_demo_checksum`
moved with it and `umbrella/module.toml` now grants `rs_demo_checksum__adapter`
(granting the native symbol would grant nothing, since no module can reach it),
and the live module carries a third, all-64-bit target (`rs_live_wide`) whose
only grant is `rs_live_wide__adapter` — a regression to the direct symbol is
refused by the JIT gate (UK-4001) instead of passing quietly. That change
needed no golden regeneration: both buffer cases already had scalar results
narrower than 64 bits. (Version 2 later added a third buffer case, `rs_stamp`,
for the word-scalar shape — see U12 above.)

**Not yet carried:** there is no text/UTF-8 distinction and no owned result
handle, and no capacity check beyond the declared length (the wrapper cannot see
the caller's allocation).

## U13 — the typed generational handle registry (bounded start)

An owned Rust value cannot be a word: the module would hold an address the
provider may have freed or reused, and neither side could tell. The first piece
is the table that owns those values. `umbrella/src/handles.rs` keeps one entry
per slot and `HandleRegistry<T>` hands out an opaque `Handle` — no integer, no
public constructor — that names a slot and the *provider generation* of the
slot's own state.

Four things are checked on every `get`/`get_mut`/`take`/`destroy`, in this
order, so the cheapest refusal does not depend on the value being live:

* **owner** — `Owner::issued` tags the handle with a per-registry instance;
  a handle from another instance (a second provider) is `ForeignOwner`.
* **type** — `TypeTag::named` carries the object type; asking the registry for
  a different `T` than the handle names is `ForeignType`, and a registry whose
  entries are not the queried type cannot be indexed by it at all.
* **provider generation** — a handle minted before the provider was rebuilt
  (`StaleProvider`) is refused even when its slot is live again.
* **slot generation** — destroying bumps the slot's generation, so the old
  handle is `Stale` forever and a value stored later in that slot is not
  reachable through it.

**Retirement, not wrap.** The slot generation is bumped with `checked_add`.
At `u32::MAX` the slot is retired instead of wrapping to 0, because a wrap would
make a handle from `u32::MAX` generations ago indistinguishable from a fresh
one; `insert` refuses with `Exhausted` only when every slot is retired. The
unit test `a_slot_whose_generation_would_wrap_is_retired_not_reused` drives that
path directly (a free slot is set to `u32::MAX`, one insert reuses it, one
`take` retires it, and the next insert must take a different slot).

**Destruction is exactly once.** `destroy` removes the value and drops it
inline, `take` moves it out to the caller's `T`; both bump the generation, so
the second attempt is `Stale` and never a second `drop`. `released` counts
successful ends (`destroy` + `take`), `retired` counts slots retired at the
generation ceiling, `issued` counts handles minted and `live` counts slots that
still hold a value — the counters the Drop oracle in the integration test is
compared against.

Evidence: `cargo test --offline --lib handles` (1 white-box test) and
`cargo test --offline --test handles` (8 behavioural tests over a `Tracked`
drop-counter oracle): schema version, read through the handle, double destroy
drops once, a reused slot does not resurrect, foreign owner, foreign type,
stale provider generation, and `take` leaving the handle stale.

**The wire form.** A handle crosses as two 64-bit words, packed without loss
because every component is 32 bits: `word0 = owner << 32 | slot`, `word1 =
provider generation << 32 | slot generation`. The pair `(0, 0)` is the null
handle and is refused as `Malformed` rather than read as slot 0, and owner tags
start at 1, so no live handle encodes to it. The words carry no type tag: the
type is the wrapper's, because the generator numbers the registries in one
adapter crate from 1, so a handle minted for one object type has an owner tag no
other registry in that crate holds. That is what keeps "wrong type" a refusal
even though the pair has no room for a type.

**The registry lives in the bridge.** `safestos/cranelift/src/handles.rs` is the
table; `umbrella/src/handles.rs` re-exports it. A generated wrapper is compiled
with `austral_cranelift_bridge` as its only dependency, so a registry hosted in
the umbrella crate could not be linked by the very code that must use it — and
two copies of the table would be two tables. `HandleRegistry::new` is `const`,
which is what lets a wrapper hold one in a `static Mutex` with no initialization
step and no allocation until the first insert.

**Allocation and free are counted separately.** `allocated` counts stored
values and `freed` counts the values this table destroyed in place;
`take` releases a slot but hands the value to the caller, so it is not a
`freed`. That is what lets a test hold `allocated`, `freed`, `released` and the
provider's own drop counter against each other: a value that left through
`take` is dropped by the caller, and the oracle sees exactly that.

**The generator.** An `abi handle 1` block names the type, the two provider
symbols, the constructor's arity and — when the module names one — the
provider's reader:

```text
schema 1
abi handle 1
symbol rs_widget__new
target rs_widget
type widget
constructor rs_widget_ctor
destructor rs_widget_dtor
use rs_widget_read
params 1
```

The `use` line is optional and sits between `destructor` and `params`, so a
descriptor without a reader is byte-identical to the older form and a reader is
never implied.

The compiler writes that block itself: a `Foreign_Import` carrying a
`Handle_Ownership` pragma names only the handle's *target*, and the CPS
lowering derives `<target>__dispose` for the call and emits one `abi handle 1`
block beside the scalar/buffer ones, in the same canonical document
(`--emit-adapters`). The pragma is
`Handle_Ownership(Object => "widget", Constructor => "avm_widget_ctor",
Destructor => "avm_widget_dtor", Params => "2", Use => "avm_widget_read")`;
the field is spelled `Object` because `Type` is a keyword, and `Use` is optional
— it names the provider's reader and is what makes a `use` line, and so a
`__use` wrapper, exist. It is validated against the import's signature on the
typed AST: it needs a `Foreign_Import`, a plain-identifier target that is not
already a wrapper name, a non-empty and distinct constructor/destructor pair,
non-negative arity, and exactly the dispose ABI (two `Int64` words in, one
`Int64` status word out). The annotation rides through monomorphization as the
descriptor's facts, since the lowering — not the typing pass — writes adapters.
A `Foreign_Import(External_Name => "avm_widget__use")` is what calls the
reading wrapper; the lowering leaves that name alone, because the wrapper is a
generated symbol, not a handle import.

**The pragma's refusals are fixtures, not prose.** Both are typing-pass errors,
so they fire before any descriptor is written and need no provider symbol to
exist. `test-programs/suites/027-handle-pragma` runs them:

* `000-dispose-signature` — the import the pragma annotates must have the
dispose ABI, two `Int64` words in and one `Int64` out. An `Int32` first
parameter is refused with "A `Handle_Ownership` import must have the dispose
wrapper's signature…", because the call the lowering derives (and the words
the registry accepts) are 64 bits wide.
* `001-not-foreign` — the pragma describes a foreign import only. Putting it on
a function with a body is refused with "The `Handle_Ownership` pragma can only
annotate a foreign import declared with `Foreign_Import`…", so a plain Austral
function cannot be given a destructor it never had.

Both stderr transcripts are captured byte for byte, so a reworded diagnostic is
a fixture update rather than a silent drift.

**The wrapper ABI.** All four wrappers speak `i64` only and return a status
word: `0` done, `1` refused. A refusal never writes the out slot, so a caller
cannot mistake an old word for a fresh one, and the words stay live unless the
wrapper is `__dispose`.

| wrapper | signature | notes |
| --- | --- | --- |
| `__new` | `(w0..wN-1, *mut i64 out) -> i64` | `out` receives the two handle words; a null `out` refuses before the provider allocates |
| `__use` | `(lo, hi, *mut i64 out) -> i64` | one provider word written through `out`; rendered only with a `Use` fact; does not consume the handle |
| `__with` | `(lo, hi, extern "C" fn(*mut c_void) -> i64, *mut i64 out) -> i64` | the callback gets the provider's pointer while the table is locked; `out` gets its word. Rendered always; a module passes an ordinary Austral function of type `Fn[Pointer[T], Int64]`, whose code address the lowering takes |
| `__dispose` | `(lo, hi) -> i64` | only a live pair reaches the destructor; every other case is `1` |

The out slot is an `i64*`, so the module hands over a `Pointer[T]` obtained from
a *record* borrow (`refMutToPointer(&!slot)`): a scalar borrow lowers to the
constant address 0 and would be refused as a null `out`, which is why the
slots are one-word records rather than bare `Int64`s.

`HandleAdapter::parse_descriptors` numbers the registries from 1 and the number
is the owner tag the words carry; the tag is not part of the text, because it is
the generator's numbering inside one crate, not a fact about the type. The
descriptor is refused unless `symbol` is exactly `<target>__new` (so the dispose
symbol `<target>__dispose` is derived from the same name and a module cannot
name a dispose for a type whose constructor it never named), unless the type is
a plain identifier, unless the tag is non-zero, and unless constructor and
destructor differ. `render_handle_wrapper` writes both wrappers into one module
over one registry:

* `__new(words…, out)` — the constructor's `params` 64-bit words, a status word,
  and the handle's two words written through `out`. Status `0` is done, `1` is
  refused; a null `out` is refused *before* the provider allocates, so a
  mistaken caller cannot leak an allocation that has no handle.
* `__dispose(lo, hi)` — the two words back through `destroy_words`: a null pair,
  another registry's owner, a previous provider generation and a stale slot are
  all `1`, and only a live pair reaches the destructor.
* `__use(lo, hi, out)` — rendered only when the descriptor has a `use` line.
  The same live pair is resolved in the same registry, the provider's reader
  (`avm_widget_read`) turns its pointer into one `i64`, and `out` receives it.
  A refusal (null `out`, stale words, another owner) returns `1` and leaves
  `out` untouched, so a caller cannot read an old word as a fresh value, and
  reading does not consume: the words stay live for `__dispose`.
* `__with(lo, hi, callback, out)` — the scoped borrow, rendered for every
  handle adapter because it needs nothing from the provider. The live pair is
  resolved, the caller's `extern "C" fn(*mut c_void) -> i64` is handed the
  provider's own pointer *while the table is locked*, and its word is written
  to `out`. The borrow ends with the call: the value cannot be destroyed while
  the callback holds it, and the module receives a word, never the pointer. A
  null `out`, a null callback, a stale pair, another owner or a previous
  generation is refused before the callback runs. The lock is not re-entrant,
  so a callback that calls back into this registry would deadlock — the price
  of a borrow the other wrappers cannot invalidate.
* The registry's `PROVIDER_GENERATION` is the render's, not the descriptor's:
  `render_handle_wrapper_at` writes the build's generation, `main.rs` takes the
  next number for every build in the process, and generation 0 (the reserved
  number) is refused at render time. A reload's table therefore refuses the
  previous build's words as `StaleProvider` instead of resolving a slot that
  now means a different value.
* The constructor's pointer is wrapped in `ProviderValue`, whose `Drop` calls the
  provider's destructor. The host never frees a provider's memory; a refused
  insert drops the value there and then, so the provider is not leaked, and a
  destroyed value is freed once because the registry never mints its words
  again.

**The module side is a linear type.** `umbrella/modules/WidgetHandle.aui`
exports only `type Widget: Linear;` plus `adopt(lo, hi)`, `read(widget)`,
`dispose(widget)` and a `main` that is the intended path; the record itself is the body's, so no client
module can construct a `Widget` by hand. `adopt` is the construction seam (it
takes the words a granted `__new` produced) and `dispose` destructures the
record — which is the linear obligation discharged — and hands the words
straight back to the dispose wrapper. The body declares no wrapper symbol:
`pragma Foreign_Import(External_Name => "avm_widget");` names the target and
`Handle_Ownership` names the provider facts (including `Use =>
"avm_widget_read"`); the compiler derives `avm_widget__dispose` for the call
and emits the block the generator renders. `read` borrows the record, passes
its words and a local `Pointer[Word]` slot to the generated
`avm_widget__use` (a pointer, not an `Address`: the wrapper writes one word
through it — and a *record* borrow, because the lowering makes a scalar borrow
the constant address 0), and returns the word the provider's reader produced
or `-1` when the words were refused — a borrow, not a consume, which is why
the same handle can still be disposed afterwards. `main` is the whole path,
from source: `widgetNew(7, 9, refMutToPointer(&!pair))` writes the minted pair
into a local record, `adopt` makes it the linear value, `read` borrows it,
`dispose` hands it back once. Both constructor parameters are 64-bit scalars,
so the call is word-native: measured, the compiled body contains
`extern au_int64_t avm_widget__dispose(au_int64_t lo, au_int64_t hi);` and the
direct call, `extern au_int64_t avm_widget__new(au_int64_t lo, au_int64_t hi,
mono_15* out);` and its direct call, with no adapter, because there is no
buffer and nothing to encode.
`--emit-adapters` writes `test-programs/adapters/handles.golden`, which
`scripts/check-adapter-descriptors` diffs byte for byte, and
`umbrella/tests/interface.rs` parses with the same reader the build uses.

Evidence: `cargo test --offline --lib handles` in the bridge (3 white-box
tests: the retirement path, the word pair and the separated counters) and
`cargo test --offline --lib --test handles` in the umbrella (63 lib tests plus
the handles integration test: the registry and encoding cases, create/destroy,
read, the scoped borrow, and a reload), The wrapper test compiles the provider and
both generated modules into one `cdylib` and drives them through `dlsym`: a
constructor returns `0` and a non-null pair, the first dispose returns `0` and
the provider's drop counter moves once, the second returns `1` with the counter
unchanged, another type's dispose refuses these words while its own still work,
a slot reused after disposal hands out the same low word with a new generation
and the old words are refused while the new value stays live, and a forged
provider generation, a null `out` and the null pair are each refused without
allocating. `test-programs/suites/025-linear-handles` runs three fixtures
(`python3 test-programs/runner.py 025-linear-handles` → 3 PASS): adopt-then-
dispose runs and exits 0, forgetting the handle is a linearity error ("is not
consumed by the time of the return statement"), and disposing it twice is a linearity error ("has already been consumed"),
and `test-programs/suites/027-handle-pragma` runs the pragma's two refusals
(`python3 test-programs/runner.py 027-handle-pragma` → 2 PASS): a dispose
import with an `Int32` word and the pragma on a non-foreign function.
`./austral compile
umbrella/modules/WidgetHandle.aui,umbrella/modules/WidgetHandle.aum
--entrypoint=WidgetHandle:main --target-type=c` exits 0.

**Not yet carried:** the module mints through `__new` (the bridge names
`__embed` per arity — `__embed$<argc>` — so one module may carry embeds of
several shapes, which is what the record out slot needed), reads through
`__use`, lends through `__with` and disposes through `__dispose`. The
callback is the module's own `onBorrow`, declared with the foreign import as
`Fn[Pointer[Pair], Int64]`: the CPS lowering names the emitted function
(`function_value_names`) and the JIT resolves a bare `Var` that is not a
local to that function's **code address** (`func_addr`), which the call
passes as the `au_fnptr_t` argument. Two limits stay visible: a function
value resolves only to a function *in the same CPS module* (a cross-module
function value would need an import with a signature the name alone does not
carry), and `MFptrCall` is still a call by name, so a module cannot yet call
through a function value it was given — it can only hand one over.The provider generation is not constant and is no longer exercised by
a second render: `umbrella/src/adapters.rs` builds each crate at the next
generation into a new artifact, and `tests/handles.rs` drives a real reload —
build, `dlopen`, build again, `dlopen` — so the reloaded crate refuses the
previous build's words while the previous crate still resolves its own.

A module that is already running follows a reload. Each build exports its
wrappers twice: the forwarder (`<target>__new`, `__with`, `__dispose`, …) is
the symbol the JIT resolved at compile time, and the body is a
generation-suffixed implementation (`<target>__new__impl_<generation>`). The
forwarder looks the name up in a per-image binding table and runs the body it
finds, or its own when nothing was bound — which is what the first build does.
`adapters::bind` walks a loaded build's `umbrella_bind_<target>` entry point and
binds each forwarder to the next build's implementation, so the compiled
module's calls reach the current table without being recompiled:
`a_module_running_across_a_reload_sees_the_next_generation` runs a module
across the bind, has it read through the reloaded table (accepting the next
generation's words), and checks that the reload then refuses the words it
minted before. What a reload does **not** swap is the provider's own code: a
new build is a new wrapper crate over the same `<target>_ctor`/`_dtor`/accessor
symbols, so a provider whose behavior changed needs a rebuilt provider *and* a
freshly loaded wrapper image, not merely a new generation. Borrows/views over an
owned handle are still U14's.

**Still not carried:**
Only literals, mediated calls, Float64 arithmetic and float comparisons carry a
float; a Float32 that goes through a slot or a record field is still one opaque
word, so nothing in the JIT checks the width for you, and a Float32 comparison
is correct because widening cannot fail, not because the width was checked.

## U14 — borrowed views over owned handles (runtime boundary)

A handle is a value the table owns, but a module also needs to *look at* that
value without consuming it, and a second path that could mutate or destroy it
while a view is live would be a use-after-free dressed as a borrow. The table
now has claims: `borrow` mints an immutable view token and `borrow_mut` an
exclusive one, `release` discharges the token, and every conflicting operation
is refused while a claim is live.

**What is checked, and where.** `Borrow` is deliberately neither `Clone` nor
`Copy`, so one claim has exactly one token and `release` takes it by value; a
token this registry did not issue — or one already released — is
`HandleError::UnknownBorrow` rather than a second decrement. Claims are kept per
slot, so:

* any number of immutable claims coexist (a group borrow, several readers);
* a mutable claim is refused while any claim is live, and a new claim of any
  kind is refused while a mutable claim is live;
* `get` is refused while a mutable claim is live (no `&T` beside a `&mut T`),
  `get_mut` while any claim is live, and `take`/`destroy` while any claim is
  live — a borrowed value cannot be replaced or freed under its view;
* `get_borrowed`/`get_mut_borrowed` are the paths through a token: reading
  works for either claim, mutation only for the exclusive one, and a copied
  handle's own `get_mut` stays refused while the claim is live — so the token
  is the capability and the handle is not;
* `borrow_words`/`borrow_mut_words` are the same checks over the two module
  words, so a caller never has to rebuild a `Handle` to take a view.

The Chest/Gem shape is the test `replacing_a_child_while_its_view_is_live_is_refused`:
a view of the child is taken, `destroy`/`take` of the child is refused with
`Borrowed`, and only after `release` may the child be replaced. The old handle
then resolves to `Stale` because the slot generation moved on, so the stale view
cannot name the replacement. `a_group_borrow_coexists_and_a_mutable_claim_excludes_everything`
checks the group/conflict rules in both directions (including a write through
`get_mut_borrowed` and the refusal of the handle's own `get_mut`),
`a_live_borrow_blocks_mutation_and_destruction_until_released` checks that
reading is what a view may do and every write is refused, and
`a_token_from_another_registry_is_refused` checks that a foreign token cannot
touch another table.

**Which guarantee is static, which runtime.** The module side is static where
U13 made it so: `Widget` is `Linear`, so forgetting or double-disposing a handle
is a type error, and no module can fabricate a handle from two integers. The
borrow rules above are **runtime** checks at the registry boundary. `__use` and
`__dispose` take no claim; they are single-word operations under the registry
mutex and the slot generation checks.

**A module-held view.** The `__with` callback scopes a borrow to one call, which
is not the same as a module *holding* a view across statements. Three generated
wrappers make that possible, and `WidgetHandle` uses them:

* `<target>__borrow(lo, hi, out)` mints an immutable claim and writes its
two-word token through `out` (claim id and owner tag); a null `out`, a stale
pair or a foreign owner is refused and the out slot is untouched;
* `<target>__view(w0, w1, out)` reads the provider's word through the token,
rendered only when the descriptor names a provider reader (it calls that
reader); a released or foreign token is refused;
* `<target>__release(w0, w1)` discharges the claim; releasing twice is
`UnknownBorrow`, so a copied token cannot decrement someone else's claim;
* when the descriptor also names a writer, `<target>__borrow_mut(lo, hi, out)`
mints an exclusive token and `<target>__write(w0, w1, value)` calls that provider
writer under the claim. An immutable token is refused by
`get_words_view_mut`, so a reader's view cannot become a writer's; a writer
panic is caught like a reader panic.

On the module side `type View: Free` is opaque, `holdView(widget): View` mints
a read-only view (`0`/`0` on refusal), `holdViewMut(widget): View` mints the
exclusive one, `viewRead(view)` reads through either token, `viewWrite(view,
value)` mutates through the exclusive one and `release(view)` discharges it.
`WidgetHandle.main` is the whole path from source: mint, adopt, hold a view,
read through it, release it, hold the exclusive view, write through it, release
it, then read through `__use`, lend through `__with` and dispose. The registry
refuses `__dispose` while either view is held, so a module that disposes before
releasing gets a status word, not a freed value; the compiler cannot see the
ordering, so this half is runtime.
The borrowed-view guarantee is thus split: **static** for ownership and
linearity (`Widget` is `Linear`), **runtime** for view lifetime and handle
validity.

`__with` remains the callback-scoped form and is safe by exclusion: it holds
the registry mutex for the callback, so a concurrent `__dispose` waits rather
than invalidating the borrow, at the cost of a non-reentrant callback. A
module still cannot call *through* the function value it hands `__with`
(`MFptrCall` is a call by name), which is why the held view exists.

**Evidence.** `cargo test --offline --lib handles` in `safestos/cranelift` →
13 passed (the 3 U13 registry tests, 8 borrow/view tests and the mutable-token
test); `cargo test --offline --lib` → 90 passed, 0 failed; `rustfmt --check`
clean on `src/handles.rs`. In `umbrella`, `cargo test --offline --lib
interface` → 30 passed (including the rendered-wrapper assertions and the
`write`-line parse/validate cases), and `cargo test --offline --test handles`
→ 21 passed, including:

* `a_held_view_blocks_disposal_until_the_module_releases_it` — a view is
minted and read, `__dispose` is refused while it is held, release lets the same
words dispose exactly once, and a second release or post-release read is
refused;
* `a_mutable_view_writes_and_the_write_is_visible_after_release` — a read-only
token's `__write` is refused, an exclusive token writes, still blocks disposal,
reads the written word through `__view`, and after release the value `__use`
returns is the written one while the released token cannot write again;
* the two `reexport` tests, exercising the claims and the word tokens only
through `umbrella::handles`, so the re-export and the bridge's table are the
same definition rather than two types that happen to agree.

`bash scripts/check-adapter-descriptors` → 6 scalar/buffer + 1 handle golden
match. The updated `WidgetHandle` compiles from source (`./austral compile ...
--target-type=c` → exit 0); its `abi handle 1` golden gained exactly one line,
`write avm_widget_write`, and was re-recorded deliberately in this change. The
three new wrappers are generated from the descriptor rather than described in
it.

**Not yet carried:** the exclusive view exists only when the descriptor names a
`write` line, so a provider that exposes no writer gets no mutation wrapper at
all — the generator never invents a writer symbol. A write carries exactly one
64-bit word; wider payloads need a buffer adapter or a handle. A claim is not
scoped to a thread or a reactor: it is per registry and released explicitly, so
lexical scoping for host callers is still the caller's to enforce. There is no
compiler-negative fixture for releasing a view after disposal — the module
ordering is checked at the registry boundary, not by the type system. U15's
payload and panic rules are separate.

## U15 — the result/panic status ABI in the generated wrappers (bounded start)

The handle wrappers already returned a status word beside the payload they wrote
through `out`; U15 makes that ABI named and checked, and stops a provider panic
from crossing it.

**The status words.** `safestos/cranelift/src/handles.rs` defines
`STATUS_OK = 0` (done), `STATUS_REFUSED = 1` (the host refused — a bad, stale or
foreign word, a null out, a null callback — so no provider code ran) and
`STATUS_PANIC = 2` (a provider panic was caught). `Status::from_word` is the one
decoder: the three words map to `Ok`/`Refused`/`Panic` and every other value is
`Err(word)`. That is what keeps a payload out of the status channel: `-1`,
`i64::MIN` and `i64::MAX` are not statuses, and `Status::from_word` returns them
as the unrecognised value rather than reading a legitimate domain result as a
failure. The generated wrappers use the constants instead of bare `0`/`1`, so
the ABI is one spelling.

**Panics.** The generated handle module declares the provider's constructor and
reader in an `extern "C-unwind"` block; the destructor stays in an `extern "C"`
block because `ProviderValue::drop` calls it through that ABI. Every provider
call the wrapper makes is wrapped in
`std::panic::catch_unwind(std::panic::AssertUnwindSafe(...))`:

* `__new` — a panicking constructor returns `STATUS_PANIC`, writes no handle and
  stores no value, so the table has nothing to clean up;
* `__use` and `__view` — a panicking reader returns `STATUS_PANIC` and leaves the
  out slot untouched, so no old word can be mistaken for a fresh read;
* `__with` — a callback that unwinds is caught the same way; the claim and the
  value stay live, so `__dispose` can still run.

A provider built with `panic=abort` never unwinds: the process aborts and this
is a **worker crash**, not a catchable status. The generated module says so in
its own comment. A panicking destructor is also not caught (it runs inside
`ProviderValue::drop`, not through a wrapper call) and is a provider fault.

**Evidence.** Bridge `cargo test --offline --lib handles` → 12 passed
(`a_status_word_is_checked_and_domain_edges_are_not_statuses` covers the three
words and the rejected edges `-1`, `i64::MIN`, `i64::MAX`, `3`). Umbrella
`cargo test --offline --lib interface` → 28 passed, including the render
assertions for the status imports, the `C-unwind` block, `catch_unwind` at each
site and the named statuses. `cargo test --offline --test handles
a_panicking_provider_is_a_status_word_and_keeps_the_value_live` → 1 passed over
a compiled wrapper crate: a panicking constructor returns `STATUS_PANIC` with
no handle and no stored value; a live value whose reader panics returns
`STATUS_PANIC` with the out slot untouched, and remains releaseable and
disposable; the same through a held view. The ordinary path still reports
`STATUS_OK`/`STATUS_REFUSED` — `the_reading_wrapper_hands_back_the_provider_value`
→ 1 passed. `a_panic_abort_provider_aborts_the_worker_instead_of_reporting_a_status`
compiles a self-contained `panic=abort` child binary that calls a panicking
`extern "C-unwind"` provider through the same `std::panic::catch_unwind` site
the rendered wrapper uses, and requires `SIGABRT`, so the "worker crash, not a
status" contract is measured rather than only documented. It does not link the
bridge rlib: this toolchain refuses to link a crate whose `crate-type` includes
`cdylib` into a `panic=abort` binary ("requires panic strategy `unwind`"), which
is a toolchain restriction, not a change in the contract; the whole file is 22
passed before U16 and 24 passed after it.

**Checked scalar/buffer channel.** The payload wrappers keep their ABI (a
payload return, abort on a bad word or a provider panic), and `interface.rs`
renders an additive checked form per descriptor:
`<symbol>__checked(words..., out: *mut u64) -> i64`. It declares the provider
`extern "C-unwind"`, decodes the words before the call (`STATUS_REFUSED` on a
word that does not decode, a negative buffer length, a null buffer with a
non-zero length, or a null out slot), catches a provider panic as
`STATUS_PANIC`, and writes the encoded payload only on success. A module opts
in by importing the checked symbol explicitly, so no existing module changes.
`cargo test --offline --test checked_adapters` → 1 passed: success writes the
payload, a boolean word `2` is `STATUS_REFUSED` with the out slot untouched, a
panicking provider is `STATUS_PANIC` with the out slot untouched, a null out is
refused, and a later call still succeeds. `--lib interface` → 31 passed with
the render assertion that the checked form uses `C-unwind`, the three statuses
and the out write, while the payload form keeps `extern "C"`.

**Not yet carried:** the checked scalar/buffer form is opt-in at the symbol
level, not selected by the compiler for an ordinary `rs_*` import, so a module
that keeps the payload import still aborts on a bad word or a provider panic.
`Result`/`Option` tags and owned-output cleanup after a failure are U15's next
steps, not carried here. The `panic=abort` crash is now
exercised by the child-process fixture above, and the module's convenience
`read` still collapses a refusal to `-1`, so a
provider result of `-1` is indistinguishable at the module level (the wrapper's
`out` slot and status word are not).

## U16 — scoped synchronous callbacks behind checked tokens (bounded start)

A callback crosses two boundaries at once: a native library may retain the
function pointer and its context, and the JIT code that issued them can be
retired. The bounded start makes the host able to refuse a call *before* it
jumps, and it removes the lock the old `__with` held across the callback.

**The registry.** `safestos/cranelift/src/callbacks.rs` adds
`CallbackRegistry`, `CallbackScope`, `CallbackToken`, `CallbackPolicy`,
`Lifetime` (`Call`/`Instance`), `ThreadAffinity` (`CallerThread`/`AnyThread`),
`Reentrancy` (`Forbidden`/`Nested`) and `ErrorPolicy`
(`StatusWord`/`RetireOnPanic`). `invoke` checks, in this order, a null token,
the generation retirement watermark, the token's generation, the scope's
closed flag, the token's slot, the registration thread and the reentrancy
depth. Only then does it copy the function pointer and context out, **drop the
mutex**, and call under `catch_unwind`. A `Call` token is one-shot: after its
first invocation it is retired, so a library that retained it and calls again
is refused by identity. `retire`, `close` and
`CallbackRegistry::retire_generations_through` are the explicit retirement
paths; dropping a scope closes it and clears its registrations.

**The wrapper.** `render_handle_wrapper_at` renders `__with` as: resolve the
handle, read the provider pointer, mint an **immutable claim** with
`borrow_to_words` and drop the table lock; open a `CallbackPolicy` with
`Lifetime::Call`, `ThreadAffinity::CallerThread`, `Reentrancy::Forbidden` and
`ErrorPolicy::StatusWord`; register the caller's callback with the provider
pointer as its context; `invoke` it; drop the scope; release the claim on every
path (including a scope-open failure). A nested `__with` on the same handle is
valid because immutable claims coexist and neither call holds the lock; a
`__borrow_mut`, `__write` or `__dispose` during the callback is refused by the
live claim. The callback declares `extern "C-unwind"` so a `panic=unwind`
provider can unwind into the catch; a `panic=abort` provider still aborts the
process (worker crash).

**Thread identity without TLS.** `std::thread::current()` inside a generated
cdylib registers a thread-local destructor in that image; after `dlclose` the
destructor points at unmapped code and the process faults at teardown (measured
as a SIGSEGV after the test body, only when `__with` had run). The registry
therefore compares `libc::pthread_self()` instead, which has no such
bookkeeping. On a non-unix host the key is `0`, so affinity is inert there; the
generated wrappers and fixtures are unix builds.

**Evidence.** Bridge `cargo test --offline --lib --no-default-features
--features cedar,tier1-rs callbacks` → 12 passed: direct-oracle argument/result
identity, one-shot `Call` token and reusable `Instance` token, a token from
another scope and the null token refused without a jump, a retired generation
refused before the jump, wrong-thread refused and `AnyThread` allowed, forbidden
reentrancy refused while the nested case succeeds (both prove no lock is held
across the callback), a panic is `Panicked` with `StatusWord` keeping the token
and `RetireOnPanic` retiring it, and `close` refuses every token and clears the
registrations. Umbrella `cargo test --offline --lib interface` → 31 passed,
including the render assertions that `__with` claims, registers through a scope,
drops the table lock before the scope opens and releases the claim after the
invoke. `cargo test --offline --test handles` → 24 passed, including
`a_callback_panic_is_a_status_word_and_the_value_survives` (a panicking
trampoline in the fixture library returns `STATUS_PANIC`, writes no out word,
leaves the value live and disposable, and the claim is released on the panic
path) and `a_callback_cannot_mutate_the_value_it_borrowed` (during the callback
`__borrow_mut` is refused by the live claim while a nested `__with` succeeds and
returns its own word; after the callback the same `__borrow_mut`/`__write`
succeed and the written value is read back). The panicking trampoline lives in
the fixture library rather than the test binary because a panic raised in one
Rust runtime and caught in another is a foreign exception Rust refuses to catch.

**Not yet carried.** This is the synchronous, call-scoped half of U16. No CPS
first-class callback value exists yet, so the only module-visible use is the
`__with` borrow; the `CallbackPolicy` is fixed by the generator rather than
described in the adapter document; the bridge can retire a provider generation,
but no host call site calls `retire_generations_through` on reload yet; retained
`Instance` callbacks, thread-hop callbacks in production and a per-descriptor
callback policy belong to the next card. The `panic=abort` fixture no longer
links the bridge rlib because this toolchain refuses to link a `cdylib` crate
type into a `panic=abort` binary, so it reproduces the catch site rather than
the rendered wrapper.

## U21 — rendering provider wrappers from a descriptor document (bounded start)

`umbrella --descriptor-document <file> --emit-wrappers <dir>` is a render-only
path: it reads the text `austral compile --emit-adapters` writes, parses the
`abi scalar`, `abi buffer` and `abi handle` blocks with the same parsers the
build path uses, renders each wrapper and writes one combined `mod wN { … }`
source to `<dir>/wrappers.rs`. It never starts cargo, the front-end or the JIT,
and it refuses a document with no adapter blocks, a malformed block, or a run
that also names modules (exit 2 for the grammar error, 1 for the operational
one). The CLI test writes a scalar document with a fake `cargo` on PATH and
requires exit 0 plus the rendered `rs_scale__adapter` wrapper and its native
signature, with the cargo marker absent; a comment-only document is refused with
`no adapter blocks`. `cargo test --offline --lib cli` → 17 passed and
`cargo test --offline --test cli` → 8 passed.

This is rendering, not building: it does not compile the generated source, link
a provider, or choose a provider generation. Those stay on the build path
(`adapters::build_at`, called by a module run).

**Retained instance callbacks and reload retirement (2026-11-25).** A `__with`
callback is call-scoped and one-shot. A provider that keeps the pointer needs an
instance scope, so every handle wrapper now renders three more exports beside
`<target>__with`: `<target>__with__retain(lo, hi, callback)` opens an
instance-lifetime scope at the crate's `PROVIDER_GENERATION`, checks the handle
through the same registry, registers the callback and keeps the checked token;
`<target>__with__invoke_retained(lo, hi, out)` is the host seam onto the same
trampoline (the token is reusable, and a caught panic is `STATUS_PANIC`); and
`<target>__with__retire_callbacks(generation)` closes every retained scope at or
below that generation and retires it in the registry. `load_for_jit` records each
mapped crate and, before loading the next generation, calls the previous
builds' own retirement hooks through their own image (`dlsym` is image-scoped:
every generation exports the same hook name). The hook is `dlsym`ed from the
crate the host already has, so no second symbol table is needed.

**The provider protocol.** A retained callback is useless to native code unless
the provider can invoke it, so the handle descriptor gained an optional
`subscribe <symbol>` line. When it is present, `__retain` also calls the
provider's subscription entry point with `(owner_ptr, dispatch, slot)`: `dispatch`
is `unsafe extern "C-unwind" fn(slot: *mut c_void, out: *mut i64) -> i64` and
`slot` is an opaque registry id, not a pointer into the wrapper's memory. The
provider stores the pair and calls `dispatch(slot, &mut value)` whenever the
event arrives. The wrapper crate stays mapped for the process, so the slot
stays valid; retirement closes the scope behind the id, and the trampoline
returns `STATUS_REFUSED` instead of jumping into replaced JIT code. A provider
that refuses the subscription (non-zero) makes `__retain` drop the entry, close
the scope and return refusal: no token survives a failed handoff. The line is
canonical text after `write` and before `params`; the compiler does not emit it
yet, so it is a host/descriptor extension that the parser and renderer share.

**Policy from the descriptor.** The handle descriptor's optional `callback`
line names `lifetime=call|instance`, `thread=caller|any`,
`reentrancy=forbidden|nested` and `error=status|retire`, in that canonical
order. Every field is optional and an absent one keeps the wrapper's previous
fixed value, so a descriptor written before the line renders exactly what it
used to. `lifetime` gates the retained API as a whole: `call` renders no
`__retain`/`__invoke_retained`/retirement hook, and a descriptor naming both
`lifetime=call` and `subscribe` is refused (a call-scoped callback cannot be
handed to a provider). `thread`, `reentrancy` and `error` feed both the
`__with` scope and the retained scope; `reentrancy` and `error` default
differently per wrapper (`forbidden`/`status` for `__with`, `nested`/`retire`
for a retained token) so an unset field changes nothing.

Evidence: `cargo test --offline --test callbacks_retained` → 3 passed. The first
builds two real wrapper generations against a provider cdylib, mints a handle in
generation one, retains a callback and invokes it twice through the host seam
(16 from the provider's value each time), loads generation two through
`load_for_jit`, and asserts the first build's retained token is now refused with
the out slot untouched while generation two retains and invokes its own. The
second is the provider protocol: the provider stores the dispatch/slot pair and
the *test calls the provider's own `fire` export*, which invokes the trampoline
it was handed (16 twice); after the reload the same provider call is refused
with the out slot untouched, and the second generation's subscription replaces
the pair and returns 3. A failure to retire would return 16 a third time; a
retirement that cleared all generations would also refuse generation two. The
third test is the policy: the canonical text keeps `thread=any
reentrancy=nested error=retire`, the rendered `__with` and retained scopes
carry `ThreadAffinity::AnyThread` / `Reentrancy::Nested` /
`ErrorPolicy::RetireOnPanic`, `lifetime=call` removes the retained API and its
retirement hook, `lifetime=call` with a `subscribe` symbol is refused, and at
run time a retained token opened on the test thread is refused (status 1, no
value) by a spawned thread under the default caller affinity and accepted
(16) under `thread=any`.

Limitations: `Reentrancy::Nested` and `RetireOnPanic` are the instance policy,
so a panicking retained callback retires that token. Affinity is checked
between the scope's opening thread and the invoking thread, so the `thread`
field changes behavior only for a retained token invoked later: a `__with`
scope is opened by the caller inside the same call. The compiler emits neither
the `callback` line nor a `subscribe` symbol yet (no `Handle_Ownership` pragma
for them), so a module compiled from source gets the defaults and cannot
request the provider protocol without the host editing its descriptor. A
`panic=abort` provider still aborts the worker, and a panic in a *different*
dylib's panic runtime cannot be caught (`Rust cannot catch foreign
exceptions`), so retention assumes the provider does not unwind.

**Result/Option tags (bounded start, 2026-11-25).** The scalar descriptor now
carries `status checked` plus `failure <tag>` / `success <tag>`, and the
generated `<symbol>__checked` returns the `Option` union pointer directly: tag
at offset 0, encoded payload at offset 8, which is the layout the `match`
lowering already reads. A word that does not decode, a provider panic, or an
unencodable payload returns the failure tag, so a refusal never reaches the
module as the payload word. The compiler half detects an `rs_*` import whose
declared result is the built-in `Option[T]` (cases `None` then `Some`, scalar
payload; tags from the union's own layout), emits the status lines in
`adapter_block`, and lowers the call to `<symbol>__checked` instead of the
payload symbol. The payload wrapper is still rendered, so a caller naming the
plain symbol is unchanged.

Evidence: `cargo test --offline --lib interface` → 32 passed, including
`a_checked_option_descriptor_renders_a_tagged_union_wrapper` (canonical
round-trip, distinct tags, the union allocation, the shared failure value and
the five refusals); `cargo test --offline --test checked_adapters` → 3 passed. Two of them measure
the wrapper in isolation (the provider and the rendered Option wrapper in one
cdylib: `Some` is tag 1 with the encoded payload at offset 8, a bad `Bool` word
and a provider panic are tag 0 with no payload, a false flag is still a
successful `Some`). The third is the live path:
`the_jit_calls_the_checked_wrapper_the_compiler_selected` runs the real
front-end over `umbrella/modules/CheckedOption.aum`, asserts the emitted
descriptor carries `status checked` with tags 0/1, builds and loads the
wrappers through `load_for_jit`, compiles the emitted CPS and executes it. The
module returns 11 only when the JIT's `match` reads tag 1 and payload 7 from
the union; a wrong tag, a payload-word return or an unselected checked wrapper
returns a different number. `dune build lib/ bin/ -j 4` under `nix develop
"git+file://$PWD#ocaml"` → exit 0.

**How the selection reaches the lowering (2026-11-25).** The typer admits
exactly the built-in `Option[T]` with an adapter-scalar payload:
`TypingPass.checked_option_result` resolves the `NamedType`, requires the
`Austral.Pervasive` declaration, and checks the `None` (no payload) / `Some`
(one slot) shape. It records `(failure tag, success tag, payload)` in
`lib/CheckedOptions.ml`, keyed by the external symbol. That indirection is
load-bearing: a foreign signature can name a union monomorph whose definition
was emitted in the importing module, so `Compiler_cps`'s own
`MUnionMonomorph` table does not hold that id — without the typer's record the
result looks like a non-scalar and the descriptor is refused. The CPS pass
uses the record to set the descriptor's `returns` (the mono form of the admitted
scalar) and registers a synthetic `None`/`Some` layout under the signature's id,
so the JIT's `match` compares a loaded tag against the union order rather than
the source order of the `when` clauses. The tags written to the descriptor are
read back from the layout the `match` lowering actually holds for that id — a
module that uses the union itself carries its own `MUnionMonomorph`, whose case
order is not always the declaration order the typer saw. With no local layout
the registered `None`/`Some` shape is that layout, so both paths agree with the
dispatch.

**Either and Result (two payload-bearing cases, 2026-11-25).** `Either[L, R]`
and `Result[T, E]` are the named two-case unions an `rs_*` result may declare
whose cases both hold a value, so a refusal is not an empty case. They take one
transport: the scalar descriptor grows four optional lines — `union Either`,
`failure-case Left`, `success-case Right`, `failure-payload Int64` (or `union
Result`, `failure-case Err`, `success-case Ok`) — and the wrapper's provider
writes `[tag, payload]` into an out pair (`fn target(params..., out: *mut
u64) -> i64`; status `0` means filled). The wrapper validates the tag (failure,
success, else refusal) and decodes/re-encodes the payload at the case's
declared scalar type; a non-zero status, a panic, an unknown tag or an
undecodable payload all return the failure tag with payload `0`. A
`failure-payload` line is what selects this form, so an `Option` descriptor is
byte-identical to before.

The admitted set is the typer's `checked_payload_unions` table
(`Either`/`Left`/`Right`, `Result`/`Err`/`Ok`), and `Result` is now a built-in
of `Austral.Pervasive` (`Ok` is argument 0, `Err` argument 1). The tags are the
ones the JIT reads, not the ones the declaration suggests. `EitherImport.aum`'s
own `Either[Int64, Int64]` monomorph lists `Right` before `Left`, so the
descriptor emits `failure 1` / `success 0`, and the provider must write tag 0
for `Right` and 1 for `Left` (the descriptor's tags, not Rust's enum order).
`ResultImport.aum`'s `Result` monomorph lists `Err` before `Ok`, so its
descriptor emits `failure 0` / `success 1`; the Result test reads those tags
back from the descriptor and writes the provider against them, which is the tag
test for that shape.

**Owned-output cleanup (2026-11-25).** The checked wrapper allocates the union
it returns (`Box::into_raw`), so it also exports `<symbol>__free`. The `match`
lowering copies the tag and every slot a case binds into locals, calls the free
symbol, and only then dispatches on the copies: a `return` inside a case cannot
skip the release, and no case body reads freed memory. The free function
recognises the wrapper's own static refusal value (and null) and does not free
it; every answered call is a one-allocation Box. The transport admits scalar
payloads, so a copied word owns nothing and needs no release of its own.
The conversion tables are sequenced explicitly in `MBlock` (`let first = … in
let second = …`) because OCaml's right-to-left argument evaluation would
otherwise register a temp's ownership after the `match` that consumes it.

Evidence: `cargo test --offline --test either_adapters` → 4 passed. The
descriptor test measures both case names, the tags and the round-trip, refuses
a partial union block, and requires the `__free` export; the wrapper test
measures both cases and every refusal through the real ABI (wrong payload,
unknown tag, non-zero status, a `Bool` word of 2) plus freeing an answered
union, freeing the shared refusal value, and a null release; the two live tests
run the real front-end over `umbrella/modules/EitherImport.aum` and
`ResultImport.aum`, assert the emitted descriptor carries the union, both case
names and `failure-payload Int64` with the layout's tags (Either 1/0, Result
0/1), build and load the wrapper, require the CPS to name
`<symbol>__free`, compile it, and execute `computeRight` → 11 / `computeLeft` →
22 and `computeOk` → 11 / `computeErr` → 22. `cargo test --offline --lib --test
checked_adapters --test callbacks_retained` → 110/3/3 passed; `dune build lib/
bin/ -j 4` under `nix develop "git+file://$PWD#ocaml"` → exit 0. Both fixtures
are in `scripts/cps-corpus` (marked `@cps`, because the checked call is
JIT-only and the C backend refuses the generated symbol): `scripts/isa-census
--corpus` → 14 modules, 992 opcode reads, 23 distinct, 23 declared, 0
uncovered, and the empty `UNREACHABLE_OPCODES` pin still holds.

Limitations: a `Result`/`Either` whose error case is empty is not admitted
(the `Option` arm is name-based). The release is emitted for the `match` that
consumes the call — directly, or through the local/temp the value is bound to
(the table follows the name until the match, and a rebinding clears it). A
result that is never matched, or whose name is rebound first, is not released —
a leak, not a wrong free, and the linear type system makes the unconsumed value
a type error in the fixtures' shape. A provider in a separate
dylib cannot have its panic caught by the wrapper's `catch_unwind` (`Rust
cannot catch foreign exceptions`: each Rust dylib carries its own panic
runtime), so the live test's provider does not panic; the
caught-panic-to-failure mapping is measured in the same-crate wrapper tests. A
checked Option import whose union monomorph is missing locally is resolved by
shape; the compiler refuses rather than guesses when more than one
`Option`-shaped layout exists.
