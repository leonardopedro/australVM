# `docs/data/gate` — recorded gate transcripts

The plan quotes per-shell gate totals (a shell can only run some checks, so the
same commit yields different `passed`/`gap` counts under `.#ocaml` and under
`.#lean`). A prose count cannot be re-read once the tree moves, so each count in
the plan points at a transcript here: the full stdout of one run, whose last
line is the summary the plan quotes.

| file | command | tree | summary |
|---|---|---|---|
| [`2026-11-19-ocaml-shell.log`](2026-11-19-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-19, after the W5 object re-pin and the bridge-copy check | `165 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-19-lean-shell.log`](2026-11-19-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-19, same tree as the row above | `174 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-19-obj-drift-control.log`](2026-11-19-obj-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/obj-drift-control` | 2026-11-19, after the W5 object re-pin | `obj-drift-control: held` |
| [`2026-11-20-ocaml-shell.log`](2026-11-20-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-20, after the certificate re-pin on x86_64 | `159 passed, 8 explained gap(s), 0 failed` |
| [`2026-11-20-default-shell.log`](2026-11-20-default-shell.log) | `nix develop "git+file://$PWD" -c bash scripts/verify-invariants` | 2026-11-20, same tree as the row above | `174 passed, 5 explained gap(s), 0 failed` |

The third row is not a gate count but a negative control's transcript: it is
here because it produced the `obj=` digest the certificate now pins, and because
it is what exposed a stale root `.so` that the gate had been measuring through.
Its command's own last line is the verdict.

The 2026-11-20 `ocaml` row exists because the 2026-11-19 one does not reproduce
on this tree: it records `165 passed, 6 explained gap(s)` with the Liquid checks
as `[pass]`, while the same command on this tree prints `159 passed, 8 explained
gap(s)` and reports the Liquid golden and subset checks as a `[gap]`
(`why3 not on PATH`). That shell carries no prover by design (`flake.nix`), so
its Liquid lines can only be a `[pass]` where why3 was on PATH for some other
reason. The `159` figure is the one the plan quotes.

The `default` shell is where the Liquid checks are not a gap, and on an x86_64
host it builds: it carries why3 1.8.2, z3 4.16.0 and alt-ergo 2.6.3, the gate
runs all fifteen Liquid checks as `[pass]`, and `scripts/w3-drift-controls
liquid-golden` holds there in 258 s (its drifted copy reports `173 passed, 5
explained gap(s), 1 failed`, which is the drift the case asserts).

The difference between the shell counts is not a regression in either shell. The `ocaml` shell cannot run
the Lean-typed checks and the `lean` shell cannot run the OCaml compiler checks,
so each reports the other's checks as explained `[gap]` lines; the `lean` shell
carries the four §W3 approval gaps (isa, slot_layout, store_projection,
store_projection_toy) because an approval is an owner commit into
`mm0/approved/` and no gate can create one.

The `tree` column names a date rather than a commit for these rows because each
one was recorded for the commit that adds or updates it, and a commit cannot
contain its own hash; the summaries are what the plan cites, and each row's
command reproduces its summary on this tree.

Reproducing a row means running its command at its tree and comparing the last
line. The transcripts are evidence, not inputs: no check reads them, so a stale
row is caught only by re-running the command, not by the gate — the same rule as
`docs/data/w3/probes.tsv` (the perturbation is the measurement; only the
citation is re-read).

Each transcript is the latest run of its command, not the first: the `ocaml` one
was re-run after the prose edits that cite it and again after the W5 object
re-pin, printing the same `164 passed, 6 explained gap(s), 0 failed` (exit 0)
both times, then once more after the bridge-copy check was added (`165 passed`,
and `174 passed` under `.#lean`). That is the expected relation: a plan-only
edit does not change the check list, and a re-pinned digest does not either — a
new check does, and its line is in both logs.

The scripts are unedited stdout, so they contain absolute paths, temp-dir names
and Nix store hashes from the recording host. The two gate transcripts open with
Nix's `warning: Git tree '…' is dirty` because each measured the tree of the
commit it was recorded for, before that commit existed; the summary line, not
the warning, is what the plan quotes.
