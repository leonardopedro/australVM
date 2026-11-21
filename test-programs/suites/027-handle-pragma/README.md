# 027-handle-pragma

`Handle_Ownership` is an import contract, and the compiler enforces its shape
rather than trusting the author's annotation: the pragma names the provider
symbols the generated `__new`/`__use`/`__dispose` wrappers call, and the import
it annotates must have the ABI those wrappers are generated with.

`000-dispose-signature` refuses a dispose call whose first word is `Int32` — the
C wrapper takes two `int64_t`s, so a narrower declaration could not name a
wrapper that exists. `001-not-foreign` refuses the pragma on a function with a
body: an annotation the compiler silently ignored would be a contract the
author believes is enforced.

`umbrella/modules/WidgetHandle.aum` is the module the pragma exists for; these
fixtures keep every provider symbol undeclared, because each case is refused
before any link.
