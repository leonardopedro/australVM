#!/usr/bin/env python3
"""verify_pairing.py - regenerate, verify, and stress-test the mm0/mm1 pairing.

Steps:
  1. Regenerate full2.mm0 / full3.mm0 from their .mm1 sources (gen_mm0.py)
     and assert the output matches the checked-in files (determinism).
  2. POSITIVE: mm0-c must accept each checked-in (.mmb, .mm0) pair.
  3. REBUILD: mm0-rs compile each .mm1 to a fresh .mmb in a temp dir, and
     mm0-c must accept it against the checked-in .mm0 (source <-> binary
     <-> mm0 agree without relying on the checked-in binaries).
  4. NEGATIVE: five deliberately-broken .mm0 variants must all be REJECTED.
  5. Optional ground truth: the upstream peano pair, if an mm0 checkout with
     examples/ is available (override path with MM0_EXAMPLES).

Pairing rule under test:
  public statements (sort/term/def/axiom/`pub theorem`) must appear in the
  .mm0 in exact .mmb statement-stream order; local statements (plain
  `theorem`, `local def`) must NOT appear at all; `delimiter`/`notation`/
  `prefix`/`infixl` lines are consumed by the verifier but not paired.

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


def gen(src, dst):
    r = subprocess.run([sys.executable, os.path.join(HERE, "gen_mm0.py"),
                        os.path.join(HERE, src), dst],
                       capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stdout + r.stderr)
        sys.exit(2)
    return r.stdout.strip()


def main():
    if not MM0C or not MM0RS:
        sys.exit("verify_pairing: need `mm0-c` and `mm0-rs` on PATH "
                 "(build github.com/digama0/mm0)")

    with tempfile.TemporaryDirectory() as tmp:
        print("== 1. regenerate companions (determinism vs checked-in) ==")
        for stem in ("full2", "full3"):
            before = open(os.path.join(HERE, f"{stem}.mm0"), "rb").read()
            out = os.path.join(tmp, f"{stem}.regen.mm0")
            print("  " + gen(f"{stem}.mm1", out))
            after = open(out, "rb").read()
            check(f"gen_mm0.py reproduces checked-in {stem}.mm0",
                  before == after)

        print("== 2. positive: checked-in pairs verify ==")
        must_accept("full2.mmb <-> full2.mm0",
                    os.path.join(HERE, "full2.mmb"),
                    os.path.join(HERE, "full2.mm0"))
        must_accept("full3.mmb <-> full3.mm0",
                    os.path.join(HERE, "full3.mmb"),
                    os.path.join(HERE, "full3.mm0"))

        print("== 3. rebuild from source, re-verify against checked-in .mm0 ==")
        for stem in ("full2", "full3"):
            fresh = os.path.join(tmp, f"{stem}.fresh.mmb")
            r = subprocess.run([MM0RS, "compile",
                                os.path.join(HERE, f"{stem}.mm1"), fresh],
                               capture_output=True, text=True)
            tail = (r.stdout + r.stderr).strip().splitlines()
            check(f"mm0-rs compile {stem}.mm1", r.returncode == 0,
                  tail[-1] if tail else "")
            if r.returncode == 0:
                must_accept(f"{stem}.fresh.mmb <-> {stem}.mm0", fresh,
                            os.path.join(HERE, f"{stem}.mm0"))

        print("== 4. negative controls: every broken variant rejected ==")
        f2 = open(os.path.join(HERE, "full2.mm0")).read().splitlines(True)
        f3 = open(os.path.join(HERE, "full3.mm0")).read().splitlines(True)
        mmb2 = os.path.join(HERE, "full2.mmb")
        mmb3 = os.path.join(HERE, "full3.mmb")

        def write(name, lines):
            p = os.path.join(tmp, name)
            open(p, "w").writelines(lines)
            return p

        must_reject("neg1: all 22 axioms removed",
                    mmb2, write("neg1.mm0",
                                [l for l in f2 if not l.startswith("axiom ")]))

        i = next(k for k, l in enumerate(f2) if l.startswith("axiom ax_1 "))
        j = next(k for k, l in enumerate(f2) if l.startswith("axiom ax_2 "))
        sw = list(f2)
        sw[i], sw[j] = sw[j], sw[i]
        must_reject("neg2: ax_1/ax_2 order swapped",
                    mmb2, write("neg2.mm0", sw))

        must_reject("neg3: public theorems ifppos/ifpneg removed",
                    mmb2, write("neg3.mm0",
                                [l for l in f2
                                 if not l.startswith(("theorem ifppos",
                                                      "theorem ifpneg"))]))

        k = next(k for k, l in enumerate(f2) if l.startswith("axiom ax_1 "))
        ins = list(f2)
        ins.insert(k, "theorem syl (h1: $ b $) (h2: $ a -> b $): $ a $;\n")
        must_reject("neg4: extra local theorem statement inserted",
                    mmb2, write("neg4.mm0", ins))

        must_reject("neg5: full3 CNL claims removed",
                    mmb3, write("neg5.mm0",
                                [l for l in f3
                                 if not l.startswith("theorem logos_cnl_")]))

        print("== 5. upstream ground truth (optional) ==")
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
