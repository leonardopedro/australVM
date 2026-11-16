# Umbrella / cap-std production review

Reviewed 2026-11-15. Implementation and test schedule:
[PLAN_umbrella_production.md](../PLAN_umbrella_production.md).
The [test matrix](umbrella/TEST_MATRIX.md) lists proposed acceptance cases;
those cases are not implemented by this documentation change.

## 1. Scope and evidence

| Checkout | Revision inspected | State |
|---|---|---|
| australVM | `95eb3bafb52d66a9618ff201fd4704ef8119773c` | Previous distributed-plan documentation changes preserved |
| Valen | `b2aa19bdcd01570efba6684fac9a3d08a43c7d37` | Clean |
| Valen rust fork | `c6f757802519a0fc2619dd79e0be879601899b06`, `per-instance-mir` | Clone completed during review; clean; no toolchain build performed |

Inspected umbrella's complete main program, manifests, demo and lockfile
presence; cap-std helper/tests and `rust_kv`; frontend FFI typing/linearity/CPS
seams; JIT resolution, authorization, execution and static-registration seams;
existing interop tests and CI; and Valen's orchestrator, type oracle, ABI query,
code-generation hooks, borrow/drop tests and Rust-fork patch.

This is primarily a source review. Two demo Rust tests were compiled directly
from `umbrella/modules/rscrate/src/lib.rs` with Rust 1.99.0 and passed on aarch64.
Six small CLI probes used the **unchanged extracted** `Cli`, `parse_cli` and
`die` definitions from `umbrella/src/main.rs`, with a printing main instead of
the build/loader pipeline. They establish parser behavior only. No full umbrella,
capability, compiler, sandbox or Valen integration suite was run.

The first probe compile failed because bare PATH had no `cc`; rerunning with the
existing aarch64 Nix GCC wrapper as rustc's linker succeeded. This is an
environment prerequisite, not a compiler defect. Temporary evidence is at
`/tmp/umbrella-review-l14e_fr2/results.txt`; it is disposable and not a portable
release artifact. The production test plan replaces this ad hoc probe with
checked-in tests using the actual library/CLI entry points.

## 2. What exists today

Umbrella builds one Rust crate as a cdylib, selects a Cargo-reported shared
library, dlopens it globally, asks stock Austral to emit CPS, then installs a
manifest authorizer, JIT-compiles and calls a named zero-argument function. The
checksum demo is useful evidence for that narrow path. Its own lockfile and the
demo crate's lockfile are tracked, although the build command does not enforce
locked resolution.

The bridge separately provides `CapFs`, manifest fs/net lists, swap subset
checks, an `rs_*` authorization namespace, a static symbol-registration seam,
and compiler rejection tests for malformed ownership annotations. These pieces
can be reused. They do not yet form a complete capability-enabled Rust compiler.

## 3. Findings

P0 blocks safe supported interop. P1 blocks production release. P2 is a feature
or operational gap to close for the declared product scope. These priorities
are planning judgments; source observations and measured probes are separated.

### UR01 — cap-std integration and the demo are incomplete (P1)

`umbrella/Cargo.toml` does not enable the bridge's `capstd` feature and umbrella
never constructs/passes `CapFs`. `capstd_mod.rs` exposes rooted directory reads,
writes, listing and existence, but it does not interpret fs/net manifest grants
or implement network capabilities. `ModuleManifest` parsing and swap list
comparison do not enforce actual Rust IO.

`examples/modules/rust_kv/` contains a manifest, data and shell script, no Rust
implementation. `run_demo.sh` prints PASS/ALL TESTS PASSED without running the
claimed filesystem, traversal or swap operations. Its optional modhost build is
not a substitute for those assertions. Existing `capstd_tests.rs` does exercise
a real `CapFs` traversal case when its feature is enabled; that test uses fixed
temporary directory names and one host path, and needs stronger isolated cases.

