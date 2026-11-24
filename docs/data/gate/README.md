# `docs/data/gate` — recorded gate transcripts

The plan quotes per-shell gate totals (a shell can only run some checks, so the
same commit yields different `passed`/`gap` counts under `.#ocaml` and under
`.#lean`). A prose count cannot be re-read once the tree moves, so each count in
the plan points at a transcript here: the full stdout of one run, whose last
line is the summary the plan quotes.

| file | command | tree | summary |
|---|---|---|---|
| [`2026-11-23-default-shell.log`](2026-11-23-default-shell.log) | `nix develop "git+file://$PWD" -c bash scripts/verify-invariants` | 2026-11-23, `0b2d0c3d`, on a clean tree against `unfer` at its `main` (`67feb84`) | `192 passed, 1 explained gap(s), 0 failed` — the same count as 2026-11-22; the only line that moved is D00, which now reads 24 contract rows (the D10 rows upstream added) and ends `measured against unfer 67feb84` |
| [`2026-11-23-ocaml-shell.log`](2026-11-23-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-23, `0b2d0c3d`, on a clean tree, same sibling | `177 passed, 4 explained gap(s), 0 failed` — `192 − 15` Liquid passes become three `why3` gaps; only the D00 line differs from the 2026-11-22 run |
| [`2026-11-23-lean-shell-rust.log`](2026-11-23-lean-shell-rust.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-23, `0b2d0c3d`, on a clean tree, same sibling | `178 passed, 3 explained gap(s), 0 failed` — `W9.1` passes here (`177 + 1`, `4 − 1`); only the D00 line differs |
| [`2026-11-23-nix-shell-python3.log`](2026-11-23-nix-shell-python3.log) | `nix-shell -p python3 --run "bash scripts/verify-invariants"` | 2026-11-23, `0b2d0c3d`, on a clean tree, same sibling | `177 passed, 4 explained gap(s), 0 failed` — no `why3` and no RV64 re-conversion; only the D00 line differs |
| [`2026-11-22-default-shell.log`](2026-11-22-default-shell.log) | `nix develop "git+file://$PWD" -c bash scripts/verify-invariants` | 2026-11-22, `892b83bc` (the merges of `79b29839`, the Lean-typing work and the capability-handle runtime, plus `mm0` in the shell, the quoted PC05 gap, the lock update the sibling's `nanoda_lib` move needs, and the nixpkgs bump with Lean pinned back to 4.30), on a clean tree | `192 passed, 1 explained gap(s), 0 failed` — only `W9.1`, which needs the lean shell; `PC05a`/`PC05b` replay here because this shell carries `packages.mm0` and `safestos/gpu`'s debug `avm-gpu` is built |
| [`2026-11-22-ocaml-shell.log`](2026-11-22-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-22, `892b83bc` (the merges of `79b29839`, the Lean-typing work and the capability-handle runtime, plus `mm0` in the shell, the quoted PC05 gap, the lock update the sibling's `nanoda_lib` move needs, and the nixpkgs bump with Lean pinned back to 4.30), on a clean tree | `177 passed, 4 explained gap(s), 0 failed` — `W9.1` and the three `why3` checks (`177 = 192 − 15`, `4 = 1 + 3`) |
| [`2026-11-22-lean-shell-rust.log`](2026-11-22-lean-shell-rust.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-22, `892b83bc` (the merges of `79b29839`, the Lean-typing work and the capability-handle runtime, plus `mm0` in the shell, the quoted PC05 gap, the lock update the sibling's `nanoda_lib` move needs, and the nixpkgs bump with Lean pinned back to 4.30), on a clean tree — the shell with `pkgs.rustc`, `pkgs.cargo` and `pkgs.stdenv.cc` | `178 passed, 3 explained gap(s), 0 failed` — `W9.1` passes (`178 = 177 + 1`) and the three `why3` checks remain; the five `toy-witness` comparisons and `[pass] logos mm0` run here |
| [`2026-11-22-lean-shell.log`](2026-11-22-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-22, `c7237736`, on a clean tree — superseded by the row above after `ff53c29b` gave the shell a linker | `166 passed, 9 explained gap(s), 0 failed` — `W9.1` passes here; `why3` is absent (3), the witness cannot be built (5), and the logos emitter cannot link, whose gap quotes the sub-gate's own line |
| [`2026-11-22-nix-shell-python3.log`](2026-11-22-nix-shell-python3.log) | `nix-shell -p python3 --run "bash scripts/verify-invariants"` | 2026-11-22, `892b83bc` (the merges of `79b29839`, the Lean-typing work and the capability-handle runtime, plus `mm0` in the shell, the quoted PC05 gap, the lock update the sibling's `nanoda_lib` move needs, and the nixpkgs bump with Lean pinned back to 4.30), on a clean tree | `177 passed, 4 explained gap(s), 0 failed` — no `why3` (3 Liquid checks) and no RV64 re-conversion (`W9.1`); this host's profile now carries `lean` (4.28.0) and a working `rustc`/`cc`, so the five `toy-witness` comparisons and the `logos` build that used to gap here (`167`/`10`/`0`) pass, leaving the bare and ocaml columns identical in count |
| [`2026-11-21-w3-approval-state-drift-control.log`](2026-11-21-w3-approval-state-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-approval-state-drift-control` | 2026-11-21, `85a368eb` plus the drift-control transcript check and the certificate re-pin `1f8ce68f`'s CPS order forced | `w3-approval-state-drift-control: holds` — gate `165 passed, 3 explained gap(s), 5 failed`, the five being the lines the deleted copy feeds |
| [`2026-11-21-w3-emitter-drift-control.log`](2026-11-21-w3-emitter-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-emitter-drift-control` | 2026-11-21, same tree as the row above | `w3-emitter-drift-control: holds` — gate `168 passed, 2 explained gap(s), 3 failed`, one drifted emitter cell and isa's approved copy with it |
| [`2026-11-21-w3-drift-controls-cert-approved.log`](2026-11-21-w3-drift-controls-cert-approved.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-drift-controls cert-approved` | 2026-11-21, same tree | `cert-approved: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the refinement's citations |
| [`2026-11-21-w3-drift-controls-liquid-golden.log`](2026-11-21-w3-drift-controls-liquid-golden.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w3-drift-controls liquid-golden` | 2026-11-21, same tree | `liquid-golden: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the pinned golden |
| [`2026-11-21-w93-certification-drift-control.log`](2026-11-21-w93-certification-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w93-certification-drift-control` | 2026-11-21, same tree | `w93-certification-drift-control: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the dropped approval |
| [`2026-11-21-w93-composition-drift-control.log`](2026-11-21-w93-composition-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w93-composition-drift-control` | 2026-11-21, same tree | `w93-composition-drift-control: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the drifted axiom count |
| [`2026-11-21-w93-closure-drift-control.log`](2026-11-21-w93-closure-drift-control.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/w93-closure-drift-control` | 2026-11-21, same tree | `w93-closure-drift-control: holds` — gate `170 passed, 2 explained gap(s), 1 failed`, the drifted closure digest |
| [`2026-11-21-ocaml-shell.log`](2026-11-21-ocaml-shell.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-21, the merge of `b3c78130` with `ab6d1074`, this x86_64 host — re-recorded after the pull, so it covers the D00 and W5 checks too | `171 passed, 4 explained gap(s), 0 failed` — `W9.1` plus the three per-file Liquid checks `why3` would run |
| [`2026-11-21-lean-shell.log`](2026-11-21-lean-shell.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-21, same tree as the row above | `166 passed, 9 explained gap(s), 0 failed` — `W9.1` passes and `why3` is still absent, but this shell cannot link: its `rustc` is the host's rustup shim whose `cc` names a removed store path, so the five `scripts/toy-witness` comparisons and the `logos` emitter build gap instead of failing (`171 + 1 − 5 − 1 = 166`, `4 − 1 + 5 + 1 = 9`) |
| [`2026-11-21-default-shell.log`](2026-11-21-default-shell.log) | `nix develop "git+file://$PWD" -c bash scripts/verify-invariants` | 2026-11-21, same tree, where `devShells.default` builds | `186 passed, 1 explained gap(s), 0 failed` — `why3` present, so the three Liquid checks are 15 passes instead of 3 gaps (`171 + 15 = 186`, `4 − 3 = 1`) |
| [`2026-11-21-ocaml-shell-aarch64-d00-gated.log`](2026-11-21-ocaml-shell-aarch64-d00-gated.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-21, `3ad01bac` plus the W5 binary note and the D00 record | `177 passed, 3 explained gap(s), 0 failed` — the same three gaps as the run below, plus the two D00 checks (record well-formed, checker refuses every mutated record) |
| [`2026-11-21-ocaml-shell-aarch64.log`](2026-11-21-ocaml-shell-aarch64.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-21, `3ad01bac` plus the binary-staleness note and its control, an aarch64 host after `make all` | `175 passed, 3 explained gap(s), 0 failed` — the two Lean-only checks, the object digest pinned on x86_64, and the new `W5: the binary note …` control |
| [`2026-11-20-lean-shell-final.log`](2026-11-20-lean-shell-final.log) | `nix develop "git+file://$PWD#lean" -c bash scripts/verify-invariants` | 2026-11-20, `b205f73f` plus the uncommitted §W3 control fixes and the regenerated `CHANGELOG.md` (log opens with Nix's dirty-tree warning) | `179 passed, 0 explained gap(s), 0 failed` |
| [`2026-11-20-ocaml-shell-final.log`](2026-11-20-ocaml-shell-final.log) | `nix develop "git+file://$PWD#ocaml" -c bash scripts/verify-invariants` | 2026-11-20, same tree as the row above | `170 passed, 2 explained gap(s), 0 failed` — the two Lean-only checks |
| [`2026-11-20-default-shell.log`](2026-11-20-default-shell.log) | `nix develop "git+file://$PWD" -c bash scripts/verify-invariants` | 2026-11-20, an x86_64 host where `devShells.default` builds | `174 passed, 5 explained gap(s), 0 failed` |
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

The 2026-11-22 set re-runs the gate once per shell on `892b83bc` — the merges of
`origin/master` `79b29839`, the Lean-typing work and the capability-handle
runtime after it, which added the
D09 record, the `PC05b` transport and the `PC05a` replay, plus `packages.mm0` in
the default and ocaml shells, the gap line that quotes `scripts/pc05-reflect`'s
own reason, the workspace locks the sibling's `nanoda_lib` move needs, and the
nixpkgs bump whose Lean stays pinned at 4.30 through `nixpkgs-lean` (the
channel's 4.35 refuses the Sail model). Its D00 line reads 20 contract rows
rather than 14 (upstream's D09 and distributed rows landed in between) and ends
`measured against unfer ad3ed80`, the sibling's committed tip; the earlier
recordings on `7bb7b3ca` ended `(dirty)` because that worktree was mid-merge.
Four checks landed after those recordings, all of them shell-independent: the
two `PC07` lines (the frozen chain replays, and the chain checker's controls
refuse and report separately) and the two `PC08` lines (the captured host
fragment decodes and is pinned, and the native edge's controls refuse), which is
why every column moved by four and no other line changed.
`devShells.default` reads `192`/`1`/`0`: `.#ocaml`'s 15 missing Liquid passes
become its three `why3` gaps, so `177 = 192 − 15` and `4 = 1 + 3`. `.#lean`
converts `W9.1` (`178 = 177 + 1`, `3 = 4 − 1`). The bare `nix-shell -p python3`
invocation — the one `AGENTS.md` documents — now reads `177`/`4`/`0` as well:
this host's profile carries `lean` 4.28.0 and a `rustc`/`cc` that links, so the
five `toy-witness` comparisons and the `logos` build no longer gap there, and
only `why3` and `W9.1` remain missing. Both PC05 replays pass in every shell now: the
earlier recordings on `395c74c3` gapped `PC05a` in all four, and three of those
`[gap]` lines blamed `mm0-rs`/`mm0-c` when the missing piece was the
`safestos/gpu` debug `avm-gpu` the script also needs — which is why the gate now
passes the script's own reason through instead of naming the toolchain. All are
clean-tree runs of one tree, so unlike the 2026-11-21 rows they carry no
`warning: Git tree '…' is dirty` line. Each of the four was first recorded on
the commit that changed its shell (`d8111a94`, `6a3e385f`, `ff53c29b`), refreshed
at `5d484b18` (the merge, which moved only the D00 row count), again at
`c24ea9a3` and `d498b930`, at `395c74c3` where the certificate and citation
re-pin moved the digests, at `2ea08fcc` where the changed lines were the two PC05
checks and their counts, at `b8d628de`, where the counts were unchanged and
the toolchain moved (OCaml 5.5.2, Lean 4.30 from the pinned input), and finally
at `7bb7b3ca`, where the D00 row count moved to 20, and finally at `892b83bc`,
where the four PC07/PC08 lines landed and D00 measures a clean sibling.

The `lean` shell appears twice. The older row (`c7237736`) is what it reported
while its `rustc`/`cargo` were the host's rustup shims, whose `cc` names a
garbage-collected `ld-wrapper.sh`: the five `scripts/toy-witness` comparisons
and the `logos` emitter could not be built, so the gate reported them as gaps
and quoted the linker's own line (`166`/`9`/`0`). `ff53c29b` adds `pkgs.rustc`,
`pkgs.cargo` and `pkgs.stdenv.cc` to `devShells.lean`, and the re-recorded row
has both checks passing with only `why3` left (`172`/`3`/`0`,
`171 + 1 = 172` and `4 − 1 = 3`). That is a shell fix, not a relaxation: the
sub-gate itself reports `logos2mm0: 15 passed, 0 failed, 0 gapped` there, and the
bare shell — which still has no C toolchain — keeps the logos gap quoting
`logos2mm0: 0 passed, 0 failed, 1 gapped`.

The footer is authoritative, not a `grep`: the bare log has 177 `[pass]` lines
because subprocesses print their own, and the logos block emits two `[gap]`
lines for one gap, so the quoted figures are the footer's `165`/`10`. The bare
shell's arithmetic closes from the 2026-11-21 `ocaml` row (`171`/`4`/`0`):
`W9.1` is already one of those gaps, so the bare shell cannot build the witness
(five more gaps) or the logos emitter (one more), giving
`171 − 5 − 1 = 165` and `4 + 5 + 1 = 10`.

The 2026-11-21 trio re-records the tree where `b3c78130` and the pulled
`ab6d1074` meet, whose five §W3 rows are all approved. Two earlier recordings of
the same date are superseded by it, and both numbers are kept here because the
difference is a fact about the tree rather than a correction:
`174` / `183` / `176` was taken at `3ad01bac`, before the `D00` contracts, the
`D00` checker and the `W5: the binary note …` control were counted by the gate,
so it is three passes short of this trio in every shell; `168` / `164` / `183`
was this host's reading of the pre-merge tree, which is the same trio minus
those three checks. Within the trio the arithmetic closes: `171 + 15 = 186` once
`why3` is on PATH (the three Liquid checks become 15 passes there, so
`4 − 3 = 1` gap), and the `lean` shell turns `W9.1` into a pass while adding six
gaps — the five `scripts/toy-witness` comparisons, which cannot be built there,
and the `logos` emitter, whose `cargo build` cannot link in that shell
(`171 + 1 − 5 − 1 = 166`, `4 − 1 + 5 + 1 = 9`).

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

The `2026-11-20-default-shell.log` row is the one shell where the Liquid checks
are not a gap: it carries why3 1.8.2, z3 4.16.0 and alt-ergo 2.6.3, so
`liquid-golden` holds there (258 s) and the five remaining gaps are the Lean-only
transcripts. Under `.#ocaml` the same tree reports the Liquid golden and subset
checks as a `[gap]` (`why3 not on PATH`), which is why that shell's count is
lower; neither figure is a regression in the other shell.

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

## A `cancelled` CI job is not a failing one

These transcripts are produced locally, but the same counts are what CI's
`verify-invariants` job re-derives, so the two can be read side by side — with
one trap. GitHub cancels a run's still-running jobs when a new push supersedes
it, and `nix-pkg` is by far the longest job in `Build and Test` (it builds the
bridge, then runs the OCaml suite under `make test`). Measured 2026-11-23: on
three consecutive commits it read `cancelled` in the `OCaml suite (build,
typecheck, tests)` step, each time within a minute of a new upstream commit,
while `avm-pkg`, `verify-invariants` and `build-and-test-linux` in the same run
had already reported `success`. It then completed `success` on the first commit
that stayed the branch tip for an hour.

There is a second way to get the same word, and it is a real finding: a job cut
by its own `timeout-minutes` also reports `cancelled`, with no annotation.
Measured 2026-11-24 on `6c8591b3`: `nix-pkg` started 08:34:14Z and read
`cancelled` at 09:34:33Z — exactly 60:19 of job time, in the same step, with
`make bridge` having taken 24.9 minutes that run against 17.6 on the previous
green one. The next commit's run had started at 09:07, 27 minutes earlier, so
this was not a supersede either — `Build and Test` had no `concurrency:` group
and three runs on `master` were in flight at once, which is where the extra
seven minutes went. That is the bound being too tight, not the tree being wrong;
the fix was to raise it (90 minutes, with the measured split in the job's
comment) and to give the workflow its concurrency group, so the slow job stops
competing with a copy of itself. The two cases separate by duration: a
superseded job dies within seconds of a newer push, a timed-out one dies on its
bound to the minute.

So read the line in this order: a job that reads `cancelled` says nothing about
the check it was running — it was cut, not failed, and a job that reports
`failure` (or `timed_out`) is the only one that carries a verdict about the
tree. The one `cancelled` line that does carry a finding is the job that sat on
its own timeout. The authoritative signal for a commit is the run whose jobs all reached
`success`; the commit before a cancellation is not evidence either way, and
re-reading its annotation finds none, because a cancellation writes no
annotation at all.

The scripts are unedited stdout, so they contain absolute paths, temp-dir names
and Nix store hashes from the recording host. The two gate transcripts open with
Nix's `warning: Git tree '…' is dirty` because each measured the tree of the
commit it was recorded for, before that commit existed; the summary line, not
the warning, is what the plan quotes.
