# Bounded fold — the C oracle

G17 admits one compile-time bounded `for` loop over a private `Nat32`
accumulator (`bounded-fold-v1`, `docs/gpu/BOUNDED_FOLD.md`). This suite is the
C half of its oracle: the same source that the GPU path unrolls and the CPS JIT
executes, compiled through the ordinary C backend.

`001-c-oracle` compiles a copy of `examples/gpu/bounded-fold/Fold.{aui,aum}`
for `Fold:main`. Its `source-copy.txt` declares that origin, so the runner also
reports `001-c-oracle/copy`: that case fails if a copied module is missing,
extra, or differs in bytes from the example, which is what keeps the copy from
drifting when the fixture changes.

The program must produce

```
7
224
```

`upToZero(7)` runs the loop body once and `upTo31(7)` 32 times, so the two
lines together pin the **inclusive** endpoint (`from 0 to K` = K+1 updates) at
both extremes the profile allows. The entry is deliberately `main` rather than
a GPU entry: this case claims only that the C executable agrees with the hand
count, not that a bundle was published or a device ran. Those belong to
`scripts/gpu-source-check`, `docs/gpu/BOUNDED_FOLD.md` and the device lane.