**Plan:** U01, U19–U22. Replace printed success with execution and an observable
oracle; fail or report unsupported when the required capability path is absent.

### UR02 — Accepted FFI types exceed the actual ABI (P0)

`lib/TypingPass.ml::is_buffer_protocol_type` accepts integer widths, booleans,
floats and recursively nested addresses/pointers. The JIT's `declare_import`
and function-header compiler in `safestos/cranelift/src/cps.rs` use I64 for
every argument and return; the return-type byte is read and ignored. A native
function expecting a float register or a different layout cannot be justified
by its name and argument count.

**Plan:** U04 restricts the supported baseline; U11–U15 add versioned typed
adapters and target checks. Test mixed register classes and asymmetric argument
vectors, not just the checksum's pointer/i64 shape.

### UR03 — Ownership vocabulary is not a cross-language lifetime proof (P0)

`TypingPass.ml::check_rust_ownership` checks annotation presence, arity,
vocabulary and type shape. The annotation is erased before CPS. Its ownership
labels do not bind Rust retention/drop behavior or provenance; `TypeSystem.ml`
classifies Address/Pointer as FreeUniverse. A raw pointer annotated owned does
not become an uncopyable owned Rust object.

The checksum demo's safe public `extern "C"` function dereferences a caller's
pointer after only null/length checks. A non-null pointer is not proof of a live
allocation or a readable length. The reference rsffi crate correctly makes its
pointer-taking functions unsafe, but callers still owe their documented contract.

**Plan:** U12–U16 use owned host handles, scoped borrows, explicit allocator/drop
ownership and safe generated Rust wrappers. No arbitrary pointer probes in the
agent process. Rust's aliasing rules still apply inside every adapter.

### UR04 — Cargo artifact selection can choose the wrong library (P1)

`umbrella/src/main.rs::cargo_build` selects the first compiler-artifact filename
ending `.so`, `.dylib` or `.dll`, without package ID, target name, crate type or
requested target validation. Dependency/procedural-macro artifacts can have the
same extension. It also lacks package/workspace/feature/profile selection.

**Plan:** U05 uses Cargo metadata plus exact artifact identity and rejects
ambiguity, stale artifacts, failed build-finished and foreign-target binaries.

### UR05 — Global native resolution has no provider identity (P0/P1)

Umbrella loads with `RTLD_GLOBAL`; the JIT can resolve imports from the process
namespace. A manifest grant authorizes a symbol name, not the loaded crate,
artifact digest, ABI version or implementation. Two crates with the same export,
a conflicting preloaded library, or a registered symbol can change resolution.
The static registry is process-global, can replace an existing address, and
must be populated before this thread's JIT initialization.

**Plan:** U08/U10 build an explicit per-build provider table, reject duplicate
or undeclared providers and bind resolved address to descriptor and library
lifetime. A small bridge host API is appropriate; keep Cargo out of the compiler.

### UR06 — Native code runs before manifest authorization (P1)

Umbrella executes Cargo before reading the manifest and dlopens the cdylib before
installing its authorizer. Build scripts, procedural macros and library
initializers can execute before an `rs_*` call is denied. Statements that no
foreign code executes on the denied path overstate the current evidence.

cap-std is an API discipline for audited native code. `capstd_mod.rs` explicitly
states that arbitrary Rust can use `std::fs`, raw sockets or unsafe code to bypass
it; hard isolation is a separate tier.

**Plan:** U02/U06/U09 separate build trust, native-load admission and call grants;
U26 validates a native isolation tier before offering untrusted execution.

### UR07 — Entry names do not establish callable signatures (P0)

Umbrella obtains a pointer by name and always invokes `execute_function`, which
transmutes it to a no-argument i64 function. It does not verify the entry's
parameter count or return convention. The parser accepts an empty entry; the
bridge's empty-name branch can select run, main, or a map entry. The bridge's
comment claiming it never selects an arbitrary library function does not match
that fallback implementation.

