# `docs/data/gate` — recorded gate transcripts

The plan quotes per-shell gate totals (a shell can only run some checks, so the
same commit yields different `passed`/`gap` counts under `.#ocaml` and under
`.#lean`). A prose count cannot be re-read once the tree moves, so each count in
the plan points at a transcript here: the full stdout of one run, whose last
line is the summary the plan quotes.

| file | command | tree | summary |
|---|---|---|---|
| [`2026-11-21-w3-approval-state-drift-control.log`](2026-11-21-w3-approval-state-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-approval-state-drift-control` | 2026-11-21, `85a368eb` plus the drift-control transcript check and the certificate re-pin `1f8ce68f`'s CPS order forced | `w3-approval-state-drift-control: holds` — gate `165 passed, 3 explained gap(s), 5 failed`, the five being the lines the deleted copy feeds |
| [`2026-11-21-w3-emitter-drift-control.log`](2026-11-21-w3-emitter-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-emitter-drift-control` | 2026-11-21, same tree as the row above | `w3-emitter-drift-control: holds` — gate `168 passed, 2 explained gap(s), 3 failed`, one drifted emitter cell and isa's approved copy with it |
| [`2026-11-21-w3-drift-controls-cert-approved.log`](2026-11-21-w3-drift-controls-cert-approved.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-drift-controls cert-approved` | 2026-11-21, same tree | `cert-approved: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the refinement's citations |
| [`2026-11-21-w3-drift-controls-liquid-golden.log`](2026-11-21-w3-drift-controls-liquid-golden.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-drift-controls liquid-golden` | 2026-11-21, same tree | `liquid-golden: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the pinned golden |
| [`2026-11-21-w93-certification-drift-control.log`](2026-11-21-w93-certification-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w93-certification-drift-control` | 2026-11-21, same tree | `w93-certification-drift-control: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the dropped approval |
| [`2026-11-21-w93-composition-drift-control.log`](2026-11-21-w93-composition-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w93-composition-drift-control` | 2026-11-21, same tree | `w93-composition-drift-control: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the drifted axiom count |
| [`2026-11-21-w93-closure-drift-control.log`](2026-11-21-w93-closure-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w93-closure-drift-control` | 2026-11-21, same tree | `w93-closure-drift-control: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the drifted closure digest |
| [`2026-11-21-ocaml-shell.log`](2026-11-21-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-21, same tree — the baseline the seven control transcripts diff against | `171 passed, 2 explained gap(s), 0 failed` — the two Lean-only checks |
| [`2026-11-20-lean-shell-final.log`](2026-11-20-lean-shell-final.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-20, `b205f73f` plus the uncommitted §W3 control fixes and the regenerated `CHANGELOG.md` (log opens with Nix's dirty-tree warning) | `179 passed, 0 explained gap(s), 0 failed` |
| [`2026-11-20-ocaml-shell-final.log`](2026-11-20-ocaml-shell-final.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-20, same tree as the row above | `170 passed, 2 explained gap(s), 0 failed` — the two Lean-only checks |
| [`2026-11-20-ocaml-shell-zero-w3.log`](2026-11-20-ocaml-shell-zero-w3.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-20, same tree as the row below | `170 passed, 2 explained gap(s), 0 failed` — the two Lean-only checks |
| [`2026-11-20-lean-shell-zero-gaps.log`](2026-11-20-lean-shell-zero-gaps.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-20, the tree that lands the four §W3 approvals | `179 passed, 0 explained gap(s), 0 failed` |
| [`2026-11-20-lean-shell-all-approved-proto.log`](2026-11-20-lean-shell-all-approved-proto.log) | `AUSTRALVM_PLAN=… AUSTRALVM_APPROVED_ROOT=… nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` — the four staged §W3 approvals in a scratch root, the plan's four state cells flipped there | 2026-11-20, `8249b592` plus the local edit to `scripts/lean2mm0-stmts` | `174 passed, 3 explained gap(s), 2 failed` — what approving all four would do today |
| [`2026-11-20-lean-shell-postcleanup.log`](2026-11-20-lean-shell-postcleanup.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-20, `37fd65ec`, after reclaiming stale build caches (38 GB free when it started) | `175 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-20-ocaml-shell-postcleanup.log`](2026-11-20-ocaml-shell-postcleanup.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-20, `8249b592`, the tree the first two rows run on | `166 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-20-ocaml-shell.log`](2026-11-20-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-20, `8b8eb323` after `make bridge` | `166 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-20-lean-shell.log`](2026-11-20-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-20, same tree as the row above | `175 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-19-ocaml-shell.log`](2026-11-19-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-19, after the W5 object re-pin and the bridge-copy check | `165 passed, 6 explained gap(s), 0 failed` |
| [`2026-11-19-lean-shell.log`](2026-11-19-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-19, same tree as the row above | `174 passed, 4 explained gap(s), 0 failed` |
| [`2026-11-19-obj-drift-control.log`](2026-11-19-obj-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/obj-drift-control` | 2026-11-19, after the W5 object re-pin | `obj-drift-control: held` |

The `2026-11-20-lean-shell-postcleanup.log` row re-runs the gate on the tree the
2026-11-20 pair describes, after the volume went from 98% full to 58% by
deleting build caches only (the bridge release artifacts and the root `.so` copy
were kept and re-verified identical, and `make check-bridge-fresh` still
passes). It is kept as evidence that the counts do not depend on the caches:
same tree, same `175`/`4`/`0`, and its `…-ocaml-shell-postcleanup.log`
counterpart repeats it under `.#ocaml` (`166`/`6`/`0`, the two extra gaps being
the Lean-only W9.1 and witness checks). The lean run was slower than the row
below it because the host was under memory pressure — one
`lean2mm0-stmts emit` step held 8.7 GB RSS — which was the gate parsing a whole
export eagerly for two roots. That is fixed: `scripts/lean2mm0-stmts` now reads
the export lazily, 8.99 GiB → 0.46 GiB peak on the 783 MB RV64 memory export,
with byte-identical output, so the slow run is history rather than a property
to plan around.

The `2026-11-20-ocaml-shell-zero-w3.log` row is the same tree under the other
shell, and it is the reason the two counts differ by more than the four approval
lines did: with the §W3 gaps closed, only the two checks `.#ocaml` cannot run
(the RV64 re-conversion and the Lean witness proofs) are left, so `170`/`2`/`0`
is the whole gap list rather than a subset of it.

The `2026-11-20-lean-shell-zero-gaps.log` row is the state the four §W3
approvals were landed for: with the statements committed, `mm0/clif.mm0` and
`mm0/store_projection.mm0` citing them, and the two checks that could only be
read before now measuring both directions, the gate reports no gap at all. The
`…-all-approved-proto.log` row below it is the probe that said the approval
commit alone would not get there, kept because it is the reason the certificate
and the controls changed in the same change.

The `2026-11-20-lean-shell-all-approved-proto.log` row is a probe rather than a
tree state: it stages the four §W3
approvals in a scratch root and flips the plan's state cells there — exactly
what an approval commit would change — to measure what the gate says about it.
It says no, twice. `W9.3: certification state matches mm0/approved/` fails
because `mm0/clif.mm0` still carries the `[gap]` certificate block, and that
block is emitted by `scripts/clif2mm0`, so citing the approval is a generator
edit rather than a comment edit. `W3: a §W3 row that disagrees with
mm0/approved/ is refused` fails because its stale-state probe is made by
flipping a *pending* row, and an all-approved table leaves it nothing to flip.
The three gaps are the same shape: `stage-approvals` stages nothing once every
artifact is approved, so the controls that copy a staging have none. Closing
those is §W3's described work — attach the proofs, cite the approval, build the
probe differently — not a copy of four files.

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
