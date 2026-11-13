# flux1d — a verified kernel spec surface (W1)

A one-dimensional finite-volume flux scheme whose three obligations are stated
as identities over a class of states, not as one closed computation.

| file | what it is |
| --- | --- |
| `Flux1d.aui` | the spec surface: `State`, `flux`, `numFlux`, `delta`, `residual` |
| `Flux1d.aum` | the execution side — the same functions, compiled and run by `austral` |
| `flux1d.obligations.json` | the reflected obligations W2 (`scripts/au2mm0`) consumes |

The kernel:

    f(x)        = 3x + x²              -- componentwise flux
    numFlux(l,r)= 2·f(l) − f(r)        -- two-point numerical flux
    delta(l,r)  = numFlux(l,r) − numFlux(r,l)
    residual(prev,cur,next) = numFlux(cur,next) − numFlux(prev,cur)

The obligations:

- **consistency** — `numFlux u u = flux u` (the two-point flux degenerates to
  the physical flux on a uniform state);
- **conservation** — `delta a b + delta b c + delta c a = 0` (the interface
  operator telescopes around a periodic cell cycle, so the residual sum is zero
  for every state vector);
- **symmetry** — `delta l r + delta r l = 0` (the two-point operator is
  antisymmetric in its arguments).

`make` compiles the module and runs it; `main` checks all three obligations on
one concrete state vector and exits non-zero if any fails. That is a *test of
the spec at a point* — the symbolic statement of the same three identities, over
free variables, is what `docs/kernel_alg.mm0` certifies.

## Why the obligations live in a sidecar

The plan's W1 prefers obligations carried in CPS IR, so the compiler is the
single source of truth. That carrier is a compiler change and this tree's
`dune` toolchain is not buildable here (`nix develop` fails on a missing
dependency), so the reflected AST is written next to the kernel and audited
against it. `au2mm0` reads the same JSON shape an obligation node would
serialise to, so moving the carrier later does not change the prover — the
decision and its fallback are recorded in `PLAN_sota_kernel_verification.md`
§9.

The certificate is over ℤ. `Int64` wraps, so a certificate covers the scheme
only for inputs that do not overflow; pinning that range is W4 and the
assumption is written into the certificate header rather than left implicit.
