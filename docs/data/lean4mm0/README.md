# Lean 4 → MM0 typing census and probe measurements

The measurements behind §13.16–§13.27 of [`../../lean4mm0.md`](../../lean4mm0.md).
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
| `enc-probe.py`, `enc-probe.txt` | §13.25's encoder-only baseline: same loader and `Enc` on `adjoint_aeval` with no derivation, so what it costs to *hold* the statement is separated from what the derivation adds. Transcript is its output on `ChapterH8`. |
| `emit-snapshot-aeval-tm4.txt`, `emit-snapshot-aeval-tm6.txt` | §13.25's two `tracemalloc` top-12 snapshots of the emitter, with the `t` they were taken at. Line numbers resolve against the `scripts/lean-typing-proof` revision of the same date; `tm4`'s numbers shifted when the sampler grew columns, so the doc quotes only `tm6`'s. |
| `emit-trace-aeval-tm6.tsv` | §13.25's growth trace of `adjoint_aeval`: one row per ~5 s sample, columns as `scripts/lean-typing-proof` writes them (`rss_mb`, `traced_mb`, `uord_mb`, `free_mb`, `mmap_mb`, `obnodes`, the accumulator byte counts, and the derivation counters). It ends at t=5 550 s because it was stopped by hand once the t=900 s snapshot was in hand — with `tracemalloc` in the process it runs an order of magnitude slower per obligation than the terminal run, so it is the composition measurement and its last row is **not** a budget outcome. The level curve is the terminal run's own trace (§13.21, every 10 s, no `tracemalloc`). |
| `emit-trace-aeval-terminal.tsv` | The full 10 GiB trace of `adjoint_aeval` (§13.21/§13.25): 795 samples at 10 s, ending at the cap. Columns are the pre-`tracemalloc` set (`t rss_mb peak_mb exprs memo_by memo_n rules axioms oblig lift msubst depth stack …`); no `uord`/`traced`/`obnodes`, which is why it measures level and `tm6` measures composition. |
| `emit-trace-aeval-full.tsv`, `emit-run-aeval-full.log` | §13.27's full-budget run of `adjoint_aeval` after both streams: 567 samples at 10 s to t=5 747.7 s, where the declaration **finished** (5 752.4 s, peak 1 662 MB, `.mm0` 1 297 596 934 B) instead of reaching the 10 GiB cap. The log is the emitter's own report — rules, assumptions, artifact size. |
| `emit-trace-aeval-stream.tsv` | §13.26's after-the-fix trace: the same command as `emit-trace-aeval-terminal.tsv` with `oblig_sink` streaming and the `to_tree` memo, capped at 4 GiB and stopped by its 2 400 s budget at 116 874 obligations. `obnodes` is 0 on every row by construction — the deriver retains no obligation trees, so the column that used to grow now measures that it does not. |

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

The emitter trace is the one artifact here that is not produced by the sweep: it
is `LTP_TRACE=<out.tsv> LTP_TRACE_SNAP=<s> scripts/lean-proof-scan …
--limit=1 --only=<file>`, so the scan applies its own `cap_mem` and the trace
records the budget the emitter died in.

`--retry <tsv>` re-measures only the rows a previous run left open and rewrites
the file in place, so the command above resumes rather than restarts. Children
need a disk-backed `TMPDIR` (§13.18).
