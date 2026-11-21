# Changelog

All notable changes to australVM are documented here.
Generated from conventional commits via git-cliff.

## [Unreleased]

### Bug Fixes
- Follow the §W3 approvals in the certificate and the controls
- Read the export lazily so emit stays bounded
- Re-pin the certificate's obj digest through the fresh bridge
- Stream the spool's big pieces so the ns row fits in 10 GiB
- Aarch64 spin-pause hint — the scheduler was x86-only
- Clear clippy -D warnings — the bridge job has been red since 652376e
- Report a missing interpreter as a gap, not a failure
- Namespace JIT exports so two `run`s in one thread both work (X4a)
- Silent JIT loss on float constants, a leaked temp file, an unquoted why3 path
- The `.mlw` half of the attestation was never checked; CI never ran the gate
- A 2^62-byte DMA transfer passed the 256 KiB SRAM gate
- Don't vendor the SpectrumOS patches; download+verify at setup time

### Build
- Carry z3 in the devShell so the fold-golden gate proves
- Mirror unfer's aarch64 fp16 rustflags so the README commands build here
- NixOS launch path — tree binaries, booted-system kernel, virtiofsd devShell
- NixOS-adapt the recipe — env bash, no /bin/echo, no .deb
- A15 -- gate the bridge on freshness, and make `make bridge` actually rebuild it
- Make test picks the loader, and diagnose it with `make loader` (X4b)

### CI
- Provision metamath/{set.mm0,set.mmu} so the statement path verifies
- Run the safestos/pkg engine tests — green but claimed by nobody
- Materialize the workspace-root ATTRIBUTION before check-links
- Pin third-party actions to SHAs; drop red macOS workflow
- Generate lib/BuiltInModules.ml before dune runtest
- Re-trigger after pushing sibling deps (unfer 7449571, dynamic-arctic ca11f49)
- Make workflows actually runnable — sibling clones, fix YAML, add arctic dep

### Documentation
- Record the ocaml shell after the §W3 approvals
- §13.47's 2 400 MiB recipe is measured, not inherited
- Cite the proto row's tree without a rebased hash
- Record the ocaml shell gate on the post-cleanup tree
- Write the ns row's run recipe at a 2 400 MiB cap
- Note that §13.22's working directories were reclaimed
- Record the lean gate after reclaiming build caches
- Re-measure both shells after the merge and bridge rebuild
- §13.45 measures the ns row's floor at 2 400 MiB
- Correct §13.43's sampled peak and measure the ns row's cap floor (§13.44)
- Measure the ns row in the shared format after the wrap fix (§13.42)
- Measure the ns row in the shared format after the wrap fix (§13.42)
- The ns row under 6 GiB, measured
- Record a complete theory for the ns row and the stop's prefix against it
- Record the shared certificate, the stop-emit prefix proof and the walk's measured costs
- Keep §13.32's part bound in step with the segment index
- Name where the nsOuterFock row's memory is
- Write down what the third streaming round bought and where the wall moved
- Record that the emit rows now emit and what still stops them
- Account for the emitter's 10 GiB per obligation (§13.25)
- Restore the celld source row lost with the workspace ledger
- Recount status block 134 -> 161 dune / 18 suites — re-verified 10-25
- Pin the trust split per def — structural vs grounded, tagged in the artifacts
- Name the workspace-root table as a path, not a link
- Record the nix-shell/dune glibc poisoning — run dune under nix develop
- Re-verify 134 dune / 16 + 108 pkg + 129 bridge — 10-18 -> 10-20
- Recount 113 -> 134 dune / 16 suites; add the unclaimed Rust surfaces
- Recount status block from today's log — 113 dune / 14 suites
- PLAN V — Valen-inspired group borrowing + Rust interop (P0)
- Surface audit — check-todo-ledger documented (federation P5)
- Changelog, TODO ledger, honest status recount after bridge rebuild
- Inventory scripts and gates with their nix wrappers; join the link gate
- Per-repo attribution ledger (X9)
- Replace "PROBABLY NOTHING WORKS" with the measured state
- Note pinned 1.97.1 Rust toolchain for the bridge

