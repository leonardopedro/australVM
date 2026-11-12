# Lean 4 → MM0 typing census and probe measurements

The measurements behind §13.16–§13.22 of [`../../lean4mm0.md`](../../lean4mm0.md).
Produced by `scripts/lean-typing-sweep` over `mm0/exports` (329 exports of the
BookProof corpus), one sweep at a time.

| file | what it is |
| --- | --- |
| `census-own.tsv` | The 493-row census: one row per own declaration, merged from the sweep with the 4-row 12 GiB pass and all three probes spliced in. This is the authoritative table behind every count quoted in the doc. |
| `probe-mm0rs.tsv` | The 20 `mm0-rs` rows re-run at `--mm0rs-mem-gb 12` (§13.21). |
| `probe-emit.tsv` | The 3 `emit: over 900 s` rows re-run at `--emit-timeout 5400` (§13.21). |
| `probe-aeval.tsv` | The one row still open after that (`adjoint_aeval`) re-run at `--emit-timeout 21600` — over 10 GiB at 8 889 s, so the 6-hour clock never fired (§13.21). |
| `probe-mm0rs.log`, `probe-emit.log`, `probe-aeval.log` | The sweeps' own output, including the budget line each run measured under. |
| `arena_proto.rs` | The prototype behind §13.23: builds math-expression trees and a hash-consed arena over the same formulas, and counts the heap each holds with a counting allocator. A `src/bin` of an mm0-rs checkout (upstream `digama0/mm0`), not of this repo. |
| `arena-probe.txt` | Its transcripts on the two largest surviving artifacts, with the checkout's commit. |

## Columns

`chapter`, `decl`, `own`, `kind`, `status`, `rules`, `assumptions`,
`selfcheck`, `shared`, `nodes`, `detail`.

`status` is the stage that stopped the row (`closed`, `emit`, `mm0-rs`,
`mm0-c`, `timeout`); `detail` is the measured bound and the exact budget text
the row was run under, in the form `[emit_timeout=… emit_mem_gb=…
mm0_rs_timeout=… mm0_rs_mem_gb=…]`. A budget is part of a measurement, so
`detail` is kept with the row rather than in the doc: re-running with different
flags changes the class a row is in.

`rules` is what the emitter produced; `shared`/`nodes` count the applications in
the proof term the checker replays. `selfcheck` is `re-derived` when the checker
accepted every one of them, `over cap` when the artifact was larger than the
self-check budget, `timeout` when it outlived its clock.

## Reproducing

```bash
nix develop -c scripts/lean-typing-sweep mm0/exports <out.tsv> \
  --emit-timeout 1800 --emit-mem-gb 10 \
  --mm0rs-timeout 3600 --mm0rs-mem-gb 12 \
  --check-max-mb 2000 --check-timeout 3600 --scan-timeout 10800
```

`--retry <tsv>` re-measures only the rows a previous run left open and rewrites
the file in place, so the command above resumes rather than restarts. Children
need a disk-backed `TMPDIR` (§13.18).
