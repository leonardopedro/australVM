# flux1d — a verified kernel spec surface (W1)

A one-dimensional finite-volume flux scheme whose four obligations are stated
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

The obligations (four):

- **consistency** — `numFlux u u = flux u` (the two-point flux degenerates to
  the physical flux on a uniform state);
- **conservation** — `delta a b + delta b c + delta c a = 0` (the interface
  operator telescopes around a periodic cell cycle, so the residual sum is zero
  for every state vector);
- **symmetry** — `delta l r + delta r l = 0` (the two-point operator is
  antisymmetric in its arguments);
- **uniform state** — `l = r ⇒ delta l r = 0`, the one obligation with a
  hypothesis. A hypothesis is an assumption the reader cannot check inside the
  artifact, so it is counted in `-- assumed: 1` and pinned in `BUDGETS`; the
  artifact also carries `-- witness: uniform_state: l=1 r=1 …` so the
  obligation is visibly not vacuous. Elimination is by substitution (MM0
  theorems carry no hypotheses): the statement proved is the identity restricted
  to the locus `l = r`.

`make` compiles the module through `--use-cps-jit` and runs it; `main` checks all
four obligations on concrete state vectors and exits non-zero if any fails. That
is a *test of the spec at a point* — the symbolic statement of the same four
identities, over free variables, is what `docs/kernel_alg.mm0` certifies. The JIT
path is the one the certificate fingerprint records, so `make` failing here means
the bytes the certificate talks about are no longer the bytes that run.

## Why the obligations live in a sidecar

The plan's W1 prefers obligations carried in CPS IR, so the compiler is the
single source of truth. That carrier is a compiler change and this tree's
`dune` toolchain is not buildable here (`nix develop` fails on a missing
dependency), so the reflected AST is written next to the kernel and audited
against it. `au2mm0` reads the same JSON shape an obligation node would
serialise to, so moving the carrier later does not change the prover — the
decision and its fallback are recorded in `PLAN_sota_kernel_verification.md`
§9.

## The certificate

```bash
python3 scripts/au2mm0 check flux1d.obligations.json   # decide the obligations
python3 scripts/au2mm0 emit  flux1d.obligations.json   # write + re-check mm0/
```

`emit` writes `mm0/flux1d_certificate.{mm0,mm1}`: the four obligations as
polynomial identities over `docs/kernel_alg.mm0`, each closed by a `peq` chain
(one ring law per step) and carried to the numeric domain by the theory's single
trusted `int_ring_transfer`. Coefficient arithmetic is grounded as visible `cf_`
decimal facts, counted in the artifact's `-- trust-split:` line and pinned in
`BUDGETS` in `scripts/au2mm0`.

The certificate is over ℤ. `Int64` wraps, so a certificate covers the scheme
only for inputs that do not overflow; that assumption is written into the
certificate header rather than left implicit, and `-- assumed:` / `-- witness:`
make the count and its non-vacuity auditable (W4). Two further lines keep the
claim honest: `-- code: cps=<sha256> …` records the compiled kernel the
certificate is about (recorded, not checked — no object bytes are exposed), and
`-- float-gap: … status=tested-not-proved` points at the generated probe
`mm0/flux1d_probe.c`, which `emit` renders from the same post-substitution terms
as the `peq` chain. The probe is a test; it is never called a proof.