### Features
- Gate the bounded-fold lane and land G22a map-sum schedule
- A shared record references its node table, not its texts
- Roll an obligation part inside a certificate
- Keep the obligation index durable across a kill
- Split the emitter's RSS with tracemalloc and mallinfo2
- Sweep every chapter export, and close the two gaps it found
- Link certificate and proof rows straight to the Zenodo record
- Sandbox exchange with an Austral policy kernel and a Zenodo proof ledger
- Carry the quasi-Riemann statement of record into set.mm (Wave 3)
- Count borrow depth, drop t_unbor_idle, certify a second fixture
- Certify the linearity ledger under mm0-c
- Prove every golden — parameter fold-steps, linked VCs, z3 gate
- Emit §5.9's init/step premises and body-link return goals
- Second Lean source — Layout certificates, generic ADT layer, phase-d CI
- Conversion gate on uk_austral_defeq — exact, with fallback
- Phase D — Lean4→DeltaNets→MM0 certificate emitter, mm0-c verified
- Avm-pkg content-addressed package engine — PLAN_avm_pkg.md
- Extern rust tier 1 + umbrella prototype — PLAN_valen.md (P2/P3)
- Group borrows — P1 of PLAN_valen.md (Valen-inspired)
- Celld CLI contracts — strict values, --help, --json NDJSON
- Register uk_summarize in UNFER_SYMBOLS (C6)
- Register uk_memory_append / uk_memory_read in UNFER_SYMBOLS
- Register uk_events_poll in the JIT bridge (S29)
- CNL formalization gate as a compiler plugin
- P11.20 arctic-auth feature + P11.24 Zenodo Austral bindings
- P6 B3 — full end-to-end hot-swap with state migration
- Add kernelEvolveStr convenience for QFM module

