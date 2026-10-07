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

## Notes

This repo **hosts** the kernel modules `unfer` compiles for, and receives
their WhyML extensions over `lib/Compiler_plugin.ml`. The gate is a plugin like
any other, which is what keeps the trust boundary in one place.

Subprocess discipline is the same as unfer's: Why3 is invoked, never linked.
