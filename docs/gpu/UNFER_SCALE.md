# One unfer workload on an Austral-authored kernel (G18)

G18 asks whether the kernel compiler can serve an operation unfer already
performs. The operation chosen is the amplitude scale behind
`QuantumState::scale_and_add` — every component of a sparse Fock state
multiplied by the same real factor — because it is the smallest operation that
is (a) already in the solver's linear combination, (b) expressible in the
admitted `wgsl-f32-v1` closure as one `trappingMultiply`, and (c) checkable
against unfer's own arithmetic rather than a re-implementation.

Nothing here replaces the SIRK solver, sparse basis construction, whitening or
the physics acceptance suites. The integration is an optional path dependency
and one example.

## Pieces

| piece | where |
|---|---|
| source kernel | `examples/gpu/unfer-scale/UnferScale.aui`/`.aum` (this repo) |
| bundle | published by `austral compile --target-type=wgsl --gpu-numeric-profile=wgsl-f32-v1 --entrypoint=UnferScale:scale` |
| host | `fock_sirk/examples/austral_pointwise.rs` (feature `austral-gpu`) |
| dependency | `avm-gpu = { path = "../../australVM/safestos/gpu", features = ["device"], optional = true }` |
| check | `python3 scripts/gpu/unfer-scale` |

The bundle is compiled **before** the Rust build and handed to the example by
argument (or `AUSTRAL_GPU_BUNDLE`); no build script launches the OCaml
compiler. `avm-gpu` has no unfer or JIT dependency, so `unfer → avm-gpu → …`
cannot cycle back.

## The operation and why the comparison is exact

`UnferScale:scale(x) = trappingMultiply(x, 2.5)`.

The scale factor is a source literal because the v1 bundle ABI carries arrays,
not scalars: a scalar parameter would be a new ABI, which is out of this card.
2.5 = 5/4 is exactly representable in both binary32 and binary64, so the
product of any two f32 values of that form is exactly representable in f64.
That matters: the kernel's multiply cannot be a rounding argument, so the
comparison can be **bit for bit** instead of a tolerance band.

The host flattens the sparse state through the existing
`TensorState::from_quantum_state` (the dense image whose layout bijection is
already asserted at upload), narrows each amplitude to f32 explicitly, sends
real and imaginary parts through the same kernel (two dispatches, one per
component), and compares each returned word to unfer's own
`QuantumState::scale_and_add` evaluated in f64 on the *narrowed* amplitude.

## Measured on this host (Intel UHD Graphics 630, Vulkan)

```
[pass] G18 kernel: UnferScale:scale published as wgsl-f32-v1 / f32 (ir=68953416064e)
[pass] G18 feature-off: `cargo build -p fock_sirk` (no features) still builds
[pass] G18 oracle: 6 value(s) of 3 complex amplitude(s) match unfer's f64 scale cast to binary32 bit for bit
[detail] G18 transfer: 24 byte(s) uploaded and the same back per component; prepare 27.020 ms,
         upload 0.339 ms, submit 93.862 ms over 8 submission(s), readback 1.063 ms; explicit copies, zero_copy=false
[detail] G18 cost: unfer's CPU oracle took 0.0049 ms for the same scale; the narrowing f64 -> f32
         changed 1 of 6 rounded value(s) relative to scaling in f64 and casting once
[pass] G18 control refused: a u32-modular-v1 bundle under the f32 example  (or at compile time)
[pass] G18 control refused: a kernel scaled by 3.0, with the mismatch count (5) naming the comparison
```

Two numbers are the point of the exercise:

- **Submission dominates.** 93.9 ms of submit against 0.0049 ms of f64 oracle
  for six values. That is not a defect of the kernel: a three-component state is
  three workgroups of one element, so the measurement is dispatch overhead. On
  this evidence the integration **hurts** end-to-end time, and no solver
  replacement is proposed. A crossover would need a state large enough for the
  per-element work to exceed submission, which this card does not claim to have
  found.
- **The narrowing is a visible cost.** Narrowing each amplitude to f32 *before*
  the multiply changed 1 of 6 rounded results relative to scaling in f64 and
  casting once. The oracle is therefore defined on the values the kernel was
  handed — which is the only comparison that can be exact — and the difference
  from the f64 chain is reported rather than hidden. Candle tensors and wgpu
  buffers are not interchangeable: the transfer is an explicit copy each way,
  measured above, and **not** zero-copy.

## What the controls show

The comparison is live in two ways, and both must fail:

- a `u32-modular-v1` bundle is refused by the example (the f32 profile is
  required, not assumed) — or, for the f32 fixture itself, refused by the
  compiler, since Float32 is not admitted under the integer profile;
- a fixture whose kernel computes `3.0 × x` while the oracle still scales by 2.5
  is refused with a non-zero mismatch count. Without that control a run that
  compared nothing could look like a pass.

## Not claimed

- No hardware run is claimed on a host without a physical adapter: the example
  exits 77 and `scripts/gpu/unfer-scale` reports `[gap]`, never a pass.
- The kernel is f32 while the solver state is f64; the conversion is explicit
  and its rounding effect is measured, but no error bound between the two
  widths is proved.
- No zero-copy, no Candle/wgpu buffer sharing, no change to the solver's
  numerical path, and no MM0/Why3 obligation for this bundle.
- The corpus is three amplitudes. It exercises the plumbing and the oracle, not
  a performance claim.