**Plan:** U04/U10 require an exact exported signature before invocation. Missing,
ambiguous, parameterized and empty entry names must refuse safely. Adding typed
invocation later must preserve the same check.

### UR08 — Multi-module emission and frontend errors are not a bundle contract (P1)

Umbrella accepts multiple module source arguments but reads one CPS file.
`lib/Compiler.ml` writes the configured CPS path while compiling each module;
it does not promise one linked bundle. Earlier library definitions may exist in
the compiler child's JIT while umbrella's fresh JIT receives only the last file.
`Compiler_cps.ml` also retains name-keyed foreign mappings globally; inspect
cross-module short-name collisions before expanding linking.

The frontend's JIT errors can be advisory and fall back to C while the process
succeeds. Umbrella should require a fresh, complete typed artifact and the
selected verification results, not infer completeness from child exit alone.

**Plan:** U07/U17 add a fail-closed emit-only artifact contract, explicit module
graph and collision tests. This review does not claim the compiler child executes
the submitted entry; the inspected compile path emits and JIT-compiles functions.

### UR09 — Temporary files and child processes lack robust lifecycle control (P1)

The CPS output is `/tmp/umbrella-<pid>.cps`; it is predictable, not in a private
directory, and is removed only on successful frontend return. The frontend
opens its output with ordinary file creation. Stale files, symlinks and failures
need explicit handling. Both Cargo and Austral use `Command::output()` with
unbounded output buffering, inherited environment and no deadline/child-tree
cleanup. Native code has no reliable hard deadline in the umbrella process.

**Plan:** U03/U06/U07/U23 use private scratch roots, bounded streaming diagnostics,
clear cancellation/deadline semantics and worker supervision.

### UR10 — Global authorization does not provide instance isolation (P1)

`auth.rs::safestos_load_auth_manifest` merges permissions into a global manifest
engine. The deployment principal is global; the JIT is thread-local. Loading a
narrower policy for the same name through that FFI path does not revoke old
merged grants. Existing tests serialize state or order denial before grant.

**Plan:** U09/U18 reuse the distributed plan's context-scoping work. For the first
production profile, one loaded generation per worker is a valid way to contain
legacy global state, provided lifecycle and revocation semantics are explicit.

### UR11 — Panic/error results are not handled as typed execution outcomes (P1)

The bridge can return `JIT_PANIC` plus last-error; umbrella prints every returned
i64 as `RESULT`, with no error-channel check. i64::MIN is also a legitimate
result. The bridge's panic tests use ordinary Rust function pointers and do not
establish that a panic escaping a real `extern "C"` cdylib is catchable. A panic
at a non-unwinding C boundary, explicit abort, segmentation fault or trap needs
subprocess testing. An outer catch_unwind is insufficient for these failures.

**Plan:** U15/U23/U24 require panic conversion inside generated C wrappers and
structured host status, plus child-process crash tests and correct exit codes.

### UR12 — CLI parsing accepts malformed inputs (P1, measured parser probes)

| Arguments to extracted parser | Observed behavior |
|---|---|
| `--help` | Exit 1, unknown flag |
| `--manifest-garbage grants.toml Demo.aum` | Accepted as a manifest option |
| `--entry --expect 7 Demo.aum` | Entry becomes `--expect`; `7` becomes a module |
| `--entry= Demo.aum` | Empty entry accepted |
| `-x Demo.aum` | `-x` accepted as a module |
| `--entry compute Demo.aum` | Expected parse succeeds |

**Plan:** U03 exact option matching, help/version, explicit `--`, unambiguous
negative numbers, raw OS paths, deterministic exit/JSON contracts and no side
effects on usage errors. Production tests must invoke the actual binary.

### UR13 — Reproducibility and installation contracts are incomplete (P1/P2)

