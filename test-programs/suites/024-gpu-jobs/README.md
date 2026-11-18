# 024-gpu-jobs — stdlib/gpu/Jobs linear ownership gates

G15 evidence for `docs/gpu/G15_DESIGN.md`: these fixtures compile the real
`stdlib/gpu/Jobs.{aui,aum}` module (not a copy) together with a program that
misuses a linear token. Each is an expected *compile failure*, pinned to the
exact JSON diagnostic in `austral-stderr.txt`:

* `001-use-after-submit` — `free_buffer` after `submit` consumed the
  `OwnedBuffer`: `The variable buffer has already been consumed.`
* `002-double-receipt` — one `CompletedResult` passed to two receipts. The
  fixture frees the retained buffer through its context on the first receipt,
  so the only error is reusing the consumed receipt.
* `003-read-before-complete` — `read` applied to an `InFlightJob`; the type
  error names `CompletedResult` as expected and `InFlightJob` as actual.
* `004-leak-linear-buffer` — return from a function that never consumes its
  `OwnedBuffer`.

The fixtures use `--target-type=tc` in `cli.txt`; they never link or run, so no
`rs_gpu_*` provider is required. Positive type-checking of the module is itself
part of every run (a broken wrapper changes these diagnostics), and the
positive lifecycle — submit, pending, completion, one receipt, quarantine — is
covered by the Rust mock tests in `safestos/gpu/tests/service.rs`, because a
linked C fixture would need a provider for the v2 symbols.

Closing a context while a buffer is live cannot be a type error here:
`Context` and `OwnedBuffer` are separate linear tokens, so that rule is
enforced at runtime (`rs_gpu_close` returns `BUSY`) and is tested in Rust.