### Other
- The four §W3 statement sets
- The entry fixture's front-end writes the adapters document
- Carry a float literal as its IEEE-754 bits
- Execute an Austral entry through the generated adapters
- Build the scalar adapters from the emitted descriptor
- Stop failing on bytes and theories this checkout cannot produce
- Re-check the Kopperman/Mehler overlay on its own
- Certify the Kopperman Hilbert-space fragment too
- Pin 1.99.0 (cranelift bridge + arctic_authority) — one compiler
- Row SafestOS and the reproduced Engram paper
- Record the keying ablation (unfer 937d56c)
- Index-carrying types over TRL terms, with erasure checked not asserted
- Mathed_kernel hosted module sample — Jupyter-compatible, grants not isolation
- Group non-major action bumps into one PR
- Remove dangling quote from demo_module step
- Fix demo_module step exec form (missing inner `-c`)
- Run demo_module gate via the canonical module_builder driver
- Fix bridge .so loading on fresh runners (make test + jit-stack steps)
- Make `bridge` regenerate BuiltInModules.ml like `make` does
- Fix nix develop exec form (exit 127) in build-and-test.yml
- Migrate build-and-test.yml from nix-shell (23.05) to the flake
- Panic guard must report the payload and the entry point
- Force-run the panic-guard OCaml suites (CliEngineTest / JitTest)
- Pin nixos-unstable so the JIT bridge and OCaml toolchain share a glibc
- Register Austral.Pervasive::trapping* Int64 arithmetic intrinsics
- Skills registry dispatch (uk_skill_* symbols)
- Register uk_logos_compile in UNFER_SYMBOLS (S31)
- Register uk_symbolic_simplify in UNFER_SYMBOLS (S30)
- Lock nanoda_lib git pin (S29 dependency)
- Register uk_proof_verify in UNFER_SYMBOLS (S29)
- Cross-repo contract + JIT fixes and hygiene
- Sync bridge UNFER_SYMBOLS to 65/65 + fix jit-stack CI
- Plan R uk_cert_* loopback marshaling + sensitive gate
- Metering, sensitive latch + secret vault at the loopback (F24/F25/F26)
- Resource caps + per-call deadline (F13)
- Sidecar supervision & auto-restart (F11)
- Loopback peer lockdown via SO_PEERCRED (F10)
- Egress boundary — enforce [grants] net (F9)
- Capability-minting chokepoint (F5)
- Observer re-check on shared reads (F8)
- Loopback caller/audit chokepoint (dispatch_loopback_as caller tags, audit trail, uk_audit_*/uk_agent_* dispatch arms)
- Per-instance sidecars + initialize_from_blueprint (instance isolation, F3/F4)
- Effects grant namespace + gatekeeper/client deferred-approval wiring
- OS sandbox layer for the workerd ECMAScript sidecar (browser-equivalent)
- Egison pattern matching — sweet-egison TH quasiquoters on GHC 9.10.3, fock_match module with Multiset Wick contractions + JIT-path tests
- Federation-aware module hosting
- Cap-std Rust modules — capability coupling in two isolation tiers
- Tidepool modules — Haskell effect-stack authoring path (infrastructure)
- Genuine module hosting — load-once/call-many/hot-swap
- Final root hygiene — archive stale status files, remove test artifacts
- Hygiene — archive stale docs, purge build artifacts, gate debug output, de-fragilize C boundary
- CI for new stack (cranelift JIT + safestos integration)
- Test harness wiring (scheduler.c noreturn, JitTest.ml, e2e --use-cps-jit)
- Unit-test cps.rs (18 CPS + permission tests)
- Live UK-4001 enforcement at JIT call time
- Uk_*/uz_* symbol auto-sync test + data-driven registration
- Build robustness + bridge artifact; fix Zenodo cross-repo edits
- Byte-buffer constants + Span lowering + record sizing (gap §9)
- Fix let-init, record destructure, cross-module calls (gap §9)
- Add linear Model wrapper to UnferKernel bindings
- Drop uk_subscribe/uk_poll JIT symbols + Austral bindings
- Fix MIfExpression to emit 0x02 (If) op with dynamic offset calculation. Ensures proper branching for CPS JIT.
- Fix binary format to match Rust expectations
- Fix read_u64, loop logic, and error reporting
- Correct indentation for `else`. (#579)
- Correct indentation, fix #572. (#573)
- Prefix identifiers that are C keywords, fix #566. (#570)
- Indent function body too. Fix #568. (#569)
- Add a function to read a line as a String from stdin (#555)
- StringBuilder (#554)
- Add functions to get spans from a Buffer (#549)
- S/Reference/&, s/WriteReference/&!
- We can't consume a linear variable in the condition of a while loop (#506)
- See how big the nix store is
- No proper type checking when assigning to a variable
- Return nil
- Only import public typeclass instances
- Qualified identifiers
- Don't import the pervasive module in the pervasive module
- Clarify
- Closing thoughts
- More syntax thoughts
- Syntax thoughts

### Performance
- Wrap the proof's binders as pieces, not copies
- Compare the goal wrap piecewise instead of joining it (§13.43)
- Wrap the goal type in one allocation
- Keep export records for the encode, not the run
- Memoise the context text per environment
- Write every axiom line as pieces
- Spool the statement and the substitution lines
- Encode terms by appending tokens, not by joining per level
- Stream the artifact's assumptions and count rules
- Stream obligations and memoise `to_tree`

### Refactor
- Git submodule -> plain subfolder
- Move into this repo (was a standalone dir); add cloud_hypervisor_vm recipe

### Tests
- Check the bridge the gate loads is the built artifact
- Gate the byte-identity claim in three modes
- A case that the loader path came from the runner
- Let the runner find the bridge library itself
- Require lean2mm0 verify to refuse a corrupted set.mmu proof
- A10 -- pin the live-JIT module-grant gate with a test of its own

## [0.2.0] - 2023-06-27

### Other
- Add a function to read a line as a String from stdin (#555)
- StringBuilder (#554)
- Add functions to get spans from a Buffer (#549)
- S/Reference/&, s/WriteReference/&!
## [0.1.3] - 2023-06-11

### Other
- We can't consume a linear variable in the condition of a while loop (#506)
- See how big the nix store is
## [0.1.1] - 2023-01-20

### Other
- No proper type checking when assigning to a variable
- Return nil
## [0.1.0] - 2022-09-02

### Other
- Only import public typeclass instances
## [0.0.5] - 2022-06-21

### Other
- Qualified identifiers
## [0.0.1] - 2021-09-10

### Other
- Don't import the pervasive module in the pervasive module
- Clarify
- Closing thoughts
- More syntax thoughts
- Syntax thoughts

