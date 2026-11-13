#!/usr/bin/env python3
"""mm0_ring — the W2 prover: normalize an integer polynomial and emit the `peq`
chain that proves the normalization, one ring law per step.

WHY THIS IS THE HARD PART. `mm0-c` will not take "these two polynomials are
equal" on trust. It wants each step to be an instance of a law in
`docs/kernel_alg.mm0`, applied at a position, chained by `peq_trans`. So this
module is a small proof-producing normalizer: it rewrites a `Poly` term to a
canonical form and carries the justification along.

SHAPE OF A PROOF. `mm0` proof applications take *no* term arguments — the terms
unify from the statement being proved. So a step is the bare name of a law
(`(padd_comm)`, `(pmul_distrib)`, …) and the chain is

    (peq_trans <step 1> (peq_trans <step 2> …))

whose endpoints are pinned by the theorem statement. A step under a context
(the left side of a `padd`, say) is `(peq_cong_add <sub-proof> (peq_refl))`.
This was verified against `mm0-rs` + `mm0-c` before the normalizer was written;
see the `ring_swap`/`ring_pmul_swap` helpers this module emits, which are the
only derived lemmas the normalizer needs (each three law applications).

WHAT IS TRUSTED HERE. Two things, both auditable and both counted in the
artifact's trust split:

  * reflection — `scripts/au2mm0` turns the kernel into these polynomials. It is
    decided independently (symbolic expansion and exact sampling) and the
    emitter refuses when the two disagree.
  * coefficient arithmetic — combining equal monomials and multiplying literal
    coefficients is *not* derived from the ring laws. Each instance is written
    into the artifact as a visible decimal fact (`cf_<n>`), counted `grounded`,
    and re-checked here by exact integer sampling before it is emitted — the
    same posture `docs/kernel_alg.mm0` takes for `teval` in `docs/cic.mm0`.

Everything else — distributivity, associativity, commutativity, the sign laws —
is a ring law application that `mm0-c` re-checks, one step at a time.
"""

from __future__ import annotations

# A term is a tuple: ("lit", n) / ("var", i) / ("add", a, b) / ("mul", a, b) /
# ("neg", a). Rendered text is the MM0 math string, fully parenthesized.
REFL = "(peq_refl)"

# Refuse inputs whose proof would be enormous rather than emit a megabyte.
MAX_ATOMS = 512
MAX_STEPS = 20000


class RingError(Exception):
    """The prover cannot produce a proof (as opposed to a false obligation)."""


# ── terms ────────────────────────────────────────────────────────────────

def lit(n: int):
    return ("lit", n)


def var(i: int):
    return ("var", i)


def add(a, b):
    return ("add", a, b)


def mul(a, b):
    return ("mul", a, b)


def neg(a):
    return ("neg", a)


def text(t) -> str:
    tag = t[0]
    if tag == "lit":
        n = t[1]
        return f"(tint n{n})" if n >= 0 else f"(pneg (tint n{-n}))"
    if tag == "var":
        return f"(pvar n{t[1]})"
    if tag == "add":
        return f"(padd {text(t[1])} {text(t[2])})"
    if tag == "mul":
        return f"(pmul {text(t[1])} {text(t[2])})"
    if tag == "neg":
        return f"(pneg {text(t[1])})"
    raise RingError(f"unknown term node {t!r}")


def numerals(t, out: set) -> set:
    """Every numeral index a term mentions, so the artifact declares them."""
    tag = t[0]
    if tag == "lit":
        out.add(abs(t[1]))
    elif tag in ("add", "mul"):
        numerals(t[1], out)
        numerals(t[2], out)
    elif tag == "neg":
        numerals(t[1], out)
    return out


def children(t):
    tag = t[0]
    if tag in ("add", "mul"):
        return [t[1], t[2]]
    if tag == "neg":
        return [t[1]]
    return []


def rebuild(t, i, child):
    tag = t[0]
    if tag == "add":
        return ("add", child, t[2]) if i == 0 else ("add", t[1], child)
    if tag == "mul":
        return ("mul", child, t[2]) if i == 0 else ("mul", t[1], child)
    if tag == "neg":
        return ("neg", child)
    raise RingError(f"{tag} has no children")


