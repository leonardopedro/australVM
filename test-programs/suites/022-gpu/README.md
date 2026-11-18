# GPU suite

The GPU backend's differential corpus lives in
`examples/gpu/pointwise/` and is driven by `scripts/gpu-source-check`, which
compiles the same source to a C executable and to a WGSL bundle and compares
both against `corpus.tsv`.

This suite keeps the authored scalar semantics on the ordinary test-program
path: `001-pointwise` reproduces the fixture bodies and their boundary values
under the C backend, so a language-level change that the GPU corpus would also
see cannot pass here silently.
