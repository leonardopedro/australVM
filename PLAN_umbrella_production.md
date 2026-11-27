# Umbrella / cap-std: debugging, feature completion and production test plan

**Status: in progress, 2026-11-25.** The §8 table records which cards have a
measured implementation and the evidence for it; cards with no row there
(U19–U32) are still pending. Nothing in this document is a production-readiness
claim: U30's release gate has not been run, and the host currently lacks the
MLIR toolchain its GPU checks need.

This plan turns the existing mixed Austral/Rust prototype into a usable,
testable compiler/host. Testing is structured for a cheaper LLM: one bounded
patch, a concrete failure, a small relevant case set, and an explicit oracle.
The deliverables of this planning turn are this document, a
[source review](docs/UMBRELLA_PRODUCTION_REVIEW.md), and a
[test matrix](docs/umbrella/TEST_MATRIX.md). They do not implement the tests or
establish production readiness.

## 1. Production scope

The product is a lightweight Austral CPS/Cranelift compiler plus an umbrella
orchestrator that builds trusted Rust adapters, resolves their typed exports,
and runs mixed modules with explicit capabilities. Existing Rust libraries
should be reusable through generated, Rust-checked wrappers, including useful
owned types and methods. Users need a project manifest, repeatable builds,
clear diagnostics, installation, and real fs/net capability enforcement.

“Full featured” means the supported surface below is complete and tested. It
does not mean arbitrary Rust layouts, macros, lifetimes, trait objects, unsafe
APIs or arbitrary native binaries become safe to import automatically.

| Area | Current implementation | Production target |
|---|---|---|
| Build | One arbitrary crate, first shared artifact | Package/workspace/target/features selected explicitly; locked build inputs; staged build/check/run |
| Interop | rs symbol name plus scalar/pointer annotation | Versioned qualified export descriptors and generated checked adapters |
| Calls | One named no-arg entry cast to i64 function | Verified entry ABI, typed status, multiple calls and declared argument shapes |
| Values | Demo pointer+i64 and scalar i64 | Checked scalars, bytes/text, Option/Result, opaque owned values, methods and selected concrete generics |
| Ownership | Annotation vocabulary; raw pointers are free types | Linear local wrappers, allocator-specific destroy thunks, scoped borrows and invalidation |
| Callbacks | No umbrella contract | Scoped synchronous callbacks with context, lifetime and reentrancy rules |
| Filesystem | Separate CapFs helper | Host-created rooted handles with enforced operation rights, ownership and quotas |
| Network | Manifest strings | Explicit bounded dial capability, actual target checks, TLS and timeout/error contract |
| Modules | Multiple CLI arguments, no validated bundle contract | Explicit mixed module graph, qualified names, dependency/link validation |
| Runtime | One native process, global loader/auth state | Instance/generation ownership, supervised workers, deterministic failure status and cleanup |
| Isolation | cap-std discipline for trusted Rust | Clearly separate trusted and verified isolated-native profiles; untrusted mode disabled until its gate passes |
| Tooling | No umbrella test suite | Test lanes, JSON diagnostics, traceable artifacts, fuzz/sanitizer controls, relocatable packaging |

Initial native release platforms: Linux aarch64 and x86_64, each tested natively.
The current loader does not implement Windows support just because it recognizes
`.dll`. Cross-building an adapter may be supported separately; cross-running
that adapter in a host-native JIT must refuse. macOS/Windows/32-bit require a
separate platform matrix before they are advertised.

The default production route uses stable Rust with explicit adapters. U32 is an
optional pinned compiler-oracle extension for direct-import ergonomics. It must
not become an accidental prerequisite for ordinary projects. The Austral compiler
continues to use Cranelift JIT; Rust compilation remains an umbrella-owned
subprocess or optional external compiler service.

## 2. Architectural decisions to test

### 2.1 Separate build authority, native admission and invocation

Use a versioned project/build description, provisionally `Umbrella.toml`, for
Rust packages, features, selected exports, compiler identity and build trust.
Preserve `module.toml`'s existing invocation/grant vocabulary; optional versioned
sections can add strict profile requirements and resource bindings. Do not
silently reinterpret legacy fs strings as granting every operation.

Validation order:

```text
parse options / project / policy -> validate tools / target / trusted build
  -> Cargo metadata and locked build -> exact artifact + export descriptor
  -> compile-only Austral artifact -> validate bundle / grants / typed entries
  -> admit pinned native provider in selected worker/isolation profile
  -> inject host-created capabilities -> JIT -> checked invocation -> cleanup
```

Cargo build scripts and procedural macros are native execution. dlopen can run
initializers. A denied target call only establishes that the call was blocked;
it cannot undo prior native initialization. Tests must track all three stages.
If a deployment requires zero untrusted execution, its build and load phases
need the corresponding isolation, not just a runtime call gate.

### 2.2 Versioned adapter ABI, not guessed native Rust ABI

Introduce an import/export description containing package identity/version,
qualified item and concrete type arguments, exported symbol, ABI version,
logical argument/result types, machine transport representation, ownership,
buffer extent and mutability, destructor, panic/error behavior and required
capabilities. Bind it to artifact and toolchain/target identity.

Initially permit only proven i64/borrowed-byte-buffer forms. Reject unsupported
frontend FFI types before native invocation. Expand through generated wrappers:
ordinary Rust functions/methods/generics are called by Rust code typechecked by
rustc; the JIT calls only the stable adapter boundary. An all-word transport can
encode booleans, handles and float bits explicitly, but that needs tested
marshal/unmarshal operations and range checks. It is not an excuse to invoke a
native float-register ABI as though it were i64. U11 decides and tests the exact
representation; unsupported cases remain rejected until that work passes.

Do not pass a native Rust String, Vec, trait-object fat pointer, enum or Rust
repr struct by a guessed layout. Opaque handles and explicit tagged/results
buffers cover these APIs without exposing unstable layouts. Direct C-layout
values are a later supported adapter form with target/layout checks.

Generated descriptor, Rust wrapper and Austral declaration should come from one
small schema, with independent golden vectors and direct Rust call tests. The
schema is part of trusted interface specification; generating both sides does
not itself prove their correspondence. A self-described arbitrary unsafe plugin
is not authenticated merely because it presents a descriptor.

### 2.3 Ownership, capability identity and native lifetime

Rust-owned objects stay in a per-instance typed/generational registry. Austral
gets abstract linear wrappers around host handles, with private constructors.
Raw `Address` and integer tokens are not ownership proofs. Each object records
its provider generation and allocator-specific destructor. Never unload native
code or reset the JIT while a dependent call/object/callback is live.

Borrowed parameters are valid only during a declared scope. Returned views must
be copied or hold a checked owner/version relationship; reject use after owner
mutation or destruction. Mutable aliasing must obey Rust's rules whenever a
wrapper creates Rust references. Valen-style group aliases do not authorize
simultaneous Rust `&mut` references. Reentrant callbacks must respect the same
exclusivity, without retaining a global lock across user code.

The trusted host alone opens ambient directory/network capabilities. Rust
libraries receive those handles or wrappers that use them. `cap-std` does not
automatically constrain a third-party library using ambient `std::fs`/sockets.
Classify an adapter as capability-aware, trusted-ambient, or isolated, and refuse
profiles whose requirements its implementation cannot satisfy.

### 2.4 Runtime limits and failure semantics

For the first production runtime, one loaded generation per supervised worker is
acceptable and simplifies containment of legacy global symbols/authorization.
A process is a lifecycle boundary, not automatically a security sandbox.
In-process trusted operation can remain an explicit lower-isolation profile.

Use structured outcomes: parse/build/compile/load/authorize/invoke failure,
domain result, panic, trap/signal, timeout and cancelled/unknown effect status.
Every i64 bit pattern remains a possible domain value. Catch unwinding panics
inside generated C wrappers; a panic at non-unwinding C ABI, abort or segfault
requires worker-failure handling. Hard deadlines require a killable worker;
timeouts on a host thread cannot safely preempt arbitrary native code.

Compile-only commands never initialize or execute the target library. Run mode
must use the validated artifact and capabilities. A cache stores compilation
products, never an authorization verdict. Policy and capability checks apply
under the specified current instance/generation at use time.

## 3. Execution discipline for the cheaper LLM

Read applicable `AGENTS.md`, this plan's current task, its review finding and
its matrix rows. Preserve the distributed-plan documents and all unrelated
dirty files. Read Valen/rust as references; keep both reference repositories
unchanged. Do not build the Rust fork while executing the stable-route tasks.

One patch should normally touch at most three production files plus relevant
tests. Split a task into its numbered substeps if necessary. First reproduce
the named symptom with a small fixture, then fix it, then run the targeted
test plus its relevant existing suite. Use process isolation for malformed
native calls, crashes and untrusted fixtures. Never explore invalid pointers
with the agent's own process or delete developer directories as test setup.

After each patch record: task/case IDs, source revisions and dirty inputs,
toolchain/features/target, exact command, exit/signal/timeout, observed oracle,
evidence file and gaps. Distinguish a mock-tool unit test from a real pipeline
test. Do not regenerate expected outputs simply to make a failure disappear.
Fix the named failure before broadening test scope.

Every test must have at least one success path and a discriminating failure
oracle where applicable. No unconditional PASS output, zero-case success,
blanket ignore, skip-as-pass, uncontrolled public network, or random sleep as
an orchestration barrier. Use readiness signals and bounded waits. A negative
test must fail for the intended reason and verify absence of the protected
effect. Record minimized seeds for randomized failures.

Do not require repeated owner permission for routine implementation choices
within an execution request. Cross-language ownership changes, broader unsafe
ABI support and new compiler-private dependencies need a concrete design and
failing witness in the execution record before expanding the task. Existing
held-out proof statement approval remains the owner's separate workflow; this
test plan does not authorize weakening proof statements.

## 4. Test lanes and commands

U00 creates `umbrella/tests/` and **new** `scripts/test-umbrella`. That runner
and the test target names below are proposed, not currently available. Existing
tests listed separately can be run immediately in a working environment.

| Lane | Contents | Dependencies / execution policy |
|---|---|---|
| `cli` | Real binary parsing and mock-tool stage boundaries | No compiler/network; every case must execute |
| `build` | Cargo message selection, tool supervision and private staging | Local fixture crates only; cache tools once |
| `abi` | Descriptor, values, handles, ownership, real cdylib calls | Built fixture providers; native crash cases in children |
| `capability` | Actual rooted fs and loopback network access | capstd feature required; unique roots/ports/certs |
| `e2e` | Real compiler → bundle → native adapter → JIT → capability IO | Fresh bridge/compiler identity; no mocks at the core seam |
| `robustness` | Worker crashes/cancellation, fuzz corpus and mutation controls | Fixed seeds and bounded runtime; separate sanitizer sublane |
| `package` | Relocation, target/version checks, locked/rebuilt artifacts | Native CI per supported target |
| `isolated` | Build and native escape controls | Tested isolation backend; required only for untrusted profile |
| `oracle` | Optional fork/private-API import extension | Exact pinned toolchain; independent feature gate |

Suggested future runner contract:

```bash
# Proposed after U00; run from australVM root.
scripts/test-umbrella --lane cli --case CLI02 --strict --json
scripts/test-umbrella --lane e2e --strict --json
scripts/test-umbrella --lane robustness --seed 17 --strict --json
```

The runner must report build setup time separately from case runtime, fail on an
unknown/empty selector, validate required-case inventory, and return nonzero on
any failure or required gap. Hard per-case watchdogs are test guards. U00 records
measured normal durations; U30 establishes product performance budgets from
those measurements instead of inventing universal throughput targets.

