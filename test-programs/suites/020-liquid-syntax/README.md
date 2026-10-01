# 020 — Liquid syntax

Surface syntax for Liquid Austral contracts (`PLAN_liquid_types.md` L2,
normative in `docs/LIQUID.md` §2.2–§2.3).

Contracts ride the `pragma` machinery as `Liquid_*(Contract => "<trl>")`.
L2 adds the TRL parser and the `liquid` compiler pass, so these tests pin the
*syntax*: which pragma shapes are accepted, and which contract strings are
refused with a span into the pragma string. Nothing is *enforced* yet — a
contract that parses is not yet discharged (L4/L5/L6).
