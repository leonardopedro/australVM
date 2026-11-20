# `docs/data/gate` — recorded gate transcripts

The plan quotes per-shell gate totals (a shell can only run some checks, so the
same commit yields different `passed`/`gap` counts under `.#ocaml` and under
`.#lean`). A prose count cannot be re-read once the tree moves, so each count in
the plan points at a transcript here: the full stdout of one run, whose last
line is the summary the plan quotes.

| file | command | tree | summary |
|---|---|---|---|
| [`2026-11-20-lean-shell-postcleanup.log`](2026-11-20-lean-shell-postcleanup.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-20, `37fd65ec`, after reclaiming stale build caches (38 GB free when it started) | `175 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-20-ocaml-shell.log`](2026-11-20-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-20, `8b8eb323` after `make bridge` | `166 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-20-lean-shell.log`](2026-11-20-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-20, same tree as the row above | `175 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-19-ocaml-shell.log`](2026-11-19-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-19, after the W5 object re-pin and the bridge-copy check | `165 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-19-lean-shell.log`](2026-11-19-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-19, same tree as the row above | `174 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-19-obj-drift-control.log`](2026-11-19-obj-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/obj-drift-control` | 2026-11-19, after the W5 object re-pin | `obj-drift-control: held` |

The first row re-runs the gate on the tree the 2026-11-20 pair describes, after
the volume went from 98% full to 58% by deleting build caches only (the bridge
release artifacts and the root `.so` copy were kept and re-verified identical,
and `make check-bridge-fresh` still passes). It is kept as evidence that the
counts do not depend on the caches: same tree, same `175`/`4`/`0`. The run was
slower than the row below because the host was under memory pressure — one
`lean2mm0-stmts emit` step held 8.7 GB RSS — which is a property of the host,
not of the gate.

The `2026-11-19-obj-drift-control.log` row is not a gate count but a negative control's transcript: it is
here because it produced the `obj=` digest the certificate now pins, and because
it is what exposed a stale root `.so` that the gate had been measuring through.
Its command's own last line is the verdict.

The rows are ordered newest first: the 2026-11-19 pair describes `48658a42` and
the re-pins that followed it (`165`/`174`), and is kept because the plan's §3.5
block quotes that series; the 2026-11-20 pair is the tree after `8b8eb323`'s
merge added a check (`166`/`175`). A superseded row is history, not a claim about
`HEAD`.

The difference between the two shell counts is not a regression in either shell. The `ocaml` shell cannot run
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
