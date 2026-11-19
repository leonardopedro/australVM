# Float32 numerical profile

G16a establishes Float32 source arithmetic before any GPU mapping: the
`TrappingArithmetic(Float32)` instance in `lib/builtin/Pervasive.aui`/`.aum`,
its CPS lowering (`au_f32_to_f64` -> `au_f64_*` -> `au_f64_to_f32`), and one
ordinary C-target fixture.

`001-affine-f32` runs the same affine and cancellation expressions at both
widths. Its expected output is the point of the case: a pass that widened
Float32 to Float64 would print `16777217.000000` and `1.000000` on the Float32
rows, because 16777216 + 1 is the first sum f32 cannot represent and 1e8 + 1
loses the addend entirely. The Float32 rows must read `16777216.000000` and
`0.000000`.

The v1 GPU subset still refuses Float32, and the `wgsl-f32-v1` profile (G16b) is
not part of this suite: no device execution is claimed here.
