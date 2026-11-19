# `docs/data/gate` — recorded gate transcripts

The plan quotes per-shell gate totals (a shell can only run some checks, so the
same commit yields different `passed`/`gap` counts under `.#ocaml` and under
`.#lean`). A prose count cannot be re-read once the tree moves, so each count in
the plan points at a transcript here: the full stdout of one run, whose last
line is the summary the plan quotes.

| file | command | tree | summary |
|---|---|---|---|
| [`2026-11-19-ocaml-shell.log`](2026-11-19-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | `48658a42`, re-run after the W5 object re-pin | `164 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-19-lean-shell.log`](2026-11-19-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | `48658a42` | `173 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-19-obj-drift-control.log`](2026-11-19-obj-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/obj-drift-control` | after the W5 object re-pin | `obj-drift-control: held` |

The third row is not a gate count but a negative control's transcript: it is
here because it produced the `obj=` digest the certificate now pins, and because
it is what exposed a stale root `.so` that the gate had been measuring through.
Its command's own last line is the verdict.

The difference between the two shell counts is not a regression in either shell. The `ocaml` shell cannot run
the Lean-typed checks and the `lean` shell cannot run the OCaml compiler checks,
so each reports the other's checks as explained `[gap]` lines; the `lean` shell
carries the four §W3 approval gaps (isa, slot_layout, store_projection,
store_projection_toy) because an approval is an owner commit into
`mm0/approved/` and no gate can create one.

Reproducing a row means running its command at its tree and comparing the last
line. The transcripts are evidence, not inputs: no check reads them, so a stale
row is caught only by re-running the command, not by the gate — the same rule as
`docs/data/w3/probes.tsv` (the perturbation is the measurement; only the
citation is re-read).

The `ocaml` transcript was re-run twice on the day it was recorded — once after
the prose edits that cite it, once after the W5 object re-pin — and printed the
same summary (`164 passed, 6 explained gap(s), 0 failed`, exit 0) both times,
which is the expected relation: neither a plan-only edit nor a re-pinned digest
changes the check list.

The scripts are unedited stdout, so they contain absolute paths, temp-dir names
and Nix store hashes from the recording host. The two gate transcripts open with
Nix's `warning: Git tree '…' is dirty` because each measured the tree of the
commit it was recorded for, before that commit existed; the summary line, not
the warning, is what the plan quotes.
