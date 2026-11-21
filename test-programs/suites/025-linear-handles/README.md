# 025-linear-handles

U13: the module side of an owned Rust value. A handle is a **linear** record of
the two 64-bit words a generated `__new` wrapper writes, with `adopt` as its
only constructor and `dispose` as its only consumer — so "destroy it exactly
once" is a type obligation, not a convention.

`000-adopt-and-dispose` is the intended path: adopt once, dispose once.
`001-forget-the-handle` is a leak the linearity checker must refuse, and
`002-use-after-dispose` is a use-after-free it must refuse.

The real module that calls the provider is `umbrella/modules/WidgetHandle`;
these fixtures keep the record local so each case is one file with no foreign
symbols to link.
