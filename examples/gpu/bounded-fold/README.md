# Bounded fold (G17a/G17b)

One compile-time bounded `for` loop over one private `Nat32` accumulator, as
admitted by the `bounded-fold-v1` source profile and expanded into immutable
KernelIR locals ([contract](../../../docs/gpu/BOUNDED_FOLD.md)).

`upToZero` runs the body once and `upTo31` 32 times, because the endpoint is
inclusive; the loop index is deliberately unused. `main` prints the C-target
oracle values for `x = 7`.

```sh
# C oracle: 7, 224
./austral compile examples/gpu/bounded-fold/Fold.aui,examples/gpu/bounded-fold/Fold.aum \
    --target-type=c --entrypoint=Fold:main --output=/tmp/fold-c/Fold
cc -x c -O0 -o /tmp/fold-c/Fold.exe /tmp/fold-c/Fold -lm
/tmp/fold-c/Fold.exe

# WGSL bundle (33 immutable lets for upTo31) and its reference evaluation
AUSTRAL_GPU_CHECKER=safestos/gpu/target/release/avm-gpu \
  ./austral compile examples/gpu/bounded-fold/Fold.aui,examples/gpu/bounded-fold/Fold.aum \
    --target-type=wgsl --gpu-source-profile=bounded-fold-v1 \
    --entrypoint=Fold:upTo31 --output=/tmp/fold-bundle-31
echo '{"inputs": [[3, 7, 11, 4294967290]]}' > /tmp/fold-inputs.json
safestos/gpu/target/release/avm-gpu reference /tmp/fold-bundle-31 --inputs /tmp/fold-inputs.json
```

Without `--gpu-source-profile=bounded-fold-v1` the default profile refuses the
loop by name.
