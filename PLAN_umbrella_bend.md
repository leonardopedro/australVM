# Extend Umbrella to Rust, Austral and standard Bend

**Proposed, 2026-11-20.** Extend the existing Umbrella strategy. Invoke each
compiler as a tool, select and bind exact artifacts, and connect components
through typed adapters. Bend source builds with the upstream Bend compiler
and executes with its standard runtime. Austral continues through SafestOS
and Cranelift. Cargo builds the Rust code and the Rust-facing adapters. Use
the existing Valen `rustc` fork as the starting point for the narrow patched
Rust toolchain lane. Keep its hooks on the Rust side of the interface.

The first release builds an Austral entry, a Rust library and a standard
Bend program, then calls the Rust provider and a Bend operation through one
Umbrella project. Bend's public native build produces a whole executable.
Expose that executable through a generated Rust `cdylib` with the same
adapter ABI, artifact selection and call grants used for Rust providers.
The adapter runs the Bend artifact across a typed process boundary using a
small entry written in standard Bend. The reciprocal direction uses Bend's
public IO facilities. [Umbrella production plan](PLAN_umbrella_production.md),
[Bend native build and limits at the inspected revision](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/README.md#limitations).

## Existing code and the integration seam

| Existing work | How this plan uses it |
| --- | --- |
| `umbrella/src/cargo.rs` builds Rust and selects an exact Cargo package, target and cdylib from structured output. | Build the user Rust crate and the Bend adapter with the same selector. Keep the actual Bend program as a separate, pinned artifact. [Cargo's tool integration reference](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages) |
| `umbrella/src/main.rs` invokes Austral and loads CPS plus generated scalar wrappers. The prototype uses global native symbol resolution and an advisory CPS emission path. | Keep the existing two-language entry working. Share the U07 compile-only and U08/U09 exact provider/admission work before releasing the three-language profile. |
| `umbrella/src/interface.rs` and `safestos/cranelift/src/adapters.rs` define the typed scalar descriptor, wrappers and word encoding. U12 has a bounded buffer-descriptor reader. | Reuse the scalar format and codecs. Add binding metadata for the Bend executable and operation. Defer buffer transport until its bounds, mutability and ownership checks are implemented. |
| `umbrella/src/process.rs` supervises compiler children and separates signals from exit codes. Its current pipe drain buffers output and waits for EOF after the leader exits. | Extend this helper with output bounds and a deadline that covers pipe drain. Use it for the Bend compiler and native executable instead of adding another supervisor. |
| The local Valen Rust fork at `c6f757802519a0fc2619dd79e0be879601899b06` adds per-instance MIR and codegen hooks. Current Umbrella invokes ordinary Cargo/rustc and does not exercise those hooks. | Build and test the fork as a separate, pinned Cargo toolchain. Reuse U32 for Rust compiler queries if the adapter needs them. Keep ordinary Rust type/borrow checking on the generated wrappers. |
| Stock Bend at `0e2f38f759cc830849e489307f3d256a7d148288` reports 2.0.40 and exposes `--check-only`, `-o` for a binary or `.mjs`/`.bendtt`, and `--verdict`. It ships `IO.args`, `IO.print`, `Process.run`, and source-level C/JS effects. | Use real Bend CLI probes and document the exact toolchain/runtime selected. Use native binaries for the initial release and reserve `.mjs` for an explicit later development profile. |

[Umbrella execution record](docs/umbrella/EXECUTION.md),
[Rust fork review](docs/UMBRELLA_PRODUCTION_REVIEW.md#4-valen-and-the-rust-fork-useful-mechanisms-distinct-trust-boundaries),
[Valen Rust fork pin](https://github.com/valen-lang/rust/commit/c6f757802519a0fc2619dd79e0be879601899b06),
[Bend CLI](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/bend2/main.ts),
[Bend guide](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/guide/GUIDE.md).

## First release contract

```text
Rust source + generated Bend adapter → Cargo / pinned rustc → Rust cdylibs
Austral .aui/.aum                     → Austral → CPS → SafestOS / Cranelift
Bend .bend                            → stock bend -o → native Bend program

Austral caller → granted rs_* adapter → exact Bend program → typed result
Rust caller    → same typed client     → exact Bend program → typed result
```

- **Project.** Extend `--project` with an opt-in JSON schema version. Name
  the Austral entry, Cargo packages and targets, Bend programs and explicit
  call edges. Each edge identifies a consumer, provider, export, logical
  signature and ABI version. Keep project paths relative to the manifest
  and distinct from toolchain configuration. Use the same plan for source
  builds and verified prebuilt artifacts. Share graph normalization with
  U17 and keep existing schema-1 behavior covered by its current tests.
- **Initial adapter.** Accept a single `U32 → U32` operation. Generate a
  Rust adapter with the existing `rs_*` scalar conventions and a tiny Bend
  entry that calls the user's ordinary Bend function. Use an exact decimal
  U32 argument via `IO.args` and one versioned result record on stdout,
  for example `BEND_RESULT 1 42` followed by a newline.
  Reject malformed numbers, trailing records and out-of-range fields. Keep
  diagnostics on stderr. Reserve the process exit code for success/failure.
  Return a checked status and payload to the caller. For an existing
  one-word Austral return, use a 64-bit status/payload pair with an explicit
  decoder so a successful zero remains distinct from a failure. Hold binary
  path and runtime options in the admitted provider, outside guest input.
- **Authority.** Reuse Umbrella's separate build, load/admission and call
  decisions. Gate the Bend compiler tool and build inputs before launching.
  Authorize the exact `rs_*` adapter and bound Bend operation before spawn.
  Where the bridge still uses a deployment-global principal, keep one
  principal per trusted worker or wait for U09's instance context. A
  supervised child limits execution time and output. Filesystem, network
  and device permissions require the selected isolation profile. Enable
  untrusted use after U26 supplies and exercises that isolation.
- **Identity.** Bind the signature, adapter cdylib, Bend executable and any
  device sidecar to the project lock together. Include Bend/Base, build
  inputs, toolchain, target and compiler options in the build key. Resolve
  approved artifacts through private staging and recheck their identity at
  use. Cargo and Rust compiler output supply evidence for the Rust component.
  Bend supplies evidence for its own source and artifacts.

## Implementation cards

### UB00 — Establish the stock Bend and current Umbrella baselines

Record the Bend pin, `bend version`, host C compiler, current Cargo/rustc,
and the Valen fork revision. Add an original fixture with a pure wrapping
increment and an IO entry. Compile through stock `bend ... --check-only`,
then `bend ... -o <scratch>/bend-increment`, and run the emitted binary
directly. Exercise `0`, `2^31` and `2^32−1`. Start from the checked-in
Umbrella fixture and replay its existing Rust-plus-Austral result. Record
the exact runtime arguments and derive results from an independent integer
oracle. Record tool gaps as such. [Bend's U32 and IO standard library](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/bend2/base.bend),
[existing Umbrella demo](umbrella/README.md).

**Exit.** Both baselines have reproducible build and run commands and a
known output. An untypeable Bend source fails before a binary is accepted.
A mutated wrapping boundary returns the wrong word and fails the oracle.

### UB01 — Pin the narrow patched Rust compiler lane

**Depends on UB00. Reuse U25 and U32.** Inspect the existing Rust patch before
adding a hook. Pin the commit, bootstrap/sysroot identities and LLVM version.
Build an opt-in toolchain through the fork's ordinary bootstrap and invoke
it with a scoped Cargo `RUSTC` selection or pinned toolchain path. Keep the
standard Cargo JSON artifact selector. First build the same generated scalar
adapter from the existing Rust fixture with ordinary rustc and with the
patched rustc, then exercise both against its direct Rust oracle. UB04 uses
the pinned compiler on the actual Bend-facing adapter. Use exact signatures
and real Rust calls in the adapters. Add a small subprocess type/layout query
only if an actual Rust import needs information unavailable in the stable
descriptor.
Pass its result through the same schema. Leave MIR replacement and extra
LLVM module injection outside this scalar path. Preserve borrow checking.

**Exit.** The toolchain identity is visible in the recorded build and in the
cache key. The pinned patched Rust build produces a provider that passes the
same scalar oracle as the ordinary Rust build. A mismatched compiler or sysroot
gets a named error, and an ill-typed wrapper fails compilation. Ordinary
two-language projects keep their current toolchain selection.
[U32](PLAN_umbrella_production.md#u32--optional-pinned-rust-type-oracledirect-import-plugin).

### UB02 — Describe and preflight the three-language build graph

**Depends on UB00 and the reusable parts of U02/U17.** Extend
`umbrella/src/project.rs` and `policy.rs`. Implement only the nodes and graph
edges exercised by the first fixture. Omit automatic symbol and entry search.
Validate a single exact entry, each declared binding and each capability
before running tools. Include both compiler build authority and native
provider/child execution authority. Initially accept acyclic dependencies
and forbid duplicate qualified exports. Document the accepted JSON and
field-specific diagnostics.

**Exit.** Reordering node declarations preserves the plan. A missing provider,
duplicate export, disallowed call, wrong logical width, cycle or typo refuses
with the relevant node/edge named and with compiler and invocation markers
absent. Schema-1 project and CLI tests still pass.

### UB03 — Add the stock Bend build adapter

**Depends on UB02 and U06.** Add a small orchestrator in `umbrella/src/bend.rs`
using `Command` and the existing process helper. Run the configured upstream
CLI, beginning with local sources and checked build inputs. Stage under a
private directory. Enumerate source and foreign files, resolved dependency
identities and runtime sidecars. Reject unresolved remote dependencies in
the first offline profile. Select a target-compatible executable from that
exact build, validate that it is fresh, and record its identity before use.
With a requested GPU build, stock Bend can execute its GPU build step and
emit an adjacent `.gpu` file. GPU tool access and device requirements belong
in the build record. Start with an explicitly CPU fixture.

Extend `process.rs` in its own patch with reading limits and supervision
through stream EOF, including a leader that exits before its descendant
closes a pipe. Keep the existing distinction between spawn failure, nonzero
exit, signal, timeout and output limit. Use a fake tool only for malformed
artifacts and process edge cases. The normal integration test uses stock Bend.

**Exit.** Missing or stale output after a zero compiler exit refuses. Tests
cover version mismatch, unsupported target, hanging/noisy children, a binary
with no execute permission and an undeclared sidecar. An ordinary Bend build
still produces the expected result when invoked directly.

### UB04 — Generate the Rust provider and link one live three-language call

**Depends on UB01–UB03 and U07/U08/U10/U11 for the strict profile.** Reuse
`interface.rs` to generate the Rust-facing adapter. Put the subprocess
transport in a reusable Rust client used by that adapter. Generate standard
Bend IO wrapper source against a qualified ordinary Bend function and
compile it with UB03. Bind the native executable to the Rust provider's
operation descriptor. Admit the provider through the same exact-artifact
resolver and `rs_*` grant checks as other Rust libraries. Bind the descriptor
and the correct callable entry by provider identity rather than global symbol
search. Use the existing strict Austral entry invocation and avoid executing
library definitions during build or load.

**Exit.** An Austral entry uses a Rust library to prepare a scalar, invokes
the stock Bend computation through the Rust provider, then observes the
decoded result. A Rust caller also invokes the same operation using the typed
client. The project runs from a given manifest with an independent direct
Bend result as the oracle and uses the patched rustc lane. Add a denied run
with an observable child-start marker, and mutations for a changed executable,
wrong signature, ambiguous provider, success-shaped output on nonzero exit,
and a valid zero result. The supported path still uses ordinary compiled
Bend code and its runtime.

### UB05 — Add stock Bend callers of Rust and Austral operations

**Depends on UB04 and U09.** Expose a supervised Umbrella host command with
generated bindings for named, typed operations. Start with a Bend adapter
using standard `Process.run` and one request per invocation. The host command
owns authentication, provider selection and operation grants. The Bend
program sees a typed result or a typed failure via an IO action. It may
select an operation and submit values only within its declared interface.
The host must derive call authority from a launch-bound context it issued
and checked, rather than a caller-supplied principal. Use the absolute pinned
host command and its finite time and output limits.
Account for upstream `Process.run`'s scope by keeping child-tree cleanup
under the Umbrella supervisor. For long-lived calls consider a single scoped
local connection through ordinary Bend effects. Evaluate a standard
`import "./bridge.c"` effect only after measuring process costs. Upstream
splices C effects into its whole native program. An effect can call an
approved Rust C ABI export, with the same binding and lifetime checks.

**Exit.** A standard native Bend program invokes one Rust scalar export and
one Austral export through the host, with direct arithmetic oracles. A wrong
provider, missing grant, narrowed policy and helper failure produce typed
errors. Check cleanup after timeout. Keep each fixture's initial call graph
acyclic. [Stock Bend `Process.run`](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/guide/GUIDE.md#io-and-concurrency),
[stock C/JS effect mechanism](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/guide/EFFECTS.md).

### UB06 — Extend data and lifetime handling through the existing adapter plan

**Depends on UB04/UB05 and the relevant U12–U16 work.** Add bounded byte
values and tagged errors before any persistent handles. Use a copied
value across a Bend process or an explicitly brokered buffer transport with
checked length, encoding and maximum size. Keep each allocation with its
originating runtime. When a later use case needs a persistent resource,
issue scoped host IDs with owner, rights and provider generation. Reuse the
existing Rust destructor protocol for Rust resources and an explicit cleanup
operation on the Bend side. Retire adapters and old child generations after
pending calls and handles are drained. Preserve affine and linear obligations
at their respective source boundaries.

**Exit.** Empty and maximum-size values round-trip with independent oracles.
Malformed/truncated buffers, invalid handles, duplicate release and use after
generation replacement have separate tests. Build failure leaves the prior
admitted generation available under its original policy.

### UB07 — Record proofs at the boundary each tool actually checks

**Depends on UB04 and existing B1/PC07 evidence work as applicable.** Add an
optional evidence manifest. Record the Bend `--verdict` result and concrete
`.bendtt` export, the Austral obligation and its Why3 or MM0 result, and the
Rust adapter descriptor, actual artifact and relevant checked contracts.
Use the existing Lean-to-MM0 tools for supported BendTT/Lean proof imports.
Tie each check to its concrete source, subject, statement and assumptions.
Prove or validate the scalar protocol relation independently. A transferred
law about the stock Bend binary also requires a semantics-to-native edge.
List that premise explicitly while it is open. Reuse the existing MM0 checker
and statement approval when a project asks for strict proof admission.

**Exit.** Changing the program, word encoding, law, declared assumptions or
artifact invalidates the appropriate record. Whether execution passed, the
source law checked, and a native property has an MM0 derivation appear as
separate outcomes. [BendTT §8](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/paper/BendTT.pdf),
[translator scope](docs/lean4mm0.md), [MM0 proof contract](docs/PROOF_CHAIN.md).

### UB08 — Package and measure native CPU and optional stock Bend GPU profiles

**Depends on UB04 and U18/U25/U27 as applicable.** Pack the exact Rust
provider, Austral CPS and stock Bend artifact in the same project release.
Record required runtime libraries, source/toolchain hashes and any device
sidecar. Test offline replay from a relocated directory. Add a separate
hardware profile using an ordinary Bend `!` example compiled by upstream for
its target. Pin the documented runtime options and require an identified
device execution record for a GPU claim. Test explicit refusal when the
toolchain or runtime would select CPU for a requested GPU lane. Launch Bend's
binary through the same supervisor and leave its parallel work inside Bend's
runtime. Track host and child thread/device budgets as the project fan-out
grows. An `.mjs` preview can later validate fast iteration in its own runtime.

**Exit.** Locked CPU CI reproduces the three-language result. Artifact, build
profile, target or toolchain changes invalidate the affected binding. A
hardware job exercises the pinned Bend backend and reports device or refusal.
Compare cold/warm build times and direct/adapter execution for small and
batched calls. Add persistent workers only after measurements justify them.

## Handoff

Start with UB00's small paired baseline. UB01 and UB02 can then land
independently. UB03 and UB04 establish the first supported three-language
release. UB05 adds the reverse direction and the later cards extend features
independently. Keep fake-tool and real native lanes distinct in
[`scripts/test-umbrella`](scripts/test-umbrella) and record implementation
results in [the Umbrella execution ledger](docs/umbrella/EXECUTION.md).
Run Dune through the repository's Nix devshell, and use locked Cargo builds.
Keep the patched Rust compiler opt-in at the Umbrella boundary. The inspected
Bend pin is Apache-2.0. Record provenance and license notices per imported
component, including any separately licensed dependency if later used.
[Bend license at the pin](https://github.com/bendlang/bend/blob/0e2f38f759cc830849e489307f3d256a7d148288/LICENSE).