Existing command templates, from the repository root:

```bash
# All dune commands and make targets needing dune run in nix develop.
nix develop "git+file://$PWD#ocaml" -c make bridge
nix develop "git+file://$PWD#ocaml" -c make all
nix develop "git+file://$PWD#ocaml" -c make test

# Existing focused Rust suites; rebuild release bridge before OCaml integration.
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs --test tier1_extern_rust
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs,capstd --test capstd_tests
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path safestos/cranelift/Cargo.toml --features test-stubs --test static_seam
nix develop "git+file://$PWD#ocaml" -c cargo test --locked --offline --manifest-path umbrella/modules/rscrate/Cargo.toml

# Existing negative compiler fixtures; runner supports a suite substring.
nix develop "git+file://$PWD#ocaml" -c bash -c 'export LD_LIBRARY_PATH="$PWD/safestos/cranelift/target/release"; python3 test-programs/runner.py 019-extern-rust'

python3 scripts/check-links
git diff --check
```

U00 verifies actual prerequisites. The source checkout's default bridge depends
on unfer and its siblings; disabling a feature does not prove those paths vanish
from Cargo resolution. A missing offline crate/prover/backend is a named gap.
Provision dependencies separately under the environment's rules; never loosen a
test or silently update a lockfile to hide a missing dependency. `#ocaml` can
build without the failing prover stack; required liquid proof tests need an
actual prover environment. Follow unfer's FFI lock instructions if those tests
become relevant; use process-local fixtures to avoid unnecessary global FFI state.

## 5. Release milestones and task dependencies

| Milestone | Required result | Tasks |
|---|---|---|
| R0 — honest baseline | Real evidence, explicit unsupported cases, no false demo pass | U00–U04 |
| R1 — safe orchestration | Exact artifacts, compile-only output, scoped resolution/auth and checked descriptors | U05–U10 |
| R2 — useful library interop | Scalars/bytes/text, owned values/borrows, Result/panic, scoped callbacks and mixed graph | U11–U18 |
| R3 — real capability modules | Enforced fs/net rights, useful Rust adapters, real demo and worker controls | U19–U23 |
| R4 — production trusted-native profile | Fuzz/controls, packaging, two native platforms, CI, operational and resource acceptance | U24–U25, U27–U30 |
| R4-isolated | Separately validated untrusted build/native execution | U26 plus R4 |
| Extensions | Optional async host and direct-import compiler oracle | U31/U32, each with its own gate |

All tasks below have backward-only dependencies. Start with the lowest ready
task; no delegation is required. U23 can be brought forward after its dependencies
pass because it makes later native crash testing cheaper and safer.

The earlier [distributed plan](PLAN_distributed_security.md) shares work with
this plan: D08 ↔ U09/U18 for instance authority; D09 ↔ U08/U19–U22 for the Rust
adapter; D17 ↔ U13/U14 for linear handles; D18 ↔ U07/U28 for required checking;
D24 ↔ U10/U25 for artifact identity. Implement each shared seam once and link
the same evidence from both ledgers. This plan does not require implementing
distributed consensus before a local capability module can work.

## 6. Debugging and implementation task cards

Paths marked **new** are proposed. Matrix IDs are acceptance requirements; use
the linked matrix for the exact stimulus and observable oracle.

### U00 — Build an honest baseline and cheap test harness

**Depends:** none. **Finding:** UR14. **Cases:** HAR01/HAR03, BLD08.
**Files:** **new** `umbrella/tests/support/`, `scripts/test-umbrella`,
`docs/umbrella/EXECUTION.md`; existing Cargo manifests read first.

1. Record compiler/bridge/umbrella identities, lock hashes, available features,
   target and native linker. Inventory existing tests and their actual assertions.
2. Create a minimal runner with case/lane selectors, timeouts, unique scratch
   directories, structured evidence and required-case accounting. Fake tools
   report exact argv/environment and write invocation markers.
3. Run existing demo-crate, tier1, capstd and static-seam tests once where tools
   exist; record baseline failures/gaps without fixing unrelated code.

**Accept:** unknown/empty selector fails; required gap fails strict mode; a
failing child cannot be reported as a passing case. Keep the harness stdlib-only
where practical and test it with one small positive and one negative case.

### U01 — Remove false capability demo success

**Depends:** U00. **Finding:** UR01. **Cases:** HAR02.
**Files:** `examples/modules/rust_kv/run_demo.sh`, its fixture directory and docs.

1. Add a test proving the current script reports success without any capability
   operation. Do not run its optional heavy build merely to test printed prose.
2. Until real execution exists, make the demo report unsupported/not implemented
   with a non-success strict result. Retain existing real unit-test references
   without presenting them as demo execution.
3. Add a guard that replacing the executable operation with `echo PASS` fails.
   U22 later replaces the gap with the full pipeline.

**Accept:** missing implementation cannot produce ALL TESTS PASSED; output says
exactly which path remains uncovered. Do not invent a fake Rust implementation.

### U02 — Define strict profiles and perform policy preflight

**Depends:** U00. **Findings:** UR01/UR06. **Cases:** AUT01, BLD06, RUN06.
**Files:** **new** `umbrella/src/project.rs`, `umbrella/src/policy.rs`; main wiring.

1. Add versioned project schema and trust/profile rules. Validate manifest,
   selected operations and feature availability before build/load. Unknown strict
   fields and malformed grants fail with named diagnostics.
2. Separate permission to build native source, permission to load a provider and
   permission for a module to invoke it. Missing call grant alone need not block
   an explicitly authorized build, but output must state that scope accurately.
3. Use build-script and library-initializer markers to prove stage ordering.
   Default untrusted profile remains unavailable pending U26.

**Accept:** a configuration that authorizes no build/load reaches neither marker;
native call denial never claims previous build/initialization was contained.

### U03 — Fix CLI grammar, diagnostics and exit contracts

**Depends:** U00. **Finding:** UR12. **Cases:** CLI01–CLI06.
**Files:** **new** `umbrella/src/cli.rs`, `umbrella/tests/cli.rs`; main wiring.

1. Parse OS-native arguments without lossy path conversion. Support exact options,
   `--key=value`, `--`, help/version and documented repeated-option behavior.
2. Reject prefix lookalikes, empty entries/paths, missing values and accidental
   option consumption. Define exit 2 for usage errors and exit 1 for operational
   failure; domain return values do not become process status accidentally.
3. Add JSON event mode with separate human diagnostics on stderr. Keep the old
   successful demo invocation as a compatibility fixture.

**Accept:** real binary tests reproduce and repair every measured parser defect;
no malformed invocation spawns Cargo or reads/loads a target library.

### U04 — Close unchecked ABI and entry invocation immediately

**Depends:** U00/U02. **Findings:** UR02/UR07. **Cases:** ABI01/ABI02, BLD05.
**Files:** `lib/TypingPass.ml`, `safestos/cranelift/src/cps.rs`, umbrella entry helper;
new compiler/bridge negative fixtures in separate patches.

1. Record the actual baseline transport: host-native 64-bit word arguments and
   i64 result. Strict interop rejects types not represented correctly by it.
2. Require an exact explicit exported entry with the declared signature before
   any pointer invocation. Empty, missing, wrong arity and unsupported return
   type refuse. Remove arbitrary-name fallback from the strict path.
3. Test all i64 boundary values independently of errors. Check target architecture
   before admission. Preserve legacy APIs behind their documented behavior if
   needed; never use them unchecked in the new strict profile.

**Accept:** mismatched-signature fixtures cannot reach the invocation marker.
Run affected bridge tests and compiler fixtures; rebuild release bridge before
OCaml integration. U11 expands the currently rejected scalar cases safely.

### U05 — Select exact Cargo artifacts and make builds explicit

**Depends:** U02/U03. **Finding:** UR04/UR13. **Cases:** BLD01–BLD05, BLD09, PKG07.
**Files:** **new** `umbrella/src/cargo.rs`, artifact-stream tests and local fixture workspace.

1. Use Cargo metadata for selected package/target identity and parse streamed
   Cargo JSON for that exact compiler-artifact. Check crate type, profile, target,
   fresh/current build and build-finished outcome; reject zero/multiple matches.
2. Expose explicit package, target directory, feature, profile, locked/offline
   policy. Preserve Cargo argument boundaries; never construct a shell command.
   Record/restrict build-affecting Cargo config, rustc/linker flags and wrappers.
3. Fixture workspace contains a proc-macro shared artifact, a transitive library,
   two cdylib targets and custom target output. Select deterministically and
   reject foreign-native execution before load.

**Accept:** the dependency's shared artifact can never win selection. Lockfiles
remain unchanged under locked builds, and requested features match evidence.

### U06 — Bound child processes and use private staging

**Depends:** U03. **Finding:** UR09. **Cases:** BLD07, CPS02, RUN03/RUN08, PKG07.
**Files:** **new** `umbrella/src/process.rs`, `umbrella/src/staging.rs`, helper-child tests.

1. Create a private per-build directory using a well-reviewed temp-directory
   facility. Refuse unexpected pre-existing outputs; cleanup on every exit with
   an explicit retain-on-debug option. Keep user paths outside cleanup ownership.
2. Stream and bound stdout/stderr without pipe deadlock; preserve phase-specific
   diagnostics. Add cancellation/deadline controls and termination/reaping of
   owned child process groups. Handle child exits and Unix signals distinctly.
   Specify an environment allowlist/override policy for tools and runtime workers,
   including PATH, compiler wrappers and native loader variables.
3. Test noisy child, hanging child, grandchild, symlink output, same project name
   in concurrent invocations and failed cleanup. Use readiness events, not sleeps.

**Accept:** no outside sentinel modification, unbounded buffering, stale output
reuse or orphaned owned child after the test. Deadlines remain parent-controlled.

### U07 — Add a fail-closed compile-only CPS artifact contract

**Depends:** U04/U06. **Finding:** UR08. **Cases:** CPS01/CPS02/CPS05–CPS07.
**Files:** **new** `umbrella/src/frontend.rs`; `lib/Compiler.ml` and CLI/plugin seams
only for a minimal reusable emit-only API, with compiler tests.

1. Emit CPS without initializing the target native provider or invoking target
   code. Return structured output identifying modules, typed exports, hashes and
   required verification outcomes. Do not depend on advisory JIT-to-C fallback.
2. Require fresh complete output in private staging. Detect missing output,
   unsupported lowering and required-check failure despite child exit 0.
3. Remove `--allow-all` from the strict compile artifact's security argument.
   Thread liquid-required mode to the actual compiler; optional missing tools
   remain explicit gaps rather than implied success.

**Accept:** bad JIT lowering never becomes a successful runnable artifact; a
compile-only effect fixture creates no runtime or initializer marker. Existing
CLI behavior can coexist if the strict artifact API is additive and explicit.

### U08 — Bind providers explicitly and own loaded-library lifetimes

**Depends:** U04/U05. **Findings:** UR05. **Cases:** LNK01–LNK04, AUT05.
**Files:** **new** `umbrella/src/provider.rs`; bridge host resolver seam and tests.

1. Resolve declared exports from the exact admitted library and register the
   selected addresses explicitly; use local symbol visibility where feasible.
   Reject duplicate providers and unexpected reserved/native imports.
2. Wrap library ownership so dependent JIT instances retain it. Do not call
   dlclose while any generated pointer/handle/callback can use the library.
