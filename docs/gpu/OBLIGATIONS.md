# GPU memory-plan obligations (G13)

This is the obligation specification for the v1 identity map wrapper
(`docs/gpu/KERNEL_V1.md` §3). It is generated from the **published bundle**:
`lib/gpu/Gpu_obligations.ml` reads `kernel.json` and `kernel.wgsl`, rebuilds
the KernelIR, recomputes the canonical IR digest and the shader digest, and
refuses the bundle if its memory plan is not the admitted plan.

The obligation set is bound to four identities:

| Identity | Source |
|---|---|
| `bundle_sha256` | SHA-256 of the exact `kernel.json` bytes |
| `ir_sha256` | recomputed from `kernel_ir` (must equal the declared field) |
| `wgsl_sha256` | recomputed from `kernel.wgsl` (must equal the declared field) |
| `plan_sha256` | SHA-256 of the canonical memory-plan object |

Changing the plan, the guard, the output index or the launch extent changes at
least one of those identities, so old evidence cannot silently describe a new
wrapper.

## The obligations

Let `N` be the job length, `0 ≤ N ≤ 1048576`, and let
`groups(N) = ceil(N / 64)`. `gid` is the wrapper's `global_invocation_id.x`,
`flat(gid) = gid`, and an invocation is *active* when `gid < N`. The wrapper
is the emitted canonical code:

```wgsl
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&input0)) { return; }
    output0[i] = scalar_entry(input0[i], input1[i]);
}
```

The WhyML file (`gpu_memory.mlw`, emitted beside the manifest) states these
goals:

| Goal | Statement | Why it matters |
|---|---|---|
| `groups_nonneg` | `in_range N → groups(N) ≥ 0` | host launch arithmetic is defined |
| `groups_bound` | `groups(N) * 64 ≤ 1048576 + 63` | no missing or extra workgroup at the tail |
| `launch_representable` | `groups(N) * 64 ≤ 4294967295` | the flattened launch bound fits u32 |
| `index_representable` | `0 ≤ gid < groups(N)*64 → 0 ≤ flat(gid) ≤ 4294967295` | the generated flat index is a representable u32 |
| `guard_implies_bound` | `len = N → 0 ≤ i < len → i < N` | after the guard, an active index is in `[0,N)`; the runtime supplies `len = N` |
| `flatten_injective` | `wg₁*64+lid₁ = wg₂*64+lid₂`, `0 ≤ lidᵢ < 64` → `wg₁=wg₂ ∧ lid₁=lid₂` | workgroup/local decomposition is unique |
| `distinct_active_writes` | active `g₁`, active `g₂`, `g₁≠g₂` → `out_index(g₁) ≠ out_index(g₂)` | two active invocations cannot write the same cell |
| `write_index_unique` | active `g₁`, active `g₂`, `out_index(g₁)=out_index(g₂)` → `g₁=g₂` | the contrapositive used by the write rule |

`out_index` is defined as `flat`; the wrapper writes `output0[i]` with the same
`i` it guarded. `flat` is `gid.x` itself, so the distinct-index goals are
discharged by the index identity and the injective decomposition, not by an
assumption about `output0`.

## Runtime premises (not axioms)

These are checked by the host/checker and are **listed as premises** in the
manifest rather than asserted in Why3:

1. **Sufficient input extents.** `run` binds each input with extent `N`; the
   guard compares against `arrayLength(&input0)`, so `len = N` is a runtime
   fact, not a shader fact.
2. **Fresh output.** The runtime allocates a separate read-write output buffer;
   the wrapper has no alias construction. This is trusted code, not an OS
   sandbox.
3. **Distinct invocation ids.** Hardware invocation semantics provide distinct
   `global_invocation_id.x` values for distinct invocations in a dispatch.

The manifest names all three. The Why3 goals deliberately do not assume them
except through the explicit `len = N` hypothesis in `guard_implies_bound`.

## Evidence split

The two routes are separate outputs:

- **Why3** — `scripts/gpu-obligations check` runs
  `why3 prove -P alt-ergo` over `gpu_memory.mlw`. Why3's `int` is
  mathematical; every index goal carries the u32 bound, so no unbounded
  integer is narrowed to a native int. A pass says the arithmetic/index goals
  above are valid under the stated `len = N` bridge.
- **MM0** — `gpu_memory.mm0` states the same vocabulary and obligations, but
  declares no `theorem` or `axiom`: it carries `-- certificate: [gap]` and
  `-- proves: 0 theorems`. PC06's application of the composition to this plan
  exists (`mm0/proof_chain/gpu_plan.mm0`, `docs/proof/GPU_PLAN.md`), but the
  plan's residual premises O1–O5 are not MM0-discharged for an artifact, so
  this file stays statements-only.
  Why3 success does not discharge an MM0-required claim, and the checker
  reports `mm0=gap` instead of inventing one.

## Commands

```sh
# optional: a missing engine or MM0 toolchain is a visible gap
python3 scripts/gpu-obligations check /tmp/g10.avkg

# required proof profile: no Why3, or an unproved goal, exits 1
nix shell ".#why3" ".#alt-ergo" -c \
  python3 scripts/gpu-obligations check /tmp/g10.avkg --require-proof

# the same obligation lane through the bounded gate
nix shell ".#why3" ".#alt-ergo" -c \
  python3 scripts/check-gpu source --require-proof
```

The emitted directory contains `gpu_memory.mlw`, `gpu_memory.mm0`,
`gpu_memory.mm1` and `manifest.json`. The manifest records:
`why3.status=pass|gap|fail`, `mm0.status=gap`, the four identities and the
runtime premises.

## Negative controls

`test/GpuObligationsTest.ml` builds a real `add` bundle and checks:

- the accepted bundle has entry `add`, arity 2, workgroup size 64, bound
  1048576 and all eight goals;
- changing `output_index` to `linear` is refused by `Gpu_ir.check_kernel`;
- setting `guard_before_load` to `false` is refused;
- changing `requirements.max_elements` to 1048575 is refused;
- appending a byte to `kernel.wgsl` is refused by the shader digest.

No `axiom` or `theorem` may appear in the WhyML file; the test checks the
declaration form, not just the word in a comment.

## Not covered

- Renderer/model correspondence: the shader text is checked for the exact v1
  wrapper, but this is not a semantic proof that WGSL's storage accesses match
  a memory model. That is PC05/PC06 work.
- General GPU memory safety, arbitrary N, arbitrary workgroup sizes, races
  beyond the identity map, and non-identity output indices are outside v1.
- Runtime allocation, binding extents and hardware invocation semantics are
  trusted; no OS sandbox or adversarial bundle authentication is claimed.
- MM0 approval: the statement artifact is not in `mm0/approved/` and must not
  be auto-approved. Closing PC06's residual obligations for the artifact can
  turn it into a certificate; until then `[gap]` is the honest state.
