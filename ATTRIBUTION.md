# Attribution

Borrowed patterns and components that concern *this* repo. The authoritative,
cross-repo table — including what was deliberately **not** adopted — is
[`../ATTRIBUTION.md`](../ATTRIBUTION.md).

| source | licence | what was adapted | where it landed |
|---|---|---|---|
| Why3 | LGPL | the compiler gate proves the kernel's authorization semantics | `lib/why3_plugin/`, pinned `.mlw` + expected extraction |
| Theseus | MIT | capability-oriented architecture as the model for linear types | capability tokens, Cedar policy in the JIT |
| velyst | see upstream | not used directly; the compiler is reached over the plugin seam | — |
| SafestOS (same author) | Apache-2.0 with LLVM-exception (README; the in-tree copy has **no LICENSE file** — verify) | the C runtime: lock-free scheduler, cell loader, memory management | `README.md` §Architecture item 3; tree in-tree at `safestos/` |
| DeepSeek Engram paper (arXiv:2601.07372) | paper text arXiv-default (authors retain copyright); companion `../Engram` code is Apache-2.0 | the paper's **full text reproduced as a design note** — recorded, not normalised | `engram.md`, `docs/ENGRAM.md` |
| Valen (`valen-lang/valen`, Apache-2.0) + the group-borrowing article (Ovadia 2025, crediting Nick Smith; blog text © its authors) | Apache-2.0 — LICENSE read firsthand 2026-10-16 | **design ideas only, no code copied**: group borrowing as an erasing ("ghost") type-pass over linear cells, and Rust-library interop kept out of the lightweight JIT — as a plugin over the existing seam or a separate umbrella compiler. Not adopted: the patched rustc. | `PLAN_valen.md` |
| `SplitMix64` (Vigna's SplittableRandom mixer) | algorithm reimplemented from the published specification — no code copied; reference implementation public domain | the gear table of the in-house FastCDC chunker is expanded from the fixed seed `GEAR_SEED` via the SplitMix64 mixer and pinned by a test (the `fastcdc` crate was not used) | `safestos/pkg/avm-cas/src/chunker.rs`; claim recorded in `PLAN_avm_pkg.md` §4.1 |
| `Vykar` | **no checkout in this workspace** — design-level adaptation taken from the engineering spec's description; licence unverified until a checkout exists (verify before any code-level reuse) | the cold CAS's block-engine *design*: append-only packfiles with a chunk index, sub-file deduplication, manifest-carried chunk lists, Ed25519 replacing Vykar's AES — adapted as **design only** from Vykar's block engine (the spec's "Vykar Adaptation"); no Vykar code was read or copied | `docs/pkg_manager_plan.md` §3.2; implemented in `safestos/pkg/avm-cas/src/engine.rs` |

## Notes

This repo **hosts** the kernel modules `unfer` compiles for, and receives
their WhyML extensions over `lib/Compiler_plugin.ml`. The gate is a plugin like
any other, which is what keeps the trust boundary in one place.

Subprocess discipline is the same as unfer's: Why3 is invoked, never linked.