3. Test load-order reversal, preloaded conflicting exports, static registry
   collisions and registration after initialization. A fresh worker may contain
   legacy process-global state; an in-process multi-provider profile needs an
   instance-bound resolver rather than a mutable global table.

**Accept:** the granted provider identity, ABI and address correspond. Existing
symbol registration resolves imports but never grants permission to call them.

### U09 — Scope authorization and injected capabilities per instance

**Depends:** U02/U08. **Finding:** UR10. **Cases:** AUT02–AUT04, LNK03.
**Files:** umbrella instance context; `safestos/cranelift/src/{auth,module,cps}.rs`
through separate small patches. Shared work: distributed D08.

1. Bind module/user/instance identity, effective grants and policy generation.
   Avoid inherited merged grants when reloading the same module name.
2. Keep compile admission checks and enforce capability operation authorization
   through the runtime context. Refuse cross-instance handles, forged module
   names and undeclared providers. Policy changes affect the promised boundary.
3. Test A/B modules, failed load preserving old context, same-name policy
   narrowing and concurrent calls. If isolation relies on one worker per
   generation, enforce and test that invariant.

**Accept:** A cannot borrow B's grants; narrowing does not retain old permissions;
UK-4001 remains stable for the corresponding denied calls.

### U10 — Generate a versioned checked adapter contract

**Depends:** U04/U05/U07/U08. **Finding:** UR02/UR05. **Cases:** ABI03/ABI06, API03.
**Files:** **new** `umbrella/src/interface.rs`, `umbrella/src/generate.rs`, golden adapters.

1. Freeze the minimal export schema from §2.2. Generate Rust wrappers, Austral
   declarations and a descriptor; rustc typechecks actual qualified library calls.
2. Version machine transport, ownership and status separately from business API.
   Verify expected descriptor/provider digest before native admission/invocation.
   An untrusted library's claim about its own unsafe behavior remains untrusted.
3. Reject unsupported lifetimes/variadics/layouts with actionable errors. Add
   independent ABI fixtures so one generator bug cannot make both sides agree
   on the same wrong answer.

**Accept:** signature/provider/schema mutations refuse before native call; golden
descriptor roundtrip is deterministic. No arbitrary native Rust layout crosses.

### U11 — Complete scalar conversions and target calling conventions

**Depends:** U10. **Cases:** ABI04/ABI05. **Files:** adapter generator, compiler
boundary conversion/CPS codec where required, direct-call fixture library.

1. Add signed/unsigned widths, bool, unit and f32/f64 through an explicitly
   defined word/buffer encoding or correctly typed trampoline. Validate extension,
   narrowing and invalid tags. Do not convert floats through numeric i64 casts.
2. Verify actual compiler-produced values and return values against independently
   compiled Rust calls, including mixed argument classes, noncommutative values,
   -0, NaN policy and infinity. Bit-preserving claims compare bits.
3. Any new CPS operation updates `Cps_codec`, ISA registry/corpus and evidence
   gaps. Reject still-unsupported lowering, even if the annotation typechecks.

**Accept:** accepted scalar signatures roundtrip and compute correctly; unsupported
ones cannot reach native code. U27 repeats target-dependent tests natively.

### U12 — Implement bounded bytes, text and caller-owned output protocols

**Depends:** U10/U11. **Finding:** UR03. **Cases:** OWN01/OWN02.
**Files:** generated adapters, **new** managed buffer helper and safe Austral wrappers.

1. Specify empty/null conventions, pointer provenance contract, length/capacity,
   extent/mutability and output initialization. Checked arithmetic precedes
   allocation or native slice creation. A non-null pointer alone is insufficient.
2. Distinguish byte strings from validated UTF-8; preserve embedded NUL. Offer
   probe-size/fill or owned result handles without cross-allocator free.
3. Tests compare binary and text edge cases to direct Rust results and verify
   error paths do not expose uninitialized output or retain borrowed pointers.

**Accept:** valid allocations and exact lengths succeed; malformed length/tag
cases refuse before dereference. Faulty pointers are tested only in child tools.

### U13 — Add linear owned Rust values and exact destruction

**Depends:** U10/U12. **Finding:** UR03. **Cases:** OWN03–OWN05/OWN08/OWN09, LNK05.
**Files:** **new** typed generational handle registry, generator, abstract Austral types.

1. Keep Rust allocations in their originating provider; export constructor,
   move/use and destroy operations through host-managed handles. Make Austral
   wrappers Linear with private construction and explicit dispose paths.
2. Record object type, owning instance, provider generation, destructor and live
   state. Handle stale slots/generation overflow and reject foreign/wrong-type
   handles. Avoid raw integer APIs in safe module interfaces.
3. Count Drop and allocation/free independently for success, early return, error
   and moved-object paths. Do not call a destructor twice after it panics.

**Accept:** compile-negative move/copy cases fail and runtime stale/foreign cases
refuse. Each successful owned allocation has exactly one correct destruction.

### U14 — Enforce borrowed-view validity and Rust aliasing

**Depends:** U13. **Finding:** UR03. **Cases:** OWN06/OWN07.
**Files:** borrow-scope/invalidation helper, generated wrappers, compiler-negative fixtures.

1. Define immutable/mutable borrow operations over owned handles. Track owner
   lifetime/version and reject mutation/drop while a protected borrow is live,
   or invalidate/copy views under the documented API. Prefer lexical scopes.
2. Adapt Valen's Chest/Gem scenario: obtain child view, replace boxed child, then
   use old view. Reject at typecheck or runtime boundary before touching freed
   memory. Include nested borrowing and disjoint valid operations.
3. Test reentrant overlapping mutation and group-borrow interaction. Never create
   simultaneous Rust mutable references from copied handles or native aliases.

**Accept:** stale child access and conflicting borrows are detected; valid scoped
borrows work. Record which guarantee is static and which is runtime-checked.

### U15 — Handle Result/Option, panic and all execution failures explicitly

**Depends:** U10/U13. **Finding:** UR11. **Cases:** ABI01/ABI06/ABI08, OWN08, CLI06.
**Files:** wrapper status ABI, umbrella invocation/result helper, real cdylib fault fixture.

1. Separate host status from value payload. Add checked tags and clear error
   lifetime rules; reject invalid tags. Preserve legitimate -1/i64::MIN results.
2. Catch unwinding panics inside each generated C wrapper before crossing ABI;
   define owned-input/output cleanup state on failure. Abort panic profiles are
   handled as worker crashes, never passed off as catchable exceptions.
3. Test real dynamic-library panic, domain Err, None, invalid status, stale error
   state, trap and abort in children. Output no RESULT success after host failure.

**Accept:** every failure has a distinct tested stage/status; real i64 edge values
remain successful domain results. A plain Rust function panic test is insufficient.

### U16 — Add scoped synchronous callbacks safely

**Depends:** U14/U15. **Cases:** API04–API07, OWN07.
**Files:** callback registry/trampolines, generator, callback lifecycle fixtures.

1. Callback descriptors include lifetime, owning instance/JIT generation, thread
   affinity, allowed reentrancy and error policy. Do not expose unowned raw JIT
   pointers to libraries that may store them indefinitely.
2. Implement call-scoped synchronous callbacks first, with explicit unregister
   and checked tokens. Use lifetime-stable host trampolines; a retained callback
   must reach a token check before any retired JIT code or freed context access.
   Reject callbacks after scope/generation retirement.
3. Test callback panic, recursive mutation, forbidden thread hop, nested valid
   callback and cleanup. Avoid locks held across arbitrary callback code.

**Accept:** callback arguments/results match the direct oracle; invalid lifetime
and aliasing cases refuse before a jump or Rust reference construction.

### U17 — Build and link a mixed module/workspace graph

**Depends:** U05/U07/U10. **Finding:** UR08. **Cases:** CPS03/CPS04, API02.
**Files:** project graph, frontend bundle API, local multi-package/module fixtures.

1. Resolve interface/body pairs, Rust package targets and selected adapter
   exports into an explicit dependency graph. Topologically build/link supported
   graphs and report unresolved/cyclic/ambiguous dependencies.
2. Produce a bundle containing every required CPS module, not the last writer's
   file. Preserve qualified identities through foreign mapping and JIT import
   resolution; test duplicate short names and separate versions of one crate.
3. Compare real cross-module values with a direct Rust/reference caller; test
   linking library modules without accidentally invoking their functions.

**Accept:** two libraries plus one client call the intended providers regardless
of source argument order where the project graph permits it.

### U18 — Make cache invalidation and generation replacement correct

**Depends:** U08/U09/U13/U17. **Findings:** UR05/UR10/UR13.
**Cases:** RUN07/RUN08, LNK04–LNK06, AUT04, OWN09, API05.
**Files:** build cache/instance lifecycle, provider registry, reload tests.

1. Cache key covers source/import graph, dependencies, generator/schema,
   toolchain/target/profile/features and ABI-relevant settings. Re-evaluate policy
   on use. Test edits to each input and a warm rebuild without edits.
2. Define replacement as a new generation. Drain/reject outstanding handles and
   callbacks explicitly; never dlclose/reset code while dependent work is live.
   A restart-based upgrade is acceptable before in-process hot replacement.
3. Narrowing grants cannot retain merged permissions; failed replacement leaves
   the old instance intact. Repeated generations have measured cleanup behavior.

**Accept:** cold and warm outputs agree; edited inputs rebuild the right artifacts;
old pointers/handles cannot resolve into a new object's storage.

### U19 — Enforce actual filesystem capabilities

**Depends:** U09/U13. **Finding:** UR01. **Cases:** FS01–FS08, AUT06.
**Files:** `safestos/cranelift/src/capstd_mod.rs`, umbrella resource injection,
new operation-rights and adversarial directory tests.

1. Host opens approved roots and injects rooted directory handles. Bind rights
   separately for read/write/create/list/rename/delete; validate relative paths
   at actual operation time using capability-aware APIs, not string prefixes.
2. Add revocation/lifetime/quota/error semantics; distinguish not-found from
   permission failure. Preserve native path names or document rejected forms.
3. Test real inside/outside sentinels, symlink/rename/hardlink paths and races,
   read-only denial and cross-instance handles. Replace fixed /tmp names and
   assumed `/etc/passwd` with isolated known fixtures.

**Accept:** denied operations leave outside bytes unchanged and return the named
reason. `capstd` is actually compiled/enabled; stub rejection is separate coverage.

### U20 — Implement bounded network capabilities

**Depends:** U09/U13/U19. **Cases:** NET01–NET06.
**Files:** **new** network adapter/rights module, resource injection and loopback fixtures.

1. Define allowed dial endpoints/services, protocol and port/IP policy. Use a
   capability network API available in the pinned dependency or a small audited
   adapter; cap-std filesystem support alone provides no TLS/HTTP policy.
2. Validate actual resolved/dialed destinations and any redirect/proxy/reconnect.
   Use normal library TLS identity verification, bounded reads and deadlines.
3. Test denied endpoint has zero accepted connections; mock DNS rebinding,
   redirects and credential/certificate mismatch without public network access.
   Unsupported UDP/listen/raw operations fail explicitly.

**Accept:** no unauthorized destination is reached; context/revocation survives
socket reuse according to the documented contract.

### U21 — Expose useful ordinary Rust libraries through checked adapters

**Depends:** U11/U12/U13/U14/U15/U17/U19/U20. **Cases:** API01–API03, ABI06.
**Files:** generator, selected local/pinned Rust adapter crates, generated Austral APIs.

1. Demonstrate free functions, constructors, immutable/mutable methods, Drop and
   concrete generic instantiations with Rust-checked trait bounds. Qualified
   schema selects explicit items; no source regex guesses arbitrary Rust types.
