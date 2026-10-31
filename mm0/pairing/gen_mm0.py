#!/usr/bin/env python3
"""gen_mm0.py <in.mm1> <out.mm0>

Produces the mm0 companion for a compiled .mmb by filtering the .mm1 source:
  - keep, in source order: sort/term/def/axiom/pub theorem/delimiter/notation
    and the prefix/infixl/infixr/notation lines attached to public names;
  - drop: plain `theorem` (LocalThm), `local def` (LocalDef), `abstract def`
    bodies (kept as bodyless `def`), lisp forms, `do` blocks, @ annotations,
    annotations-attached statements, and -- comments.
Public statements must appear in .mm0 in exactly the .mmb statement-stream
order; local statements must NOT appear. Verified against mm0-c.
"""
import os, re, sys

KEEP_HEAD = {"sort", "provable", "strict", "term", "def", "axiom",
             "delimiter", "notation", "prefix", "infixl", "infixr",
             "theorem", "pub", "abstract"}
KNOWN = KEEP_HEAD | {"local", "lisp", "do"}

# notation statements over local names are dropped; the only public notation
# needing mm0-typed binders is `sb` (mm1 infers binder types, mm0 requires them).
NOTATION_FIX = {
    "sb": "notation sb (a: Term) {x: Term} (p: Prop x): Prop =",
}