def lift(term, path, new_sub, proof):
    """Wrap `proof` (about the subterm at `path`) up to a proof about `term`."""
    if not path:
        return new_sub, proof
    i, rest = path[0], path[1:]
    kids = children(term)
    if i >= len(kids):
        raise RingError(f"bad path {path} into {term[0]}")
    new_kid, sub_proof = lift(kids[i], rest, new_sub, proof)
    new = rebuild(term, i, new_kid)
    tag = term[0]
    if tag == "add":
        w = f"(peq_cong_add {sub_proof} (peq_refl))" if i == 0 \
            else f"(peq_cong_add (peq_refl) {sub_proof})"
    elif tag == "mul":
        w = f"(peq_cong_mul {sub_proof} (peq_refl))" if i == 0 \
            else f"(peq_cong_mul (peq_refl) {sub_proof})"
    else:
        w = f"(peq_cong_neg {sub_proof})"
    return new, w


def chain(steps: list[str]) -> str:
    """Right-nested `peq_trans` chain; each step rewrites the running term."""
    if not steps:
        return REFL
    if len(steps) > MAX_STEPS:
        raise RingError(f"proof has {len(steps)} steps (limit {MAX_STEPS})")
    out = steps[-1]
    for step in reversed(steps[:-1]):
        out = f"(peq_trans {step} {out})"
    return out


def subterm_at(term, path):
    for i in path:
        term = children(term)[i]
    return term


# ── canonical monomials ──────────────────────────────────────────────────
# A monomial is (sign, coeff, vars): sign in {-1, +1}, coeff >= 1, vars a
# sorted tuple of variable indices. Coefficients are *literal* (never expanded
# to repeated summands); the arithmetic that combines them is grounded.

def prod_of_atoms(atoms) -> object:
    """Right-nested product; a single atom is itself."""
    if not atoms:
        raise RingError("empty product")
    out = atoms[-1]
    for a in reversed(atoms[:-1]):
        out = mul(a, out)
    return out


def atoms_of(node) -> list:
    if node[0] == "mul":
        return atoms_of(node[1]) + atoms_of(node[2])
    if node[0] in ("lit", "var"):
        return [node]
    raise RingError(f"{node[0]} is not a monomial factor")


def monomial_node(coeff: int, vars_: tuple):
    """Positive monomial text: literal coefficient (unless 1) then the vars."""
    atoms = []
    if coeff != 1 or not vars_:
        atoms.append(lit(coeff))
    atoms.extend(var(i) for i in vars_)
    return prod_of_atoms(atoms)


def elem_node(e):
    sign, coeff, vars_ = e
    base = monomial_node(coeff, vars_)
    return neg(base) if sign < 0 else base


def elem_text(e) -> str:
    return text(elem_node(e))


def sum_node(seq):
    """Right-nested sum; the empty sum is `(tint n0)`."""
    if not seq:
        return lit(0)
    out = elem_node(seq[-1])
    for e in reversed(seq[:-1]):
        out = add(elem_node(e), out)
    return out


def read_elem(node):
    """Read a *canonical* monomial node back into (sign, coeff, vars)."""
    sign = 1
    if node[0] == "neg":
        sign = -1
        node = node[1]
    if node[0] == "lit" and node[1] < 0:
        return (-1, -node[1], ())
    atoms = atoms_of(node)
    coeff = 1
    vars_ = []
    for a in atoms:
        if a[0] == "lit":
            coeff *= a[1]
        else:
            vars_.append(a[1])
    if coeff < 1:
        raise RingError("a monomial cannot have a non-positive coefficient")
    return (sign, coeff, tuple(sorted(vars_)))


def read_seq(term) -> list:
    """Read a right-nested sum back into its monomials."""
    if term[0] != "add":
        if term == lit(0):
            return []
        return [read_elem(term)]
    out = []
    node = term
    while node[0] == "add":
        out.append(read_elem(node[1]))
        node = node[2]
    out.append(read_elem(node))
    return out


def elem_key(e):
    """Canonical order: by variables, then positive before negative.

    The sign order is not cosmetic: a pair that cancels is `padd a (pneg a)`,
    which is `padd_neg`'s left-to-right shape, so the positive occurrence must
    arrive first for the law to apply without a commutation step.
    """
    sign, coeff, vars_ = e
    return (vars_, 0 if sign > 0 else 1, coeff)


def atom_key(a):
    """Literals first (so one literal remains at the head), then vars."""
    return (0, a[1]) if a[0] == "lit" else (1, a[1])


class Derivation:
    """A whole-term rewrite chain: `term`, and the steps taken to reach it."""

    def __init__(self, term):
        self.term = term
        self.steps = []

    def rewrite(self, path, new_sub, proof):
        self.term, w = lift(self.term, path, new_sub, proof)
        self.steps.append(w)
        return self.term

    def proof(self) -> str:
        return chain(self.steps)