2. Add bytes/text, Option/Result and owned collection adapters using stable
   representations/opaque handles. Keep zero-sized and unsupported dynamic
   layouts explicit. Rust's private implementation layout never leaks by default.
3. Integrate one capability-aware filesystem library path and one network client
   path using host-provided resources. A library with ambient APIs needs an
   explicit trusted-ambient/isolated classification; do not label it constrained.

**Accept:** generated code performs genuine library operations matching a direct
Rust caller; all unsafe adapter contracts are listed and tested at their boundary.

### U22 — Replace the prototype demo with a real capability workflow

**Depends:** U01/U16/U21. **Cases:** HAR02, FS01/FS04, NET01, AUT02/AUT03.
**Files:** `examples/modules/rust_kv/`, umbrella example project, smoke runner.

1. Real Rust adapter reads a granted local record, performs a deterministic
   transformation, invokes a scoped callback and writes through a granted output
   capability. A second path calls the allowed loopback service.
2. Execute through stock Austral compile-only output, umbrella provider loading,
   checked JIT invocation and actual IO. Denied paths verify absent filesystem
   and server markers, exact status and unchanged outside sentinels.
3. Compare output to an independent direct Rust/reference executable. Perturb
   input data so hardcoded expected output or a canned PASS cannot survive.

**Accept:** one script runs real positive and negative paths and reports actual
evidence. Unavailable features fail strict mode instead of printing success.

### U23 — Supervise native workers and bound execution

**Depends:** U06/U09/U15. **Findings:** UR09/UR11. **Cases:** RUN01–RUN03/RUN06, API06.
**Files:** worker protocol/supervisor, invocation state, crash/timeout fixtures.

1. Use one native instance/generation per supervised worker for the initial
   production profile. Authenticate/control its IPC locally and bind instance
   identity; bound frames/output and reject malformed responses.
2. Parent enforces hard deadlines, cancellation and supported resource limits.
   Reap child trees, retain bounded evidence and distinguish failure before/after
   an externally visible effect. Cancellation is not a guarantee of rollback.
3. Test native loops, initializer hangs, traps, abort, segmentation fault,
   callback deadlock and worker exit during a call; parent stays responsive.

**Accept:** no native fault becomes a success value, indefinite parent hang or
orphan. This establishes lifecycle control; U26 establishes any sandbox claim.

### U24 — Fuzz, sanitize and mutation-test the boundaries

**Depends:** U10/U14/U15/U16/U19/U20/U23. **Cases:** HAR04, CPS05 and matrix controls.
**Files:** **new** parser/descriptor/IPC fuzz targets, regression corpus,
`scripts/test-umbrella` robustness lane.

1. Fuzz CLI/config/Cargo JSON/CPS/descriptor/status/handle/IPC decoding under
   explicit input-size/depth limits. Minimize/save crashes and hangs as fixtures.
2. Run adapter/handle Rust logic with Miri where supported; use ASan/LSan and
   suitable thread/race tooling on supported native children. JIT execution may
   be outside Miri/TSan support; record those limitations and use real process
   tests, not blanket disabled coverage.
3. Apply matrix enforcement mutations in scratch copies. Verify baseline passes,
   edit really applied, intended case fails for its oracle, and tree is restored.

**Accept:** fixed bounded seed corpus is green, no new relevant sanitizer defect,
and every required mutation control distinguishes working from removed protection.

### U25 — Lock artifact identity and package a relocatable toolchain

**Depends:** U05/U07/U08/U10/U18. **Finding:** UR13. **Cases:** PKG01–PKG03/PKG07, AUT05.
**Files:** bundle manifest/doctor command, packaging recipe, loader version handshake.

1. Bind Cargo.lock/features/target, compiler/bridge/generator ABI, Rust library and
   relevant native dependency identities. Do not use a symbol name or short hash
   as provider authentication. Prevent validate/use replacement through immutable
   staging/open-handle ownership appropriate to the platform.
2. Install into a fresh prefix with versioned resources/tool discovery; remove
   compile-time checkout path assumptions and unsafe loader-path guessing.
   `doctor` reports exact missing/incompatible prerequisites without running code.
   Test environment/config overrides, preloading and transitive dynamic-library
   resolution; a pinned top-level cdylib alone does not pin its native dependencies.
3. Test offline locked rebuilds, moved prefix and mismatched toolchain/bridge.
   Define reproducibility scope: canonical manifest/semantic artifact identity
   first; byte-identical native output only where measured/supported.

**Accept:** installed demo runs away from the source tree; no stale neighboring
bridge or changed native dependency silently satisfies the version handshake.

### U26 — Validate an untrusted-native/build isolation profile

**Depends:** U02/U19/U20/U23. **Cases:** RUN02/RUN04/RUN05/RUN06.
**Files:** existing VM/native isolation integration where applicable, separate
profile launcher and escape fixtures; see `docs/spectrum_plan.md`.

1. Reuse a supported existing VM/OS isolation backend instead of assuming the
   ECMAScript sandbox directly supports native cdylibs. Specify shared mounts,
   network access, keys, process/resource controls and capability RPC boundary.
2. Isolate both Cargo build scripts/proc macros and runtime native code where
   the profile promises it. Test direct ambient std/fs/socket/process escape,
   initializer code, hostile subprocesses and resource exhaustion.
3. Record actual hardware/backend availability and pass/gap separately. If the
   backend cannot run here, keep untrusted-native mode disabled and publish only
   the tested trusted-native profile.

**Accept:** every promised escape control runs and blocks its marker. A manifest,
cap-std API, child process or malicious-fixture refusal alone is not a sandbox.

### U27 — Complete native target and platform validation

**Depends:** U11/U25. **Cases:** ABI07, BLD05, PKG04.
**Files:** platform module, target CI configuration and adapter ABI vectors.

1. Run the same real ABI/ownership/capability/e2e fixtures natively on Linux
   aarch64 and x86_64. Capture actual calling convention and loader dependencies.
2. Validate architecture, pointer width, endianness, binary format and compiler
   target metadata. Separate build target from worker execution target.
3. Unsupported OS/architecture or foreign-target execution gives a precise
   early error. Do not add superficial DLL detection as Windows support.

**Accept:** both advertised native targets pass their required lanes; any missing
target is removed from the release claim until measured evidence exists.

### U28 — Wire strict CI and coverage inventory

**Depends:** U22/U23/U24/U25/U27; U26 additionally for isolated profile.
**Cases:** HAR01/HAR03, AUT06, CPS07, PKG06.
**Files:** **new** umbrella CI job, runner case inventory, existing invariant gate hooks.

1. Fast PR lanes cover CLI/build/schema/ownership/capability tests. Real e2e runs
   on touched seams; resource/sanitizer/mutation/platform lanes run on a bounded
   schedule with saved evidence. Do not require a full Rust-fork rebuild for PRs.
2. Assert case collection, features and required lane status. Every claimed
   capability operation has a positive/negative executed test; no echo/ignored
   demo counts as e2e coverage. Check owned/borrowed API census and descriptor drift.
3. Preserve existing bridge freshness, opcode, TODO-ledger and proof gates.
   Required liquid checks fail without a prover; optional gaps remain explicit.

**Accept:** intentionally removing a required test/feature or replacing a guard
with a no-op makes CI fail for the expected reason, with bounded readable logs.

### U29 — Write operator/debugger and compatibility runbooks

**Depends:** U28. **Cases:** PKG01/PKG02/PKG06.
**Files:** umbrella README, **new** troubleshooting/ABI/support-status documents.

1. Document build/check/run/doctor, trusted vs isolated profiles, supported API
   types/targets, generated adapters, build-script trust and actual unsafe seams.
2. Publish a diagnostic decision tree: parse → metadata → Cargo → artifact →
   frontend → descriptor → load → authorization → invocation → IO → cleanup.
   Each stage has reproduction command, evidence path and expected status.
3. Document format/version migrations, dependency upgrades, provider reload,
   revocation, rollback and effect-uncertain recovery. A restart-based upgrade
   is a supported operation; arbitrary in-place ABI replacement is not implied.

**Accept:** a clean environment user can follow the runbook without developer
checkout paths, unexplained flags or claims stronger than the executed lanes.

### U30 — Establish resource/performance budgets and final release gate

**Depends:** U18/U22/U23/U25/U29. **Cases:** PKG05, FS08, RUN02/RUN07/RUN08.
**Files:** bounded benchmark/soak script, release manifest and evidence summary.

1. Measure cold/warm build, no-change rebuild, scalar/handle/buffer/callback
   crossing and realistic fs/net workflow. Compare to direct Rust and the
   prototype baseline; separate setup from per-call overhead.
2. Repeated runs/reloads/cancellations measure RSS, mappings, fd/child/temp growth
   and cache/disk bounds. Record machine/target/features and calibrated budgets.
   Avoid unstable timing assertions for semantic correctness.
3. Gate release on all required cases/controls, no unexplained skips, clean
   native fault handling, documented residual unsafe contracts and tested package.
   Isolated and oracle profiles require their extra gates independently.

**Accept:** reproducible evidence supports the exact advertised profile, budgets
are enforced and regressions cause a useful failure. A green build alone cannot
set the project status to production ready.

### U31 — Optional asynchronous host integration

**Depends:** U16/U20/U23. **Cases:** API08 and applicable runtime/ownership cases.
**Files:** future/task handle adapter, worker protocol and cancellation tests.

1. Keep Rust futures/executor state inside the host; expose owned pending/result
   handles with explicit polling/wakeup/cancel semantics, not native Future layout.
2. Test completion/cancel races, double completion, worker restart, callback
   lifetime and an external side effect with unknown outcome. Do not hold borrowed
   Austral buffers across await unless a distinct owned protocol guarantees it.

**Accept:** no use-after-free, leaked task or false rollback claim. Until this
passes, the product explicitly supports synchronous adapters only.

### U32 — Optional pinned Rust type-oracle/direct-import plugin

**Depends:** U10/U14/U17/U25/U27. **Cases:** ORC01–ORC06.
**References:** Valen/Rust files named in the review; keep them unchanged.

1. First write a capability/compatibility report: stable adapter schema versus
   requested direct imports, exact missing information, rustc query/ABI hook
   needed, toolchain/LLVM/private-API cost. Prefer a subprocess oracle that emits
   the same descriptor schema; do not embed rustc in the OCaml compiler.
2. Pin rust fork `c6f757802519a0fc2619dd79e0be879601899b06` and a compatible Valen
   reference if used. Verify rustc/sysroot/LLVM fingerprints before private API
   use; reject unsupported pass modes or types with typed errors. Build only
   the toolchain/components necessary for this optional experiment.
3. Test cold/warm query/dependency results, methods/concrete generics/Drop and
   borrow invalidation. If experimenting with replacement MIR/extra LLVM modules,
   test symbol retention, placeholder-derived optimizer attributes and generated
   body safety separately. Do not assume those hooks run a borrow check on
   arbitrary replacement code.
4. Compare results to stable adapter/direct Rust oracles. Require borrow checking
   enabled and every unsupported path explicit. No release depends on Valen
   demonstrations that use `--no-borrow-check`.

**Accept:** direct-import feature passes its exact toolchain/ABI/ownership gate
and uses the production host's provider/capability boundary. Otherwise retain
it as an experimental plugin; the stable production route remains functional.

## 7. Debugging order and stop rules

