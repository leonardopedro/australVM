# 019-extern-rust

PLAN_valen §4, tier 1: the `Rust_Ownership` pragma that annotates `rs_*`
foreign imports with per-parameter and return-value ownership.

This suite holds the **negative** tests: every malformed or misplaced
annotation is a hard error at typechecking, never a silent default (an
annotation the compiler ignores is an annotation the author believes is
enforced).

The **positive** path lives in the JIT tests, not here: the C backend emits a
link-time wrapper for every declared foreign function, so a C-target program
declaring `rs_checksum` cannot link against libc — the symbol only exists in
the Rust bridge's symbol table. The granted/denied end-to-end (compile → CPS →
JIT → execute, gated by the module manifest) is covered by
`test/JitTest.ml` (`rs_denied_without_manifest`,
`rs_denied_by_uk_only_grant`, `rs_granted_executes_checksum`) and
`safestos/cranelift/tests/tier1_extern_rust.rs`, which exercise the real
symbol table and the a10 authorization gate.