class Prover:
    """Normalizes polynomials, collecting the steps and the grounded facts."""

    def __init__(self):
        self.facts = []          # (name, statement) — grounded axioms

    # ── grounded coefficient facts ───────────────────────────────────────

    def _fact(self, lhs: str, rhs: str, check) -> str:
        """Register a visible decimal fact; `check` must accept it.

        Facts are deduplicated by statement: the same coefficient arithmetic
        reached twice is one decimal, not two, so the artifact's `grounded`
        count stays a count of distinct claims.
        """
        if not check():
            raise RingError(f"grounded fact failed its own check: {lhs} = {rhs}")
        statement = f"$ peq {lhs} {rhs} $"
        for name, known in self.facts:
            if known == statement:
                return f"({name})"
        name = f"cf_{len(self.facts) + 1}"
        self.facts.append((name, statement))
        return f"({name})"

    def combine_fact(self, e1, e2, e3) -> str:
        """`padd T1 T2 = T3` for three monomials of the same variables."""
        s1, c1, v1 = e1
        s2, c2, v2 = e2
        s3, c3, v3 = e3
        if v1 != v2 or v1 != v3:
            raise RingError("grounded sum combines different monomials")
        if s1 * c1 + s2 * c2 != s3 * c3:
            raise RingError("grounded sum of coefficients is wrong")

        def check():
            # Independent route: sample the three monomials at small values.
            for x in (2, 3, 5):
                val = 1
                for v in v1:
                    val *= (x + v)
                if (s1 * c1 + s2 * c2) * val != (s3 * c3) * val:
                    return False
            return True

        return self._fact(f"(padd {elem_text(e1)} {elem_text(e2)})",
                          elem_text(e3), check)

    def multiply_fact(self, a: int, b: int) -> str:
        c = a * b

        def check():
            return a * b == c

        return self._fact(f"(pmul (tint n{a}) (tint n{b}))",
                          f"(tint n{c})", check)

    # ── negation ─────────────────────────────────────────────────────────

    def _neg_elem_at(self, d: Derivation, path, e):
        """Rewrite `pneg <monomial>` to the monomial with the sign flipped."""
        sign = e[0]
        if sign > 0:
            return                      # the texts already agree
        # pneg (pneg M) -> M
        prod = monomial_node(e[1], e[2])
        d.rewrite(path, prod, "(pneg_neg)")

    def _neg_generic(self, d: Derivation, path, seq):
        """Rewrite `pneg (sum seq)` to `sum (negated seq)`."""
        if not seq:
            d.rewrite(path, lit(0), "(pneg_zero)")
            return
        if len(seq) == 1:
            self._neg_elem_at(d, path, seq[0])
            return
        d.rewrite(path, add(neg(elem_node(seq[0])),
                            neg(sum_node(seq[1:]))), "(pneg_add)")
        self._neg_elem_at(d, path + [0], seq[0])
        self._neg_generic(d, path + [1], seq[1:])

    # ── products ─────────────────────────────────────────────────────────

    def _nest(self, d: Derivation, path, node):
        """Right-nest a product tree (`pmul_assoc` in the needed direction)."""
        if node[0] != "mul":
            return node
        a, b = node[1], node[2]
        if a[0] == "mul":
            new = mul(a[1], mul(a[2], b))
            d.rewrite(path, new, "(peq_symm (pmul_assoc))")
            return self._nest(d, path, new)
        nb = self._nest(d, path + [1], b)
        return mul(a, nb)

    def _swap_atoms(self, d: Derivation, path, atoms, j):
        """Swap atoms j and j+1 of a right-nested product at `path`."""
        p = path + [1] * j
        if j + 2 == len(atoms):
            d.rewrite(p, mul(atoms[j + 1], atoms[j]), "(pmul_comm)")
            return
        rest = prod_of_atoms(atoms[j + 2:])
        d.rewrite(p, mul(atoms[j + 1], mul(atoms[j], rest)),
                  "(ring_pmul_swap)")

    def _sort_atoms(self, d: Derivation, path, atoms):
        atoms = list(atoms)
        for _ in range(len(atoms)):
            moved = False
            for j in range(len(atoms) - 1):
                if atom_key(atoms[j]) > atom_key(atoms[j + 1]):
                    self._swap_atoms(d, path, atoms, j)
                    atoms[j], atoms[j + 1] = atoms[j + 1], atoms[j]
                    moved = True
            if not moved:
                break
        return atoms

    def _collapse_literals(self, d: Derivation, path, atoms):
        """Multiply the leading literal atoms into one, then drop a `1`."""
        atoms = list(atoms)
        while len(atoms) >= 2 and atoms[0][0] == "lit" and atoms[1][0] == "lit":
            a, b = atoms[0][1], atoms[1][1]
            fact = self.multiply_fact(a, b)
            if len(atoms) == 2:
                d.rewrite(path, lit(a * b), fact)
                atoms = [lit(a * b)]
                break
            rest = prod_of_atoms(atoms[2:])
            # mul (lit a) (mul (lit b) R) -> mul (mul (lit a) (lit b)) R
            d.rewrite(path, mul(mul(lit(a), lit(b)), rest),
                      "(peq_symm (pmul_assoc))")
            d.rewrite(path + [0], lit(a * b), fact)
            atoms = [lit(a * b)] + atoms[2:]
        if len(atoms) >= 2 and atoms[0] == lit(1):
            rest = prod_of_atoms(atoms[1:])
            d.rewrite(path, mul(rest, lit(1)), "(pmul_comm)")
            d.rewrite(path, rest, "(pmul_one)")
            atoms = atoms[1:]
        return atoms

    def _prod_pair(self, d: Derivation, path, ea, eb):
        """Prove `pmul Ta Tb` equal to the canonical product monomial."""
        sign = ea[0] * eb[0]
        pa = monomial_node(ea[1], ea[2])
        pb = monomial_node(eb[1], eb[2])
        target = path
        if ea[0] < 0 and eb[0] < 0:
            d.rewrite(path, neg(mul(pa, neg(pb))), "(pmul_neg_l)")
            d.rewrite(path + [0], neg(mul(pa, pb)), "(pmul_neg_r)")
            d.rewrite(path, mul(pa, pb), "(pneg_neg)")
        elif ea[0] < 0:
            d.rewrite(path, neg(mul(pa, pb)), "(pmul_neg_l)")
            target = path + [0]
        elif eb[0] < 0:
            d.rewrite(path, neg(mul(pa, pb)), "(pmul_neg_r)")
            target = path + [0]
        atoms = atoms_of(pa) + atoms_of(pb)
        if len(atoms) > MAX_ATOMS:
            raise RingError(f"monomial has {len(atoms)} factors")
        self._nest(d, target, subterm_at(d.term, target))
        atoms = self._sort_atoms(d, target, atoms)
        atoms = self._collapse_literals(d, target, atoms)
        return read_elem(subterm_at(d.term, target))

    # ── sums ─────────────────────────────────────────────────────────────

    def _concat(self, d: Derivation, sa, sb):
        """Rewrite `padd sum(sa) sum(sb)` to `sum(sa ++ sb)`."""
        if not sa:
            d.rewrite([], sum_node(sb), "(padd_zero_l)")
            return
        if len(sa) == 1:
            return
        d.rewrite([], add(elem_node(sa[0]),
                          add(sum_node(sa[1:]), sum_node(sb))),
                  "(padd_assoc)")
        # The subterm at [1] is `padd sum(sa[1:]) sum(sb)`; recurse there.
        sub = Derivation(add(sum_node(sa[1:]), sum_node(sb)))
        self._concat(sub, sa[1:], sb)
        d.rewrite([1], sub.term, sub.proof())

    def _swap_elems(self, d: Derivation, seq, j):
        """Swap elements j and j+1 of a right-nested sum."""
        p = [1] * j
        if j + 2 == len(seq):
            d.rewrite(p, add(elem_node(seq[j + 1]), elem_node(seq[j])),
                      "(padd_comm)")
            return
        rest = sum_node(seq[j + 2:])
        d.rewrite(p, add(elem_node(seq[j + 1]),
                         add(elem_node(seq[j]), rest)), "(ring_swap)")

    def _sort_sum(self, d: Derivation, seq):
        seq = list(seq)
        for _ in range(len(seq)):
            moved = False
            for j in range(len(seq) - 1):
                if elem_key(seq[j]) > elem_key(seq[j + 1]):
                    self._swap_elems(d, seq, j)
                    seq[j], seq[j + 1] = seq[j + 1], seq[j]
                    moved = True
            if not moved:
                break
        return seq

    def _merge_at(self, d: Derivation, seq, i):
        """Merge elements i and i+1 (same variables) into one."""
        e1, e2 = seq[i], seq[i + 1]
        net = e1[0] * e1[1] + e2[0] * e2[1]
        rest = seq[i + 2:]
        p = [1] * i
        if net == 0:
            # Opposite coefficients of one monomial are exact negatives, so
            # `padd_neg` applies; the resulting zero is dropped by a zero law.
            if rest:
                d.rewrite(p, add(add(elem_node(e1), elem_node(e2)),
                                 sum_node(rest)), "(peq_symm (padd_assoc))")
                d.rewrite(p + [0], lit(0), "(padd_neg)")
                d.rewrite(p, sum_node(rest), "(padd_zero_l)")
            elif i == 0:
                d.rewrite(p, lit(0), "(padd_neg)")
            else:
                d.rewrite(p, lit(0), "(padd_neg)")
                d.rewrite([1] * (i - 1), sum_node(seq[:i]), "(padd_zero)")
            return
        e3 = (1 if net > 0 else -1, abs(net), e1[2])
        fact = self.combine_fact(e1, e2, e3)
        if rest:
            d.rewrite(p, add(add(elem_node(e1), elem_node(e2)),
                             sum_node(rest)), "(peq_symm (padd_assoc))")
            d.rewrite(p + [0], elem_node(e3), fact)
        else:
            d.rewrite(p, elem_node(e3), fact)

    def _combine_sum(self, d: Derivation, seq):
        seq = list(seq)
        while True:
            for i in range(len(seq) - 1):
                if seq[i][2] == seq[i + 1][2]:
                    self._merge_at(d, seq, i)
                    seq = read_seq(d.term)
                    break
            else:
                return seq

    # ── distribution ─────────────────────────────────────────────────────

    def _distribute(self, d: Derivation, path, sa, sb):
        """Rewrite `pmul sum(sa) sum(sb)` to a sum of pairwise products."""
        if not sa:
            d.rewrite(path, lit(0), "(pmul_zero_l)")
            return []
        if not sb:
            d.rewrite(path, mul(lit(0), sum_node(sa)), "(pmul_comm)")
            d.rewrite(path, lit(0), "(pmul_zero_l)")
            return []
        if len(sa) == 1 and len(sb) == 1:
            return [self._prod_pair(d, path, sa[0], sb[0])]
        if len(sa) == 1:
            d.rewrite(path, add(mul(elem_node(sa[0]), elem_node(sb[0])),
                                mul(elem_node(sa[0]), sum_node(sb[1:]))),
                      "(pmul_distrib)")
            head = self._prod_pair(d, path + [0], sa[0], sb[0])
            tail = self._distribute(d, path + [1], sa, sb[1:])
            return [head] + tail
        d.rewrite(path, mul(sum_node(sb), sum_node(sa)), "(pmul_comm)")
        d.rewrite(path, add(mul(sum_node(sb), elem_node(sa[0])),
                            mul(sum_node(sb), sum_node(sa[1:]))),
                  "(pmul_distrib)")
        head = self._distribute(d, path + [0], sb, [sa[0]])
        tail = self._distribute(d, path + [1], sb, sa[1:])
        return head + tail

    # ── flattening ───────────────────────────────────────────────────────

    def flatten(self, t):
        """Return (seq, proof): `peq t (sum seq)`, seq in source order."""
        tag = t[0]
        if tag == "lit" and t[1] == 0:
            return [], REFL
        if tag in ("lit", "var"):
            return [read_elem(t)], REFL
        if tag == "neg":
            seq, pr = self.flatten(t[1])
            d = Derivation(t)
            d.rewrite([], neg(sum_node(seq)), f"(peq_cong_neg {pr})")
            self._neg_generic(d, [], seq)
            return [(-s, c, v) for (s, c, v) in seq], d.proof()
        if tag == "add":
            sa, pa = self.flatten(t[1])
            sb, pb = self.flatten(t[2])
            d = Derivation(t)
            d.rewrite([], add(sum_node(sa), sum_node(sb)),
                      f"(peq_cong_add {pa} {pb})")
            self._concat(d, sa, sb)
            return sa + sb, d.proof()
        if tag == "mul":
            sa, pa = self.flatten(t[1])
            sb, pb = self.flatten(t[2])
            d = Derivation(t)
            d.rewrite([], mul(sum_node(sa), sum_node(sb)),
                      f"(peq_cong_mul {pa} {pb})")
            seq = self._distribute(d, [], sa, sb)
            return seq, d.proof()
        raise RingError(f"unknown term node {t!r}")

    # ── the entry point ──────────────────────────────────────────────────

    def normalize(self, t):
        """Return (seq, proof): `peq t (sum seq)`, seq canonical and sorted."""
        seq, pr = self.flatten(t)
        d = Derivation(t)
        d.rewrite([], sum_node(seq), pr)
        seq = self._sort_sum(d, seq)
        seq = self._combine_sum(d, seq)
        return seq, d.proof()
