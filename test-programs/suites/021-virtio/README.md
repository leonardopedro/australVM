# 021-virtio — stdlib/sys/Virtio.aui linearity gates

Grounded plan: `docs/spectrum_plan.md` §3.1 / Sprint 6 task 6.3.

These tests compile the real `stdlib/sys/Virtio.{aui,aum}` module (not a
copy) together with a driver program:

* `001-ownership-roundtrip` — typecheck + run gate: allocate → submit
  (buffer consumed) → complete (ownership restored) → read length → free
  exactly once, printing the buffer length.
* `002-use-after-submit` — expected *compile failure*: touching the
  `DmaBuffer` after `submit` consumed it is a linearity error, pinned to
  an exact JSON diagnostic (`austral-stderr.txt`).