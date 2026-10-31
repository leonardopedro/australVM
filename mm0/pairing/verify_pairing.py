#!/usr/bin/env python3
"""verify_pairing.py - regenerate, verify, and stress-test the mm0/mm1 pairing.

Steps:
  1. Regenerate full2.mm0 from full2.mm1 (gen_mm0.py) and assert the output
     byte-matches the checked-in file (determinism).
  2. POSITIVE: mm0-c must accept the checked-in (full2.mmb, full2.mm0) pair.
  3. REBUILD: mm0-rs compile full2.mm1 to a fresh .mmb in a temp dir, and
     mm0-c must accept it against the checked-in .mm0 (source <-> binary
     <-> mm0 agree without trusting the checked-in binary).
  4. NEGATIVE: ten deliberately-broken .mm0 variants must all be REJECTED -
     removed axioms, reordered statements, removed public statements,
     an inserted local statement, dropped defs/notations/delimiter, and a
     weakened sort modifier.
  5. TOLERATED: one variant that mm0-c correctly accepts (adjacent notation
     lines swapped), pinning the property that notation lines are consumed
     but never stream-paired.
  6. Optional ground truth: the upstream peano pair, if an mm0 checkout with
     examples/ is available (override path with MM0_EXAMPLES).

Pairing rule under test:
  public statements (sort/term/def/axiom/`pub theorem`) must appear in the
  .mm0 in exact .mmb statement-stream order; local statements (plain
  `theorem`, `local def`) must NOT appear at all; `delimiter`/`notation`/
  `prefix`/`infixl` lines are consumed by the verifier but not paired -
  though their content still matters, because later statements must parse.

Exit status: 0 iff every expectation holds.
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
MM0C = shutil.which("mm0-c")
MM0RS = shutil.which("mm0-rs")
PEANO_DIR = os.environ.get("MM0_EXAMPLES", "/tmp/mm0/examples")
MMB = os.path.join(HERE, "full2.mmb")
MM0 = os.path.join(HERE, "full2.mm0")

results = []


def mm0c(mmb, mm0):
    with open(mm0) as f:
        return subprocess.run([MM0C, mmb], stdin=f,
                              capture_output=True, text=True).returncode


def check(name, ok, detail=""):
    results.append((name, ok))
    tag = "ok  " if ok else "FAIL"
    print(f"  [{tag}] {name}" + (f"  {detail}" if detail else ""))


def must_accept(name, mmb, mm0):
    rc = mm0c(mmb, mm0)
    check(name, rc == 0, f"(mm0-c exit {rc})")


def must_reject(name, mmb, mm0):
    rc = mm0c(mmb, mm0)
    check(name, rc != 0, f"(mm0-c exit {rc}, expected nonzero)")


def main():
    if not MM0C or not MM0RS:
        sys.exit("verify_pairing: need `mm0-c` and `mm0-rs` on PATH "
                 "(build github.com/digama0/mm0)")

    lines = open(MM0).read().splitlines(True)

    def idx(prefix):
        return next(k for k, l in enumerate(lines) if l.startswith(prefix))

    def variant(name, newlines):
        p = os.path.join(TMP, name)
        open(p, "w").writelines(newlines)
        return p

    with tempfile.TemporaryDirectory() as TMP:
        print("== 1. regenerate companion (determinism vs checked-in) ==")
        before = open(MM0, "rb").read()
        out = os.path.join(TMP, "full2.regen.mm0")
        r = subprocess.run([sys.executable, os.path.join(HERE, "gen_mm0.py"),
                            os.path.join(HERE, "full2.mm1"), out],
                           capture_output=True, text=True)
        if r.returncode != 0:
            print(r.stdout + r.stderr)
            sys.exit(2)
        print("  " + r.stdout.strip())
        check("gen_mm0.py reproduces checked-in full2.mm0",
              before == open(out, "rb").read())

        print("== 2. positive: checked-in pair verifies ==")
        must_accept("full2.mmb <-> full2.mm0", MMB, MM0)

        print("== 3. rebuild from source, re-verify against checked-in .mm0 ==")
        fresh = os.path.join(TMP, "full2.fresh.mmb")
        r = subprocess.run([MM0RS, "compile",
                            os.path.join(HERE, "full2.mm1"), fresh],
                           capture_output=True, text=True)
        tail = (r.stdout + r.stderr).strip().splitlines()
        check("mm0-rs compile full2.mm1", r.returncode == 0,
              tail[-1] if tail else "")
        if r.returncode == 0:
            must_accept("full2.fresh.mmb <-> full2.mm0", fresh, MM0)

        print("== 4. negative controls: every broken variant rejected ==")
        must_reject("neg01: all 22 axioms removed", MMB,
                    variant("neg01.mm0",
                            [l for l in lines if not l.startswith("axiom ")]))

        i, j = idx("axiom ax_1 "), idx("axiom ax_2 ")
        sw = list(lines)
        sw[i], sw[j] = sw[j], sw[i]
        must_reject("neg02: ax_1/ax_2 order swapped", MMB,
                    variant("neg02.mm0", sw))

        must_reject("neg03: public theorems ifppos/ifpneg removed", MMB,
                    variant("neg03.mm0",
                            [l for k, l in enumerate(lines)
                             if k not in (idx("theorem ifppos"),
                                          idx("theorem ifpneg"))]))

        ins = list(lines)
        ins.insert(idx("axiom ax_1 "),
                   "theorem syl (h1: $ b $) (h2: $ a -> b $): $ a $;\n")
        must_reject("neg04: extra local theorem statement inserted", MMB,
                    variant("neg04.mm0", ins))

        k = idx("theorem logos_cnl_addcom")
        claims = [k, k + 1, k + 2]
        must_reject("neg05: CNL claims removed", MMB,
                    variant("neg05.mm0",
                            [l for m, l in enumerate(lines)
                             if m not in claims]))

        k = idx("infixl eq:")
        must_reject("neg06: infixl eq notation moved to end "
                    "(breaks later statements)", MMB,
                    variant("neg06.mm0",
                            lines[:k] + lines[k + 1:] + [lines[k]]))

        must_reject("neg07: public def `eq` dropped", MMB,
                    variant("neg07.mm0",
                            [l for l in lines if not l.startswith("def eq ")]))

        must_reject("neg08: delimiter line dropped", MMB,
                    variant("neg08.mm0",
                            [l for l in lines
                             if not l.startswith("delimiter ")]))

        must_reject("neg09: sort modifier weakened (provable sort -> sort)",
                    MMB,
                    variant("neg09.mm0",
                            [l.replace("provable sort Prop;", "sort Prop;")
                             for l in lines]))

        must_reject("neg10: notation sb dropped (peano5 needs [0 / x])", MMB,
                    variant("neg10.mm0",
                            [l for l in lines
                             if not l.startswith("notation sb ")]))

        print("== 5. tolerated variant (documents what is NOT paired) ==")
        a, b = idx("infixl eq:"), idx("infixl tadd:")
        tv = list(lines)
        tv[a], tv[b] = tv[b], tv[a]
        must_accept("tol01: adjacent notation lines swapped "
                    "(notations are never stream-paired)",
                    MMB, variant("tol01.mm0", tv))

        print("== 6. upstream ground truth (optional) ==")
        peano_mm0 = os.path.join(PEANO_DIR, "peano.mm0")
        peano_mmb = os.path.join(HERE, "peano_ref.mmb")
        if os.path.exists(peano_mm0) and os.path.exists(peano_mmb):
            must_accept("peano.mm0 <-> peano_ref.mmb", peano_mmb, peano_mm0)
        else:
            print("  [skip] upstream examples not found "
                  f"(set MM0_EXAMPLES; looked in {PEANO_DIR})")

    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} checks passed"
          + (f"; FAILED: {failed}" if failed else " - pairing rule verified"))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
