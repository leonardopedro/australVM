#!/usr/bin/env python3
"""mm0_emit — the shared MM0 emission primitives (W0 of
PLAN_sota_kernel_verification.md).

`scripts/lean2mm0` proved the emission logic is reusable: declaration
rendering, numeral declarations, `prim_eval`-style grounded bridges, the
trust-split/coverage header lines, and the two spec/proof consistency checks.
`scripts/au2mm0` (W2) needs exactly these over a different theory, and
`docs/lean4mm0.md` §0 forbids a second copy of the logic.

Everything here is stdlib-only and theory-agnostic: the equation symbol, the
literal constructor and the evaluator name are parameters, not assumptions.
Refactoring into this module must not change a byte of generated MM0 — that is
what `scripts/lean2mm0 check` (`git diff --exit-code mm0/`) pins.
"""

from __future__ import annotations

import re

# The agent signature of docs/cic.mm0, in declaration order. Which of these a
# generated file actually uses is *reported in the artifact* (§9.4): a reader
# must be able to tell a checked rule from a reserved one, and prose goes stale
# the moment the encoder changes.
SIGNATURE_TERMS = [
    "tvar", "tapp", "tabs", "tint", "tf64", "ttrue", "tfalse", "tcon",
    "tnil", "tcons", "tfold", "tdup", "tera", "tprim", "tentity",
    "tsubst", "teval",
]


class EmitError(Exception):
    """Any loud failure: unknown construct, value disagreement, drift."""


# ── declarations ─────────────────────────────────────────────────────────


def axiom(name: str, math: str) -> str:
    return f"axiom {name}: $ {math} $;"


def theorem(name: str, math: str) -> str:
    return f"theorem {name}: $ {math} $;"


def pub_theorem(name: str, math: str, proof: str) -> str:
    """The proof-carrying half of a theorem pair (`pub ` + `= '<proof>`)."""
    wrapped = proof if proof.startswith("(") else f"({proof})"
    return f"pub theorem {name}: $ {math} $ = '{wrapped};"


def defn(name: str, sort: str, math: str) -> str:
    return f"def {name}: {sort} = $ {math} $;"


def grounded_axiom(name: str, lhs: str, rhs: str, eq: str = "teq") -> str:
    """One visible decimal fact, the auditing stance of docs/cic.mm0."""
    return f"axiom {name}: $ {eq} {lhs} {rhs} $;"


def prim_bridge(name: str, tag, a, b, rhs: str, *,
                eq: str = "teq", const: str = "tint",
                evaluator: str = "teval") -> str:
    """A `prim_eval` instance: ground the uninterpreted evaluator.

    `docs/cic.mm0` states `teq redex (tint (teval op n m))`; the emitter
    grounds `teval` at each byte it uses. `warnings are proof holes` applies:
    a wrong decimal is visible here.
    """
    return grounded_axiom(
        name, f"({const} ({evaluator} {tag} {a} {b}))", rhs, eq)


def numeral_decl(n) -> str:
    return f"term n{n}: nat; prefix n{n}: ${n}$ prec max;"


def numeral_lines(nums) -> list[str]:
    return [numeral_decl(n) for n in sorted(nums)]


# ── header lines (read by scripts/verify-invariants) ─────────────────────


def trust_split_line(stem: str, structural: int, grounded: int,
                     certificates: int) -> str:
    """The machine-readable trust split, read by scripts/verify-invariants."""
    return (f"-- trust-split: structural={structural} grounded={grounded} "
            f"certificates={certificates}")


def coverage_line(body: str, signature_terms=None) -> str:
    """Which signature terms the generated declarations mention.

    `body` must be the generated part only — the `.mm0` embeds the theory
    verbatim for audit, so scanning the whole file would report every
    declaration as used.
    """
    terms = SIGNATURE_TERMS if signature_terms is None else signature_terms
    used = [t for t in terms if re.search(rf"\b{re.escape(t)}\b", body)]
    unused = [t for t in terms if t not in used]
    return (f"-- theory coverage: used={','.join(used) or '-'} "
            f"unused={','.join(unused) or '-'}")


def pinned_split(stem: str, budgets: dict, actual: dict) -> dict:
    """Refuse a trust split that drifted from its pinned row.

    The split is a *contract*, not a statistic: movement toward `grounded`
    means mm0-c checks less of this certificate, which a proof-checking
    pipeline cannot see (the .mmb would still verify). Widening is allowed;
    claiming it is not is the bug.
    """
    budget = budgets.get(stem)
    if budget is None:
        raise EmitError(
            f"{stem}: no trust-split budget in BUDGETS — add one (the split is "
            "a pinned contract, not a statistic)")
    if actual != budget:
        raise EmitError(
            f"{stem}: trust split widened/drifted: {actual} != pinned {budget}. "
            "Movement toward `grounded` means mm0-c checks less of this "
            "certificate; if that is intended, edit BUDGETS in the same commit "
            "and say why in the artifact header.")
    return budget


# ── spec/proof consistency checks ────────────────────────────────────────


def check_spec_proof_parity(mm0_lines: list[str], mm1_lines: list[str]) -> None:
    """The two streams must differ ONLY in `pub ` and the proof tail."""
    stripped = [re.sub(r"^pub ", "", m) for m in mm1_lines]
    stripped = [re.sub(r" = '[\s\S]*;$", ";", s) for s in stripped]
    if len(mm0_lines) != len(stripped):
        raise EmitError("spec/proof line counts differ")
    for i, (a, b) in enumerate(zip(mm0_lines, stripped), 1):
        if a != b:
            raise EmitError(f"spec/proof divergence at line {i}:\n"
                            f"  mm0: {a}\n  mm1: {b}")


def check_proof_refs(mm1_lines: list[str],
                     builtin_refs=("teq_trans", "teq_app", "teq_refl",
                                   "prim_eval")) -> None:
    """Every proof reference must be declared somewhere in the file."""
    declared = set()
    for line in mm1_lines:
        m = re.match(
            r"(?:pub )?(?:theorem|axiom|def|term) ([A-Za-z_][A-Za-z0-9_]*)",
            line)
        if m:
            declared.add(m.group(1))
    ref_re = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
    for line in mm1_lines:
        if " = '" not in line:
            continue
        proof = line.split("= '", 1)[1]
        for t in ref_re.findall(proof):
            if t in builtin_refs:
                continue
            if t not in declared:
                raise EmitError(f"proof references undeclared {t!r}: "
                                f"{line[:120]}")


def render(lines: list[str]) -> str:
    return "\n".join(lines) + "\n"
