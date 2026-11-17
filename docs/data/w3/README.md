# `docs/data/w3` — the §W3 probe evidence

Two §W3 checks are only worth what their failures are worth, and both were
measured by perturbing the tree and reading the message. Those measurements are
recorded here as rows rather than prose, so `scripts/verify-invariants` can
re-read them: a row whose `gate_line` no longer names a check the script emits,
or whose `artifact` is no longer a §W3 row, fails
`W3: the recorded probe evidence still names live gate lines`.

| file | what it records |
|---|---|
| [`probes.tsv`](probes.tsv) | One row per measured perturbation: the artifact, what was done to it, the check that fired, the message it produced, and the date. The perturbations are *not* re-run by the gate — appending to `mm0/isa.mm1` or editing `EMITTERS` is a measurement, not a routine check — but a citation that has gone stale is caught on every run. |

`gate_exit` and `gate_total` record how a gate run against the perturbed tree
ended, and are `-` for probes that are exercised in-gate instead (the retired
emitter has no repository edit to run a gate against). The gate re-reads both for
shape: a recorded numeric exit must be non-zero — a perturbation that exits 0
caught nothing — and the total must read `<n> passed, <n> explained gap(s), <n>
failed`, so a row cannot claim a success for an experiment that failed to fire.

`expected_message` is written as the literal skeleton of the message, with `…`
wherever the gate interpolates something (an artifact stem, a command, a count) or
assembles the sentence from pieces: every literal chunk between the ellipses,
four characters or longer, must still appear verbatim in
`scripts/verify-invariants`. That is what makes the message checkable at all —
variable parts differ per artifact — and it is what catches a reworded check
before the recorded measurement starts describing output the gate can no longer
produce.

Reproducing a row means performing its perturbation (`probe` names the shape)
and re-running `scripts/verify-invariants`, after which the message in
`expected_message` should appear. The in-gate controls
(`W3: an approved §W3 row whose emitter left the registry is refused`, and the
stale-`.mm1` reason kept by the re-derivation check) exercise the same code paths
without touching the repository.
