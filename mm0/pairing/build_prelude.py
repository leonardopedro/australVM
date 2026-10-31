#!/usr/bin/env python3
"""Extract peano.mm1's derived-lemma sections, adapted to logos_math's names.

Ranges (1-indexed, inclusive):
  R1: 20..1490   generic mm1 do-blocks + propositional/quantifier/nf/sb derived lemmas
  R2: 1799..1840 numerals, peano1/peano2/peano5, suceq, `ind`
  R3: 2032..2051 arithmetic axioms (addeq..mulS) + congruence generation
  R4: 2057..2082 the derived addition theorems, through `addcom` itself

Declarations owned by logos_math's head are dropped; renames:
  wff -> Prop, nat -> Term, d0 -> tz, 'add/'mul/'eq/'suc -> 'tadd/'tmul/'teq/'ts
"""
import re

src = open('/tmp/mm0/examples/peano.mm1').read().split('\n')


def seg(a, b):
    return src[a - 1:b]


lines = (seg(20, 1490) + ['', '-- ==== numerals + peano axioms + ind ===='] +
         seg(1799, 1840) + ['', '-- ==== arithmetic axioms ===='] + seg(2032, 2051) +
         ['', '-- ==== addition theorems ===='] + seg(2057, 2082))

out = []
skip = False
for l in lines:
    if skip:
        skip = False
        continue
    s = l
    # declarations owned by the head
    if s.startswith('@(add-eval 0) term d0: nat;'):
        # the head owns tz and the `0` notation
        continue
    if s.startswith('@(add-eval @ fn (a) {(eval a) + 1})'):
        skip = True  # drops the term suc line too
        continue
    if s.startswith('term suc (n: nat);'):
        continue
    if s == 'sort nat;':
        continue
    if s.startswith('term al {x: nat}'):
        continue
    if s.startswith('def ex {x: nat}'):
        skip = True  # drops the prefix ex line too
        continue
    if s.startswith('@(add-eval @ fn (a b) {(eval a) = (eval b)})'):
        skip = True  # drops the term eq line too
        continue
    if s.startswith('term eq (a b: nat):'):
        continue
    if re.match(r'^axiom ax_(gen|4|5|6|7|10|11|12) ', s):
        continue
    out.append(s)

text = '\n'.join(out)
text = re.sub(r'\bwff\b', 'Prop', text)
text = re.sub(r'\bnat\b', 'Term', text)
text = re.sub(r'\bd0\b', 'tz', text)
text = re.sub(r"'add\b", "'tadd", text)
text = re.sub(r"'mul\b", "'tmul", text)
text = re.sub(r"'suc\b", "'ts", text)
# `add-eqN-thms 'tadd` names its generated lemmas after the constant, so the
# proofs' references must follow: addeq1 -> taddeq1, addeq2d -> taddeq2d.
text = re.sub(r"\baddeq([12])(d?)\b", r"taddeq\1\2", text)
text = re.sub(r"\bmuleq([12])(d?)\b", r"tmuleq\1\2", text)
# evaluator registrations refer to a per-constant evaluator we don't port; the
# defs themselves are what the prelude needs.
text = re.sub(r"@\((?:add-eval(?:[^()]|\([^()]*\))*)\)\s*", "", text)
open('/tmp/ind/prelude.mm1', 'w').write(text + '\n')
print('prelude lines:', len(out))
