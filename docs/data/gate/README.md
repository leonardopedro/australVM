# `docs/data/gate` — recorded gate transcripts

The plan quotes per-shell gate totals (a shell can only run some checks, so the
same commit yields different `passed`/`gap` counts under `.#ocaml` and under
`.#lean`). A prose count cannot be re-read once the tree moves, so each count in
the plan points at a transcript here: the full stdout of one run, whose last
line is the summary the plan quotes.

| file | command | tree | summary |
|---|---|---|---|
| [`2026-11-19-ocaml-shell.log`](2026-11-19-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | `48658a42` | `164 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-19-lean-shell.log`](2026-11-19-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | `48658a42` | `173 passed, 4 explained gap(s), 0 failed` |

The difference is not a regression in either shell. The `ocaml` shell cannot run
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

The `ocaml` transcript was re-run once after the prose edits that cite it and
printed the same summary (`164 passed, 6 explained gap(s), 0 failed`, exit 0),
which is the expected relation: a plan-only edit is not a check-list change, so
the recorded run still describes the tree.

Both files are unedited stdout, so they contain absolute paths, temp-dir names
and Nix store hashes from the recording host.
