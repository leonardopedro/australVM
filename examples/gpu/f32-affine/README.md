# `wgsl-f32-v1` fixture (G16b)

`F32Affine.aui/.aum` is the source half of the explicit Float32 numeric profile:

```sh
./austral compile examples/gpu/f32-affine/F32Affine.aui,examples/gpu/f32-affine/F32Affine.aum \
  --target-type=wgsl --gpu-numeric-profile=wgsl-f32-v1 \
  --entrypoint=F32Affine:axpy --output=/tmp/axpy.avkg
```

`axpy` is `2.5*x + y`, `cancel` is `(x + 1) - x`. The values are chosen so the
profile's width is observable: `cancel(16777216.0)` is `0` in binary32 and `1`
in binary64, and `axpy(16777216.0, 1.0)` rounds to `41943040` rather than
`41943041`. `Refusals.aum` holds the constructs the profile must reject
(Float64, division, float-to-integer conversion).

`scripts/gpu-f32-profile` compiles both entries, validates the bundles with
`avm-gpu`, compares the KernelIR reference bit for bit against an independent
Python binary32 oracle, and requires each refusal to name what it refused. With
`--require-hardware` it also dispatches the bundle and applies the profile's
2-ULP budget (1 ULP per admitted operation); without an adapter that is printed
as a `[gap]`, never as a pass.