The child build lacks `--locked`/offline/frozen policy and explicit feature/target
identity. Compiler discovery defaults to a compile-time checkout path; frontend
loading prepends a sibling bridge directory into LD_LIBRARY_PATH. This may pick
an incompatible bridge and does not establish an installed, relocatable toolchain.
The main loader is Unix libc code despite recognizing `.dll` artifact names.

**Plan:** U05/U25/U27 identify compiler/bridge/provider versions, lock inputs and
packaging boundaries; declare supported native platforms and reject others early.
Cross-building a foreign artifact is different from executing it in the native JIT.

### UR14 — Current tests do not establish the intended full feature set (P2)

The useful existing tests cover known scalar/buffer calls, name grants,
annotation errors, basic path traversal and grant-list escalation. Umbrella has
no orchestrator unit/integration suite of its own. Missing evidence includes
real fs/net grant enforcement, provider ABI compatibility, opaque Rust ownership,
typed methods/generic adapters, callback lifetimes, mixed workspaces, repeatable
incremental builds, diagnostics, resource limits, crash recovery and installation.

**Plan:** the matrix and task cards define each supported/unsupported behavior,
independent oracle and mutation control before stronger readiness claims.

## 4. Valen and the Rust fork: useful mechanisms, distinct trust boundaries

Valen's README explicitly calls it a prototype and notes incomplete String/
HashMap interop. Its code is a design reference, not a production certification.

| Source inspected | Useful lesson for umbrella |
|---|---|
| `Valen/src/typing/rust_interop/bifrost/orchestrator.rs` | Mixed-language project manifest, generated Cargo workspace, separate staging/build phases and toolchain bundle discovery |
| `.../oracle.rs`, `real_rustc_oracle.rs` | A mockable typed provider interface, qualified item identity, method/type resolution and synthesized Drop |
| `.../driver.rs`, `generate_final_rust_file.rs` | Explicit importer/typecheck phase and generated final linkage source; test intermediate artifacts separately |
| `Valen/src/instantiating/rust_interop/bifrost/extern_abi.rs` | Ask the compiler for calling convention/layout; do not guess from size or argument count |
| `.../per_instance_mir/`, `collect_and_partition_mono_items.rs` | Make foreign dependency/monomorphization requests explicit; test cold/warm builds because hook side effects can be skipped by caching |
| `.../deduced_param_attrs.rs` | A substituted body cannot inherit optimizer assumptions derived from a placeholder body |
| `Valen/src/typing/test/rust_interop/{drops,aliasing,resolution,driven}.rs`, `tests/rust_interop/valen_build_e2e.rs` | Test typing, ownership/drop behavior, qualified naming, codegen and real subprocess results independently |
| `Valen/examples/borrow_check_over_rust_boundary/` | Reject a borrowed subobject used after owner mutation replaces it; successful FFI calls alone do not test lifetime safety |

The Rust fork's top commit changes seven compiler files, with 95 insertions and
3 deletions according to `git show --stat`: a per-Instance MIR query and default
provider, use of that query by mono-item collection, an extra-codegen-modules
backend hook and LLVM module/context accessors. Valen then supplies its own LLVM
modules and dependency mentions. This is an AOT LLVM integration, whereas the
current Austral compiler remains CPS/Cranelift JIT.

Those hooks do not automatically extend Rust's borrow checker to arbitrary
generated replacement code. Valen handles its own source semantics and has
unimplemented ABI/type cases, optional borrow-check disabling, and an explicit
warm-build hook-side-effect workaround. Some demonstrations disable borrow
checking; those are not acceptance evidence for a safety-preserving profile.

Recommended adoption: a typed, testable import/adapter schema; generated Rust
wrappers that rustc checks; explicit monomorphized dependencies; ownership/drop
and invalidation tests; and reproducible toolchain identity. Keep a pinned
rustc-oracle plugin as a separately gated extension if direct-import ergonomics
require it. Reusing these ideas does not require embedding rustc or replacing
the JIT with LLVM. Neither a successful Rust build nor an annotation proves
unsafe native code follows the adapter's contract.