When a case fails, first locate the last completed pipeline stage and verify
tool/target identity. Reduce to a local fixture, compare the direct Rust oracle,
then compare generated schema/declaration, CPS signature, resolved provider and
runtime context. Inspect memory/ABI only in a child after the static contract
checks. Keep both the original failing trace and the minimized one.

Do not repair wrong native results by changing goldens, disabling optimizer
checks, extending `--allow-all`, making a handle Clone, switching every borrow to
raw pointers, suppressing sanitizer failures, accepting arbitrary `.so` paths or
making all errors success. Those changes destroy the test's intended property.

If two small repair attempts do not reduce the same failure, record the exact
symptom, input, competing explanations and next discriminating experiment. Split
the task rather than expanding into a compiler rewrite. No schedule assumes a
cheaper model will infer Rust ABI/ownership rules from passing one checksum.

## 8. Execution record and copyable assignment

U00 creates the real ledger; all tasks start pending. Preserve baseline findings
separately from post-fix results. Use these columns:

| Task/substep | Case IDs | Status | Source/tool/feature identity | Command and evidence | Oracle/result | Remaining limits |
|---|---|---|---|---|---|---|
| U00 | HAR01/HAR03 | pending | — | — | — | Harness not implemented |
| U07 compile-only artifact | CPS01/CPS02/CPS05–CPS07 | implemented with gaps | `lib/Compiler.ml`/`Compiler_cps.ml` emit-only branch + `typed_exports`; `umbrella/src/frontend.rs`, `umbrella/src/staging.rs` (new); `umbrella/src/main.rs` stage 3 | `dune build lib/ bin/ test/` → 0; `019-extern-rust/010-jit-unresolved-symbol` with `--emit-cps-only` → exit 0, 182-byte CPS, no bridge compile, typed export `main()->Int64`; `examples/memory-ops` includes a space-bearing monomorph name; the unchanged path → exit 255; reduced-feature `cargo test --lib` → 98 passed | A JIT-unresolvable module still yields CPS, but never a runnable artifact; no `--allow-all`/advisory; the artifact names its typed exports with wire types | `front_end` still takes the adapter document from the last prefix; default-feature Rust build blocked by the sibling `unfer` merge conflict |
| U08 explicit provider binding | LNK01–LNK04, AUT05 | implemented with gaps | `umbrella/src/provider.rs` (new); stage 2 in `umbrella/src/main.rs`; `umbrella/tests/entry.rs` | reduced-feature `cargo test --lib` → 96 passed; entry/cli/policy → 17 passed; real C fixtures: preloaded first image loses to the admitted handle, two definitions refused; two providers refused before mapping | granted `rs_*` exports bind to exact addresses before JIT init; provider is RTLD_LOCAL; registration is not permission | multi-provider refused; default-feature cargo blocked by sibling `unfer` conflict |
| U10 adapter/provider linkage | LNK01/LNK02, ABI03 | implemented for driver path | `umbrella/src/adapters.rs` link input + `load_for_jit`; `policy::dlopen_local`; `provider` uses local mapping; `umbrella/tests/linking.rs` | `cargo test --test linking` → 1 passed; lib → 96 passed; entry/cli/policy → 17 passed; adapter `DT_NEEDED` is the provider's absolute path and its call returns the provider value under RTLD_LOCAL | adapter `rs_*` calls resolve to the admitted image, not global scope | legacy `build_next`/`load` reload tests still use global mapping; moving the provider after build breaks the adapter; default-feature build blocked |
| U10 typed signatures + descriptor/provider digests, generated trust record, contract-before-mapping | ABI03, ABI06, API03 | implemented for driver path | `umbrella/src/project.rs` `trusted.signatures`/`descriptor_sha256`; `umbrella/src/trust.rs` `verify_signatures`/`verify_descriptor`/`verify_artifact`/`trust_record`; `umbrella/src/cli.rs` `--emit-trust-record`; typed check before `ProviderSet::load` and event `kind`/`abi_version` fields in `umbrella/src/main.rs`; `umbrella/tests/entry.rs` | reduced-feature `cargo test --offline --lib` → 105 passed; `--test entry --test cli` → 9 + 6 passed; a pinned `compute(Int64) -> Int64` against `compute() -> Int64` exits 1 before `RESULT` and leaves no `load` marker, a zero `descriptor_sha256` does the same, the success event carries `exports`, `adapters`, `adapters_sha256`, and `--emit-trust-record` writes pins that parse back into `trusted` | exact typed signature and compiler descriptor pinned before use; provider initializer only after that check; pins generated from the admitted artifact | wire types only; descriptor pin covers last-prefix document bytes including comments; default-feature Rust blocked by sibling `unfer` conflict |
| U09 per-instance identity/policy | AUT02–AUT04, LNK03 | started | `InstanceContext.policy` + `bind_instance_with_policy`/`bind_instance_with_identity`/`replace_policy` in `safestos/cranelift/src/auth.rs`; `module.rs` load/swap; `identity.user` in `umbrella/src/project.rs`; umbrella stage 4 | reduced-feature bridge `auth::tests` → 13 passed; umbrella `project::tests` → 8 passed; `cargo check --lib --bin` → 0; entry/cli/policy → 17 passed | A/B snapshots do not share grants; narrowed rebind refuses the removed grant with UK-4001; the user delegator is recorded without becoming the grant principal; compilation runs in the instance caller scope | capstd operations and handle owner tags remain separate; host-issued rebind is trusted; no concurrent A/B worker test; default features blocked by sibling `unfer` conflict |
| U11 steps 1–3 (scalar adapters) | ABI04/ABI05 | passed within stated profile | `lib/Compiler_cps.ml` descriptor emission, `safestos/cranelift` adapters, `umbrella/src/interface.rs` generator | `cargo test --offline --test scalar_roundtrip` → 3 passed (1993 s); live `AdapterLive` lane builds and loads 8 wrappers; `cargo test --offline --lib interface` → 12 passed, `--test interface` → 7 passed | Independent Rust calls compared bit-for-bit (`-0`/NaN/infinity included); unsupported scalars refused at typing (`019-extern-rust/008`) | Only x86_64-linux measured; U27 repeats natively |
| U11 step 3 (fail-closed JIT) | ABI05 | passed within stated profile | `lib/Compiler.ml` `Jit_compile_failed` → `GenericError`, exit 255; `CPS_JIT_ADVISORY` remains a bridge compatibility path. U07's umbrella front-end no longer sets it: compile-only never initializes the JIT | `019-extern-rust/010-jit-unresolved-symbol` (real compile, exit 255); `AdapterLive` without the advisory env exits 255; U07 compile-only fixture → exit 0 with a CPS artifact and no JIT compile | The runnable path cannot exit 0 with no runnable binary; the advisory opt-out is no longer used by the umbrella | The advisory env is process-wide, so a third-party caller can set it; the bridge still reports the reason on the last-error channel |
| U12 bounded start (buffer descriptor) | OWN01/OWN02 | implemented with gaps | `umbrella/src/interface.rs` `BufferAdapter` (reader) + `render_buffer_wrapper`, `lib/Compiler_cps.ml` `adapter_block` (writer), `lib/TypingPass.ml` (identical admission rule) | `scripts/check-adapter-descriptors` → 6 descriptor(s) match the golden (2 of them `abi buffer 1` and 1 `abi buffer 2`, from the real lowering); `cargo test --offline --lib interface` → 14 passed, `--test interface` → 7 passed (incl. `the_compilers_emitted_descriptors_parse_here` over the emitted document); `019-extern-rust/009-buffer-needs-adapter` refuses a buffer with no length after it; `cargo test --offline --test buffer_roundtrip` → 3 passed (798 s); `cargo test --offline --test buffer_roundtrip the_jit_executes_a_buffer_call_from_a_compiled_module` → 1 passed (634 s, then 951 s on the three-target bytes) over `umbrella/modules/BufferLive.aum`; `make all` under `nix develop .#ocaml` → 0, after which `Demo.aum` lowers to `rs_demo_checksum__adapter` with `abi buffer 1` / `returns Int64` (was the direct symbol) | `Nat8` buffer + next-parameter 64-bit length + scalar result is described, generated and parsed from one document; direction from the pointer type (`Address` readwrite, `Pointer` read); the wrapper refuses a negative or `>i64::MAX` length and a null pointer with non-zero length; executed through the real ABI, a 5-byte fill inside an 8-byte allocation leaves the bytes past the declared length at their canary and matches the direct call, a checksum over bytes containing NULs matches the direct call and differs from the NUL-stripped one, the length-0 probe reaches the target as a null pointer, and a negative length or null-plus-length aborts (`SIGABRT`, message checked) in a child before the target is called; and a compiled Austral module now allocates its own buffer (`allocateBuffer` = the `au_calloc` JIT symbol) and calls three `rs_*` targets through the generated wrappers, `RESULT 170` with the driver reporting all three wrappers built and loaded, where the target returns -1 if the byte past the declared extent was overwritten (so a wrapper passing the allocation size yields 0); every buffer call is mediated, including the all-64-bit `(Address[Nat8], Nat64) -> Int64` shape that used to stay word-native, and that third target's only manifest grant is its `__adapter` symbol, so a regression to the direct call is refused by the JIT gate | A module written against the old rule must now grant `<target>__adapter` for a buffer call (the native symbol is unreachable from a module); text/UTF-8, owned result handles and a capacity check beyond the declared length remain |
| U13 (handle registry, generated wrappers, linear type) | OWN03–OWN05/OWN08/OWN09, LNK05 | implemented with gaps | `safestos/cranelift/src/handles.rs` (new) `HandleRegistry<T>`, `Owner`, `TypeTag`, `Handle` (two-word encoding), `ProviderValue`, `HandleError`; `umbrella/src/handles.rs` re-exports it; `umbrella/src/interface.rs` `HandleAdapter` + `render_handle_wrapper`/`_at` (`abi handle 1`, optional `use`/`__use`, `__with`, per-build provider generation); `umbrella/src/adapters.rs` renders and reloads the adapter crate at the next generation (`build_next` + `load`), used by `umbrella/src/main.rs`; `umbrella/modules/WidgetHandle.{aui,aum}`; `test-programs/suites/025-linear-handles` | `cargo test --offline --lib handles` in `safestos/cranelift` → 3 passed; `cargo test --offline --lib --test handles` in `umbrella` → 63 lib tests + 14 integration tests passed, and `cargo test --offline --test interface` → 7 passed, EXIT 0; `python3 test-programs/runner.py 025-linear-handles` → 3 PASS; `./austral compile umbrella/modules/WidgetHandle.aui,umbrella/modules/WidgetHandle.aum --entrypoint=WidgetHandle:main --target-type=c --use-cps-jit --allow-all --emit-adapters=test-programs/adapters/handles.golden` → exit 0, emitting `extern au_int64_t avm_widget__dispose(au_int64_t lo, au_int64_t hi);` and its direct call from a source that names only the target `avm_widget`, plus the `abi handle 1` block (`symbol avm_widget__new`, `target avm_widget`, `type widget`, `use avm_widget_read`, `params 2`) and `extern au_int64_t avm_widget__use(au_int64_t lo, au_int64_t hi, au_int64_t* out);` from the module's `read` seam, and `main` now constructs from source through a record out slot (`widgetNew(7, 9, refMutToPointer(&!pair))` → `extern au_int64_t avm_widget__new(au_int64_t lo, au_int64_t hi, mono_15* out);` plus its direct call) with the descriptor golden still byte-identical; the bridge names `__embed` per arity (`__embed$<argc>`), so one module may carry embeds of several shapes; `__with` lends a live pair to a Rust callback (the callback sees the provider's value, returns a word, the pair stays live and disposes once, and a stale/future-generation/null pair, a null callback and a null out are all refused before the callback runs); the reload evidence is a real host reload (`adapters::build_next` twice, each a new artifact and a real `dlopen`): the reloaded crate refuses the previous build's words while the previous crate still resolves and disposes its own; `bash scripts/check-adapter-descriptors` → 5 scalar/buffer + 1 handle descriptor(s) match their goldens; `rustfmt --check` clean; `bash scripts/check-todo-ledger` green | A handle is two words, `owner << 32 \| slot` and `provider generation << 32 \| slot generation`, so every checked component is carried without truncation; the null pair `(0,0)` and owner tag 0 are refused, and the generator numbers registries in one crate from 1, so a handle for one object type has an owner tag no other registry holds (the type tag stays the in-process check, since the pair has no room for it). `__new(words…, out)` writes the pair and returns 0/1, refusing a null `out` before the provider allocates; `__dispose(lo, hi)` returns 0 only when a live pair reaches the destructor. The constructor pointer is a `ProviderValue` whose `Drop` calls the provider's destructor, so a refused insert frees the value once and a destroyed value is never freed twice; the measured oracle is the provider's own drop counter read through `dlsym` (constructor 0 + non-null pair, first dispose 0 and one drop, second dispose 1 and no second drop, another type's dispose refuses the words while its own still work, a reused slot hands out the same low word with a new generation and the stale words are refused while the new value stays live, forged generation / null out / null pair each refused without allocating); the `__use` wrapper returns the provider's word for a live pair, refuses a stale, foreign, null or future-generation pair without writing its out slot, refuses a null `out`, and does not destroy; a second render at generation 2 in the same image refuses the generation-1 words for both `__dispose` and `__use` while the generation-1 value stays live and destroyable through the table that minted it, and the generation-2 value reads and destroys through its own. `allocated`/`freed` are counted independently of `issued`/`released` (`take` releases but does not free), which is what the drop oracle is compared against. Austral side: `Widget` is an opaque `Linear` record with `adopt` as the only constructor and `dispose` as the only consumer, so forgetting it and disposing it twice are linearity errors — measured by the three suite fixtures | The lowering emits the `abi handle 1` block from a `Handle_Ownership` pragma, derives the import's `<target>__dispose`, and an optional `Use` names the provider reader that the generated `__use` wrapper calls, so the module's `read` seam is exercised (`main` now mints through `__new`, adopts, reads through `__use` and disposes). A `__with` wrapper is rendered for every handle adapter and tested through the host ABI, but no module can call it yet: the CPS lowering has no first-class function value to hand it (`MFptrCall` is a call by name), so a module lends nothing and keeps `read`. The generation bump is exercised by a real reload path (`adapters::build_next` + `load`), not by a second render; a provider that is rebuilt while a module runs is not hot-reloaded live. Borrows/views over an owned handle are U14. Each `cargo test` run in `umbrella` links the bridge and takes ~20 min |
| U14 borrowed views (runtime claims + module-held view) | OWN06/OWN07 | implemented with gaps | `safestos/cranelift/src/handles.rs` `Borrow`, `borrow`/`borrow_mut`/`release`, `borrow_to_words`/`borrow_mut_to_words`/`get_words_view`/`get_words_view_mut`/`release_words`, `HandleError::Borrowed`/`UnknownBorrow`; `umbrella/src/interface.rs` `<target>__borrow`/`__view`/`__borrow_mut`/`__write`/`__release` wrappers and forwarders, gated on the descriptor's optional `write` line; `Handle_Ownership` `Write` field through `lib/Common.ml`, `CstUtil.ml`, `TypingPass.ml`, `Monomorphize.ml`, `Stages.ml` and `Compiler_cps.ml`; `umbrella/modules/WidgetHandle.{aui,aum}` opaque `View`, `holdView`/`holdViewMut`/`viewRead`/`viewWrite`/`release`; `test-programs/adapters/handles.golden`; `umbrella/tests/handles.rs` | bridge `cargo test --offline --lib handles` → 13 passed and `--lib` → 90 passed; umbrella `cargo test --offline --lib interface` → 30 passed and `--test handles` → 21 passed, including `a_mutable_view_writes_and_the_write_is_visible_after_release` and the two re-export tests; `bash scripts/check-adapter-descriptors` → 6 scalar/buffer + 1 handle golden match; the updated `WidgetHandle` compiles from source (`./austral compile … --target-type=c` → exit 0) and emits `write avm_widget_write` in the `abi handle 1` block | immutable group claims coexist; a mutable claim excludes every other claim; any claim refuses `get_mut`/`take`/`destroy` and `__dispose`, so a view never outlives its value; mutation only through the exclusive token, because `get_words_view_mut` refuses an immutable claim; a module mints the exclusive token with `holdViewMut`, writes with `viewWrite`, and the value `__use` returns after release is the written one; a writer panic is a `STATUS_PANIC` word; the token is not the handle | the exclusive view exists only when the descriptor names a `write` line (the generator never invents a writer symbol); one write carries one 64-bit word; ordering of release vs dispose is runtime-checked, not type-checked; no compiler-negative fixture; claims are explicit rather than lexical |
| U15 status/panic ABI (handle wrappers) | ABI01/ABI06/ABI08, CLI06 | implemented with gaps | `safestos/cranelift/src/handles.rs` `STATUS_OK`/`STATUS_REFUSED`/`STATUS_PANIC` and `Status::from_word`; `umbrella/src/interface.rs` `extern "C-unwind"` constructor/reader block, `catch_unwind` in `__new`/`__use`/`__view`/`__with`, named statuses; `umbrella/tests/handles.rs` panicking fault provider and `panic=abort` child fixture; `umbrella/src/interface.rs` `render_wrapper_checked`/`render_buffer_wrapper_checked` (`<symbol>__checked`, status + out payload, `C-unwind` provider, `catch_unwind`); `umbrella/tests/checked_adapters.rs` | bridge `cargo test --offline --lib handles` → 13 passed and `--lib` → 90 passed; umbrella `cargo test --offline --lib interface` → 31 passed; `cargo test --offline --test handles` → 22 passed; `cargo test --offline --test checked_adapters` → 1 passed (success, refused word, provider panic, null out, recovery); `cargo test --offline --test handles a_panicking_provider_is_a_status_word_and_keeps_the_value_live` → 1 passed over a compiled wrapper crate (panicking constructor: `STATUS_PANIC`, no handle, nothing stored; panicking reader: `STATUS_PANIC`, out untouched, value still releaseable and disposable; same through a held view); `the_reading_wrapper_hands_back_the_provider_value` → 1 passed and `--test handles panic_abort` → 1 passed (the same wrappers compiled with `-C panic=abort` exit on SIGABRT) | status is separate from payload: `0` ok, `1` host refusal, `2` caught provider panic; `-1`/`i64::MIN`/`i64::MAX` are not statuses and `Status::from_word` rejects them; a panic never writes the out slot, stores nothing from a constructor, and leaves the value live | scalar and buffer wrappers return only their payload, so they have no status channel and still abort on a panic (a compiler-side call ABI change is needed); this status-channel form carries neither tags nor owned-output cleanup (the compile-selected `Option`/`Either`/`Result` path does, U15b/U15c); the checked scalar/buffer form is opt-in at the symbol level rather than chosen by the compiler for an ordinary `rs_*` import, so the payload import still aborts on a bad word or panic; this status-channel form carries neither tags nor owned-output cleanup (the compile-selected path does, U15b/U15c); the module's `read` still maps refusal to `-1` |
| U16 scoped callbacks (bounded start) | API04–API07, OWN07 | implemented with gaps | `safestos/cranelift/src/callbacks.rs` (new) `CallbackRegistry`/`CallbackScope`/`CallbackToken`/`CallbackPolicy` (`Lifetime`, `ThreadAffinity`, `Reentrancy`, `ErrorPolicy`); `umbrella/src/interface.rs` `__with` rewritten to claim the value, drop the table mutex, run the callback through a call-scoped token and release the claim on every path | bridge `cargo test --offline --lib --no-default-features --features cedar,tier1-rs callbacks` → 12 passed; umbrella `cargo test --offline --lib interface` → 32 passed; `cargo test --offline --test handles` → 24 passed; `cargo test --offline --test callbacks_retained` → 3 passed (two wrapper generations: generation one's retained callback is refused after generation two loads and generation two works; through the real provider protocol the provider's own `fire` export invokes the trampoline, is refused after the reload, then reaches generation two's callback; and the descriptor's `callback` policy round-trips, renders, gates the retained API and is measured across threads) | a null/unknown/retired/stale-generation/wrong-thread/reentrant token is refused before the callback runs; a `Call` token is one-shot; the table lock is never held across callback code, so a nested `__with` succeeds and a forbidden reentrancy is a refusal rather than a deadlock; a `__borrow_mut`/`__write`/`__dispose` during the callback is refused by the live immutable claim; a caught callback panic is `STATUS_PANIC` with the out slot untouched and the value still live; the claim is released on the panic path; a handle wrapper's `<target>__with__retain` keeps an instance scope, `<target>__with__invoke_retained` invokes its reusable token, and `<target>__with__retire_callbacks(g)` closes the scopes and retires the generation; an optional `subscribe <symbol>` descriptor line makes `__retain` hand the provider `(owner, dispatch, slot)`, and the provider invokes that trampoline itself (a slot is an id, so a stale call is refused); an optional `callback` line names `lifetime`, `thread`, `reentrancy` and `error`, each field optional and defaulting to the previous fixed value, `lifetime=call` omits the retained API and rejects `subscribe`, and the runtime test refuses a retained token invoked from another thread under `thread=caller` while `thread=any` accepts it; `adapters::load_for_jit` calls each older loaded build's own hook through its own image before mapping the next generation | no CPS first-class callback value, so the module-visible use is the `__with` borrow plus the descriptor-driven `subscribe` protocol; the compiler emits neither the `callback` line nor `subscribe` (no `Handle_Ownership` pragma), so a module built from source gets the defaults and cannot request the protocol without the host editing its descriptor; `thread` only differs for a retained token invoked later, because `__with` opens its scope on the caller; the lifetime field gates the retained API but cannot make `__with` instance-scoped; a panic in a different dylib's panic runtime cannot be caught, so retention assumes the provider does not unwind |
| U15b Either / two-payload checked union | ABI01/ABI06 | implemented with gaps | `umbrella/src/interface.rs` `CheckedUnion`, `checked_union`, `failure-payload`/`union`/`failure-case`/`success-case` descriptor fields, `render_wrapper_checked_union` (provider writes `[tag, payload]` through an out pair, wrapper returns the union pointer and validates tag/payload); `lib/TypingPass.ml` `checked_payload_union_result` and `checked_payload_unions` admit the built-in `Either[L, R]` and `Result[T, E]` when both payloads are adapter scalars; `lib/BuiltInModules.ml`/`BuiltIn.ml` declare and import `Result` (`Ok`/`Err`); `lib/CheckedOptions.ml` carries union/case names and the failure payload; `lib/Compiler_cps.ml` `ar_checked_union`, union block emission, one-slot-per-case synthetic layout, tags read back from `union_layouts`; `umbrella/modules/EitherImport.aum`; `umbrella/modules/ResultImport.aum`; `umbrella/tests/either_adapters.rs` | `cargo test --offline --test either_adapters` → 4 passed (descriptor names/tags/round-trip and partial-block refusal; wrapper through the real ABI: both cases, unknown tag, non-zero status, undecodable `Bool` payload; live front-end → wrapper → JIT: `computeRight` 11, `computeLeft` 22; and the `Result` shape: `computeOk` 11, `computeErr` 22, with the provider's tags read back from its descriptor); `cargo test --offline --lib --test checked_adapters --test callbacks_retained` → 110/3/3 passed; `dune build lib/ bin/ -j 4` under `.#ocaml` → exit 0; `scripts/isa-census --corpus` → 14 modules, 23/23 opcodes, 0 uncovered, empty pin | one word cannot carry both a tag and either payload, so the provider writes `[tag, payload]`; the wrapper validates rather than trusts the tag; tags are the layout the `match` dispatch reads, so this module's `Either` monomorph (Right=0, Left=1) sets `failure 1`/`success 0` and its `Result` monomorph (Err=0, Ok=1) sets `failure 0`/`success 1`, and the provider must write the descriptor's tags, not Rust's enum order; the admitted set is one table, so the two shapes share the transport | a `Result`/`Either` whose error case is empty is not admitted (the `Option` arm is name-based); a cross-dylib provider panic is still uncatchable, so the live provider does not panic; the union release (U15c) is emitted for the `match` that consumes the call (directly, through its bound local/temp, or in the callee it is passed to; a rebinding clears the record), so a result never matched or traceable to a callee leaks rather than freeing wrongly |
| U20 bounded network capabilities | NET01, NET02, NET04, NET06 | implemented with gaps | **new** `safestos/cranelift/src/netstd_mod.rs` (`NetPolicy`, `Endpoint`, `IpNet`, `NetError`, `Connection`, `certs_from_pem`/`pem_blocks`); `capstd_mod.rs` `Handle.conn`, `InstanceCaps::with_net_policy`/`net_policy`/`dial`/`net_write`/`net_read`/`net_shutdown`/`with_net_handle`, `CAP_ERR_TIMEOUT`/`CAP_ERR_TLS`/`CAP_ERR_UNSUPPORTED`/`CAP_ERR_IO`, `rs_cap_net_dial`/`read`/`write`/`shutdown`/`unsupported`; `lib.rs` the five `rs_cap_net_*` symbols; `module.rs` `net_ranges`/`net_ca`/`net_timeout_ms`, load-time policy construction, `net_grants_do_not_escalate`; `tests/fixtures/tls/` (CA, `localhost`+`127.0.0.1` server cert, `good.test`-only cert, one key); `umbrella/src/project.rs` `NetworkCapability`/`Capabilities.net_policy` and field validation; `umbrella/src/main.rs` policy construction and `with_net_policy` before install; `Cargo.toml`/`Cargo.lock` rustls+ring | bridge `cargo test --offline --no-default-features --features cedar,tier1-rs --lib net` → **14 passed** (endpoint/CIDR/PEM parsers; endpoint-without-range refuses a resolvable name; no policy ⇒ no Net handle; denied endpoint listener counts **0** accepted connections; allowed loopback `ping`/`pong` with the dialed peer returned; `localhost` resolving outside the ranges refused before connecting, 0 accepts; TLS accepts a `127.0.0.1` certificate under the granted CA and refuses a `good.test`-only one for that address as `CAP_ERR_TLS` with no retained connection; ABI open/dial/revoke/write and `udp`/`listen`/`raw-socket` → `CAP_ERR_UNSUPPORTED`; an executed HTTP redirect where the second hop is a fresh checked dial, followed when both endpoints are granted and refused with 0 connections when only the first is; a resolver that does not answer releasing the caller at the deadline; manifest swap narrows endpoints/ranges/deadline and refuses every widening direction); `--lib fs_tests` → 14 passed; umbrella `cargo test --offline --lib project::` (reduced bridge features) → **15 passed** (network endpoints/ranges/anchor/deadline parse and build a policy; seven malformed forms named by field); `cargo check --offline --bin umbrella` → 0; full reduced bridge `--lib` → 156 passed, 1 pre-existing `a10_live_grant_tests` failure that needs `unfer-kernel` | a Net handle is a dial under the host's policy, not a socket or a name the module can reuse; every resolved address must be inside `[grants] net_ips` before a connection is attempted, so a name pointing at a denied address is refused rather than dialed; the endpoint list, ranges, trust anchor and deadline are all part of the grant, and TLS verifies the peer name/chain during the dial with no plaintext fallback; read/write are frame- and deadline-bounded, revocation drops the connection immediately, and UDP/listen/raw are refused by name; a swap may narrow the network grant but not widen any field | resolution is bounded for the caller (`getaddrinfo` runs on a thread the dial stops waiting for) but the lookup itself cannot be cancelled, so an abandoned thread may linger; no HTTP/proxy client ships — the redirect fixture lives in the test and application-level credential policy is out of scope; the trust anchor is a manifest-named file, not the system root store, and TLS without one is refused; only outbound TCP is represented; the project block is measured at document/policy level and by `cargo check` of the binary, not by a full project run that dials; NET03/NET05 remain partial (executed loopback redirect; handle revocation measured, socket-token reuse by another instance not) |
| U15c owned-output cleanup for checked unions | ABI01/ABI06 | implemented with gaps | `umbrella/src/interface.rs` `render_wrapper_checked_option`/`render_wrapper_checked_union` also export `<symbol>__free` (null and the wrapper's own static refusal are not freed; an answered call is one `Box<[u64;2]>`), `GeneratedAdapter::exported_symbols` lists it; `lib/Compiler_cps.ml` `checked_value_free`, `checked_owned_locals`, `checked_owned_fn_params`, `note_checked_argument_transfer`, `register_owned_parameters`, `callee_meth_key`, `concrete_method_symbol`, `local_concrete_methods`, `build_cps_decl`, `emitted_callee_keys`, `caller_release_needed`, `caller_release_setup`, `refuse_unsequenced_release`, `xfer_hoisting` (an `MConcreteInstance` emits one function per method under the instance-method id, so an `MConcreteMethodCall` resolves and its transfer is traced), the transfer fixpoint in `compile_module_cps`, ownership moved through a copy of an owned variable, and the `match` lowering copies tag+slots, releases the union, then dispatches; `umbrella/modules/EitherImport.aum` `consume`/`computeThrough`; `umbrella/modules/EitherMethod.aum`; `umbrella/modules/EitherProducer.{aui,aum}` + `EitherConsumer.aum`; `umbrella/tests/either_allocations.rs`; `test-programs/adapters/either.golden`/`result.golden`; `scripts/check-adapter-descriptors` | `cargo test --offline --test either_adapters` → 4 passed, including the `__free` export, freeing answered unions, freeing the shared refusal value and null, the CPS naming `<symbol>__free` in both live fixtures, and `computeThrough` → 11 (the union crosses into `consume`); `cargo test --offline --test either_allocations` → 3 passed (counting allocator: 2000 balanced `__checked`+`__free` = 2000 allocs/2000 frees/no live bytes, 500 unreleased calls visible as 500 live 16-byte allocations and back to baseline after release, 5000 JIT `computeThrough` calls = 5000 allocs/5000 frees/no live bytes; 5000 JIT calls through the concrete method `EitherMethod`'s `send` = 5000 allocs/5000 frees/no live bytes; the cross-module fixture's 5000 calls = 5000 allocs/5000 frees/no live bytes (the caller releases the union after the callee returns; the same test pinned 5000 allocs/0 frees/`5000*16` live bytes before the caller-side release existed); negative controls: moving the release after the dispatch makes the free-function test fail with 0 frees instead of 5000, and disabling the concrete-method transfer makes the method test fail with `left: 0, right: 5000`); `scripts/check-adapter-descriptors` → 6 scalar/buffer + 1 handle + 1 either + 1 result descriptor(s) match their goldens; `scripts/isa-census --corpus` → 14 modules, 1024 reads, 23/23 opcodes; `scripts/clif2mm0 --write` → 20 forms, 918 instructions; `dune build lib/ bin/ -j 4` under `.#ocaml` → exit 0 | release happens before dispatch, so an early `return` in a case cannot skip it and no body reads freed memory; a call transfers ownership to the callee's parameter, so the callee's `match` releases it, including when the monomorphizer hoists the argument into a temp; ownership through a copied variable moves rather than duplicates, so one linear value has one release; the static refusal pointer is recognised, so a refusal is never passed to the allocator; a copied scalar word owns nothing, so the copy needs no release | the release is exactly one-sided: a callee whose body this compilation emits frees a union transferred into it, and any other callee's by-value union is freed by the caller after the call returns (statement-boundary hoist `let __xfer_arg = union; let __xfer_ret = callee(__xfer_arg); <free>(__xfer_arg)`); the remaining leaks are a call whose declared result is a union (the callee may return the pointer, so the caller keeps ownership), a call nested in an expression the hoist cannot sequence (refused at compile time, not leaked), a function-pointer call, and a transferred parameter the callee never matches — leaks, not wrong frees; the cross-module release is measured by the allocation counter (5000/5000/0); the release symbol is an extra JIT call a manifest must grant like `<symbol>__checked`; the C backend does not render the checked call, so the two fixtures are `@cps` corpus entries |
| U17 build order and one bundled artifact (slice) | CPS03/CPS04, API02 | implemented with gaps | `umbrella/src/frontend.rs` `ModuleSource`, `read_module_sources`, `check_dependency_order`, `import_cycle` (duplicate declared names, self-imports and import cycles among listed modules are refused — no ordering exists for them), `dependency_order` (Kahn's algorithm, caller order as tie-break: the list itself need no longer be typed in dependency order); `umbrella/src/main.rs` `front_end` (the refusals run first, then one `frontend::compile` over the derived order); `lib/Compiler.ml` `emit_cps_bundle`, `bundled_functions`, `flush_cps_bundle`; `lib/CliParser.ml`/`lib/CliEngine.ml` `--emit-cps-bundle` (requires `--emit-cps-only`); `lib/Vm_plugin.ml` flushes after the whole list; `umbrella/src/frontend.rs` passes the flag; `umbrella/tests/entry.rs` recording front-end | `cargo test --offline --lib frontend::` → 22 passed, including the stable, reversed and three-deep ordering cases plus a duplicate name (both paths/positions named), a self-import, a two-module cycle and a three-module cycle; `CliParserTest.exe` → 5 tests OK (the fifth parses `--emit-cps-only --emit-cps-bundle --emit-cps=out.cps` with `--target-type=tc` and one module); `cargo test --offline --test entry` → 11 passed, including `a_two_module_list_is_one_bundled_front_end_invocation` (one `compile` invocation, `--emit-cps-bundle`, both module paths, `RESULT 7`) and `a_client_listed_before_its_dependency_is_compiled_after_it` (the client's import is read and `Lib.aum` precedes `Client.aum` in the recorded argv); `./austral compile EitherProducer.{aui,aum} EitherConsumer.aum --emit-cps-only --emit-cps-bundle` → one `Generated 3 functions (734 bytes)` artifact holding `consume`, `computeThrough` and `main`; `cargo test --offline --test either_allocations` → 3 passed (its consumer/producer fixture is a real dependency-ordered list) | a list that declares a name twice, imports itself, or closes an import cycle is refused before anything is compiled, with the offending names/positions, while an out-of-order list is ordered leaf-first instead of refused; imports of modules outside the list (built-ins such as `Austral.Memory`, Rust providers) are left to the compiler; only `module`/`module body` and `import` declaration lines are read, so the check costs no extra compile; the bundled artifact is written once after every module is lowered, so a dependency's functions and the entry module's travel in the same bytes and a failed lowering writes nothing | the bundle keeps the last (entry) module's name and the adapter document is still the last module's; a module whose declaration ends before its first token (e.g. an argument list on the module name) is the compiler's report; the rest of U17 (explicit graph, duplicate short names, cross-module identity) remains pending |
| U18 run cache + generation replacement (slice) | RUN07, LNK04–LNK06, OWN09 | implemented with gaps | `umbrella/src/cache.rs` `SCHEMA`, `key`/`KeyInputs`/`ProviderInputs`, front-end `load`/`store`/`invalidate`/`compile_cached_keyed`, provider `load_provider`/`store_provider`/`invalidate_provider`, adapter `adapter_digest`/`load_adapter`/`store_adapter`; `umbrella/src/main.rs` `provider_build_cached`, project-relative default in `with_defaults`, one `run_key` for all three artifacts; `umbrella/src/adapters.rs` `build_cached` + drain hooks; bridge `handles::drain`; `umbrella/tests/cache.rs`, `umbrella/tests/entry.rs` | `cargo test --offline --lib cache::` → 6 passed; `cargo test --offline --test cache` → 7 passed; `cargo test --offline --test entry` → 12 passed, including `a_project_run_warms_a_project_relative_front_end_cache` (two real-binary runs with no flag: one compile, then a reported hit, cache under `<project dir>/.umbrella-cache`); `cargo test --offline --test cli` → 8 passed; `--lib cli::` → 18 passed; `cargo check --offline --bin umbrella --tests` → 0; bridge `--lib handles::tests::draining` → 1 passed; umbrella `--test handles a_generation_replacement_drains_the_old_tables_live_values` → 1 passed | one key covers the front-end artifact, the provider cdylib and the compiled adapter crate; the key includes lowering args, liquid mode, compiler bytes, source bytes, provider crate tree, build request, recorded environment and toolchain; the provider copy is the single mapped/linked path, and a project's `library_sha256` still refuses stale bytes; the cache defaults next to the project document and `--cache-dir` overrides it; every load re-checks its recorded digest and a refused half is removed before the rebuild; a hit is bytes, not an admission — `trust::verify_*`, `install_auth` and UK-4001 still run; on replacement, `load_for_jit` drains the old generations' live handle values (provider destructors) before retiring callbacks, and a live borrow refuses the drain so the old build stays intact | the provider/adapter halves are measured by keying/storage tests, not a cold/warm provider build through the binary; the adapter key includes the process generation, so reuse assumes one adapter build per process; the provider key does not cover the cargo registry, path dependencies outside the crate directory or build-script inputs (a project's `library_sha256` is the backstop); JIT compilation is not cached; concurrent invocations sharing one cache directory are untested (`RUN08`); `RUN08`/`OWN09` remain partial |
| U19 descriptor-relative rooted filesystem capabilities | FS01, FS02, FS04, FS05, FS06 | implemented with gaps | `safestos/cranelift/src/capstd_mod.rs`: `RootDir` (an `OwnedFd` opened once with `O_DIRECTORY|O_NOFOLLOW`), `components`, `open_at`, `mkdir_at`, `stat_at`, `renameat2(RENAME_NOREPLACE)`, per-operation `openat` walking; `FsRights`, `RootGrant`, `InstanceCaps::new_with_grants`/`open_root`/`root_rights`/`revoke`/`revoke_all`/`charge`/`written`, `FsError::Revoked`; `rs_cap_open_root`/`rs_cap_list`/`rs_cap_delete`/`rs_cap_rename`/`rs_cap_revoke` and `CAP_ERR_REVOKED` registered in `RS_FFI_SYMBOLS` (`lib.rs`); `module.rs` `fs_root_rights`, `max_write_bytes`, `fs_grants_do_not_escalate`; `umbrella/src/project.rs` `CapabilityRoot`, `capabilities.rights`/`roots`/`quota_bytes`, `state_rights`/`root_grants`; `umbrella/src/main.rs` capability construction | bridge `cargo test --offline --no-default-features --features cedar,tier1-rs --lib fs_tests` → **14 passed** (rights per root and index; `../` and a symlinked root refused; cumulative quota refuses before opening and keeps old bytes; revocation/every verb, closed handle, `revoke_all`, install/uninstall; ABI write/list/rename/delete/`NOT_FOUND`/`REVOKED`; malformed manifest entries refused; a swapper thread alternating a real directory and an outside symlink over 400 reads never yields outside bytes and the sentinel is unchanged) and the full `--lib` suite → 142 passed, 1 pre-existing failure that needs `unfer-kernel` for `uk_version` in the strict profile; umbrella `cargo test --offline --lib project::` (reduced features) → 13 passed (rights/roots/quota parse; six malformed forms named) | a handle is an open root descriptor, never a path; every component is resolved by the kernel with `O_NOFOLLOW`, so a symlink swapped in mid-operation is refused, not followed; each root carries exactly its grant's rights, a plain string keeping the historical all-rights meaning; `create` and `write` are separate, `delete` refuses directories, `rename` refuses an existing target race-free; the quota is cumulative and charged before the target is opened; revocation is checked before every syscall and every open, and a replaced/uninstalled set is revoked; the manifest and project documents now name roots, rights and quota, and a swap may narrow but not widen them | an externally created hardlink to an outside file is still not detected (no `openat2 RESOLVE_BENEATH`/inode check); the quota is one cumulative figure per instance, not live per-root usage; the manifest/project wiring is measured at parser and `InstanceCaps` level, not by an umbrella binary run over a capability document (the `distributed_io` demo has no `[grants] fs` and keeps the historical all-rights state handle); `tests/capstd_tests.rs` still cannot compile here (`netcert` needs `unfer-kernel`, sibling `unfer` mid-merge) and `capstd` is unavailable offline; AUT06, FS07 and FS08 remain |
| U15 Result/Option tags (bounded start) | ABI01/ABI06 | implemented with gaps | `umbrella/src/interface.rs` `Adapter.checked`/`checked_tags`, `status`/`failure`/`success` descriptor fields, `render_wrapper_checked_option` (`<symbol>__checked` returns the Option union pointer: tag at 0, payload at 8), `GeneratedAdapter::render_at` renders payload + checked; `lib/Compiler_cps.ml` `checked_option_of`, `adapter_block` status lines, `MConcreteFuncall` lowers to `<symbol>__checked`, synthetic `None`/`Some` layout for the signature's id; `lib/TypingPass.ml` `checked_option_result` admits the built-in `Option[T]`; `lib/CheckedOptions.ml` carries the typer's tags/payload to the CPS pass | `cargo test --offline --lib interface` → 32 passed (checked Option descriptor round-trip, tags, union allocation, refusals); `cargo test --offline --test checked_adapters` → 3 passed, including the live front-end → descriptor → wrapper → JIT run of `umbrella/modules/CheckedOption.aum` (result 11 only when tag 1 and payload 7 are read); `dune build lib/ bin/ -j 4` under `.#ocaml` → exit 0; `cargo test --offline --lib` → 110 passed | the checked ABI allocates the union the JIT already represents, so no CPS IR change is needed; a decode failure, panic or unencodable payload is the failure tag, never the payload word; tags are the union's own indices from `union_layouts`, not a guessed order; the payload wrapper stays beside the checked one | the typer admits exactly the built-in `Pervasive` `Option[T]` with a scalar payload, and the CPS pass refuses rather than guesses when more than one `Option`-shaped layout exists; a provider in a separate dylib cannot have its panic caught by the wrapper (`Rust cannot catch foreign exceptions`), so the live test's provider does not panic and the caught-panic→`None` mapping is measured in the same-crate wrapper test; this status-channel form has no tags or release (the compile-selected two-payload path is U15b/U15c, and `Result` is now a built-in there) |

Allowed statuses: pending; reproducing; implemented with gaps; passed within
stated profile; experimental/unsupported.

**Next production path.** U15's bounded start now admits `Option[T]`,
`Either[L, R]` and `Result[T, E]` in the typer, selects `<symbol>__checked`
end-to-end (the two-payload forms add the provider's `[tag, payload]` out pair
and read their tags from the JIT layout), and releases the union it allocates
through `<symbol>__free` before dispatching the `match`; what remains is the
rest of that card (aggregate shapes beyond one scalar payload per case). U16 now has retained instance scopes and
reload retirement, a provider-driven `subscribe` protocol and a descriptor
`callback` policy; what remains is letting the compiler emit that policy and
the subscription from a pragma, and thread-hop dispatch. U17's qualified identities
across packages and its duplicate short-name handling (the build order is now
derived from the sources, so a list no longer has to be typed in dependency
order). U18 now reuses the front-end artifact, the provider cargo build and the
compiled adapter crate under one key over their complete visible input set (an
edited source, import line, provider crate, build request or toolchain rebuilds
the right half, and a corrupt entry is refused and replaced), defaults the cache
next to the project document, and drains a replaced generation's live handle
values before retiring its callbacks; what remains there is caching the JIT
compilation and concurrent cache access. U19 now opens approved roots as descriptors and resolves every path with
`openat`/`O_NOFOLLOW`, serves manifest/project per-root rights, extra roots by
index, the list/delete/rename ABI verbs, a cumulative write quota and
revocation; what remains there is hardlink detection, per-root quota and a
binary-level project run over a capability document. U20 now issues a `Net`
handle only under a host policy and dials through it: approved endpoints, every
resolved address checked against the granted ranges, bounded connect/read/write,
TLS identity verification against a granted anchor, revocation that drops the
connection, and named refusals for UDP/listen/raw, with the project document
spelling the same grant under `capabilities.network`; what remains there is a
bounded resolver, an executed redirect fixture, and the HTTP-facing adapter that
U21 builds on this dial. U21's
provider wrapper generation from a descriptor document (the renderers exist but
no CLI path writes them for a user build), U25's locked packaging, and U30's
release gate. Each is a card, not a checklist item: a bounded start is only
recorded here once it has a measured test. No case is passed because its test file
exists. A profile is production ready only after U30's actual release gate.

```text
Execute Uxx substep N from PLAN_umbrella_production.md.
Read its review finding and matrix rows, applicable AGENTS.md and dependency
evidence. Preserve unrelated dirty files. Create the smallest fixture that
reproduces the named defect, repair the production seam, and run the relevant
cases and existing focused suite. Native crash/UB fixtures run only in children.
Do not weaken grants, ABI/ownership contracts, goldens, proof statements or
required feature coverage to get a pass. Report exact command/status, independent
oracle and evidence paths; update the ledger. Stop at the next bounded patch.
```

## 9. References and attribution

Valen source at `b2aa19bdcd01570efba6684fac9a3d08a43c7d37` is Apache-2.0; its
LICENSE was read locally. The Rust fork at
`c6f757802519a0fc2619dd79e0be879601899b06` carries the Rust top-level MIT/Apache
license choices, read locally. This plan adapts design/testing ideas and copies
no source. Its reference analysis is recorded in the
[review](docs/UMBRELLA_PRODUCTION_REVIEW.md#4-valen-and-the-rust-fork-useful-mechanisms-distinct-trust-boundaries).

Existing constraints remain in [PLAN_valen.md](PLAN_valen.md),
[liquid types](PLAN_liquid_types.md), [compiler proof boundaries](PLAN_sota_kernel_verification.md),
and [VM isolation](docs/spectrum_plan.md). The [distributed security plan](PLAN_distributed_security.md)
is complementary: a production local adapter is a foundation for that work, not
evidence that its distributed claims have been proved.