class Scanner:
    def __init__(self, s):
        self.s = s; self.i = 0; self.n = len(s)

    def err(self, msg):
        line = self.s.count("\n", 0, self.i) + 1
        raise SystemExit(f"gen_mm0: line {line}: {msg}: {self.s[self.i:self.i+60]!r}")

    def ws(self):
        s, n = self.s, self.n
        while self.i < n:
            c = s[self.i]
            if c in " \t\r\n":
                self.i += 1
            elif s.startswith("--", self.i):
                j = s.find("\n", self.i)
                self.i = n if j < 0 else j + 1
            else:
                return

    def word(self):
        m = re.match(r"[A-Za-z_][A-Za-z0-9_?!+*/<>=|&$%^~#@.]*", self.s[self.i:])
        if not m: self.err("expected identifier")
        self.i += len(m.group(0)); return m.group(0)

    def balanced(self, open_c, close_c):
        """Skip a balanced (..) / {..} datum (string/comment aware)."""
        depth = 0; s = self.s; n = self.n
        while self.i < n:
            c = s[self.i]
            if c == '"':
                self.i += 1
                while self.i < n and s[self.i] != '"':
                    if s[self.i] == "\\": self.i += 1
                    self.i += 1
                self.i += 1; continue
            if s.startswith("--", self.i):
                j = s.find("\n", self.i); self.i = n if j < 0 else j + 1; continue
            if c == open_c: depth += 1
            elif c == close_c:
                depth -= 1
                self.i += 1
                if depth == 0: return
                continue
            self.i += 1
        self.err("unbalanced datum")

    def statement(self, start=None):
        """Read from start (keyword start) through depth-0 ';' outside math."""
        if start is None: start = self.i
        self.i = start
        depth = 0; in_math = False; s = self.s; n = self.n
        while self.i < n:
            c = s[self.i]
            if in_math:
                if c == "$": in_math = False
                self.i += 1; continue
            if c == "$": in_math = True; self.i += 1; continue
            if s.startswith("--", self.i):
                j = s.find("\n", self.i); self.i = n if j < 0 else j + 1; continue
            if c == '"':
                self.i += 1
                while self.i < n and s[self.i] != '"':
                    if s[self.i] == "\\": self.i += 1
                    self.i += 1
                self.i += 1; continue
            if c in "([{": depth += 1
            elif c in ")]}": depth -= 1
            elif c == ";" and depth == 0:
                self.i += 1
                return s[start:self.i]
            self.i += 1
        self.err("statement without ';'")

    def lisp_tail(self):
        """Skip optional ';' after a lisp datum (only if it directly follows)."""
        j = self.i
        while j < self.n and self.s[j] in " \t\r\n": j += 1
        if j < self.n and self.s[j] == ";": self.i = j + 1

    @staticmethod
    def top_eq(text):
        """Index of first top-level '=' outside math/parens/brackets/braces."""
        depth = 0; in_math = False
        for k, c in enumerate(text):
            if in_math:
                if c == "$": in_math = False
                continue
            if c == "$": in_math = True
            elif text.startswith("--", k): break
            elif c in "([{": depth += 1
            elif c in ")]}": depth -= 1
            elif c == "=" and depth == 0: return k
        return -1


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: gen_mm0.py <in.mm1> <out.mm0>")
    src = open(sys.argv[1]).read()
    sc = Scanner(src)
    out = []
    local_names = set()
    warnings = []

    while True:
        sc.ws()
        if sc.i >= sc.n: break
        c = sc.s[sc.i]

        if c == "@":
            sc.i += 1
            if sc.i < sc.n and sc.s[sc.i] == "(":
                sc.balanced("(", ")")
            elif sc.i < sc.n and (sc.s[sc.i].isalpha() or sc.s[sc.i] == "_"):
                sc.word()
            else:
                sc.err("bad annotation")
            continue

        if c == "(":
            sc.balanced("(", ")"); sc.lisp_tail(); continue

        if not (c.isalpha() or c == "_"):
            sc.err("unexpected character")

        pos = sc.i
        kw = sc.word()

        if kw == "do":
            sc.ws()
            if sc.i < sc.n and sc.s[sc.i] in "({":
                o, cl = sc.s[sc.i], {"(": ")", "{": "}"}[sc.s[sc.i]]
                sc.balanced(o, cl)
            else:
                sc.err("expected datum after do")
            sc.lisp_tail()
            continue

        if kw in ("local", "lisp"):
            text = sc.statement(pos)
            if kw == "local":
                m = re.match(r"local\s+(?:term|def)\s+([A-Za-z0-9_?!+*/<>=|&$%^~#@.]*)", text)
                if m: local_names.add(m.group(1))
                else: warnings.append(f"unparsed local: {text[:60]!r}")
            continue

        if kw not in KNOWN:
            sc.err(f"unknown top-level keyword {kw!r}")

        text = sc.statement(pos)

        if kw == "pub":
            m = re.match(r"pub\s+(theorem|def|axiom|term|sort)\b", text)
            if not m: sc.err("unhandled pub form")
            body = re.sub(r"^pub\s+", "", text)
            k = Scanner.top_eq(body)
            if m.group(1) == "theorem":
                if k < 0: sc.err("pub theorem without proof")
                body = body[:k].rstrip() + ";"
            out.append(body)
            continue

        if kw == "abstract":
            m = re.match(r"abstract\s+def\b", text)
            if not m: sc.err("unhandled abstract form")
            k = Scanner.top_eq(text)
            if k < 0: sc.err("abstract def without body")
            out.append(re.sub(r"^abstract\s+", "", text[:k]).rstrip() + ";")
            continue

        if kw in ("prefix", "infixl", "infixr", "notation"):
            m = re.match(r"(?:prefix|infixl|infixr|notation)\s+([A-Za-z0-9_?!+*/<>=|&$%^~#@.]*)", text)
            name = m.group(1) if m else ""
            if name in local_names:
                continue
            if kw == "notation":
                if name in NOTATION_FIX:
                    rest = re.sub(r"^notation\s+\S+\s*(\([^)]*\)\s*)*=", "", text).strip()
                    text = NOTATION_FIX[name] + " " + rest
                else:
                    warnings.append(f"notation {name} passed through untyped: {text[:60]!r}")
            out.append(text)
            continue

        # sort / term / def / axiom / delimiter / theorem (plain)
        if kw == "theorem":
            continue  # plain theorem = LocalThm: no .mm0 statement
        out.append(text)

    hdr = (f"-- Generated by gen_mm0.py from {os.path.basename(sys.argv[1])}: public statements only,\n"
           "-- in .mmb statement-stream order (local thms/defs and lisp omitted).\n")
    open(sys.argv[2], "w").write(hdr + "".join(t.rstrip() + "\n" for t in out))
    kinds = {}
    for t in out:
        k = t.split()[0]
        kinds[k] = kinds.get(k, 0) + 1
    print(f"gen_mm0: {len(out)} statements -> {sys.argv[2]} {kinds}")
    for w in warnings: print(f"  warning: {w}")


if __name__ == "__main__":
    main()
