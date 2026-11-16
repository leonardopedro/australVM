// scripts/toy-witness.rs — run the list witness that `mm0/store_projection_toy.mm0`
// claims, and check every one of its axioms against it.
//
// WHY THIS EXISTS. The toy's header says its axioms are mutually consistent
// because a three-cell list memory with `put` = "cons the pair, drop any pair at
// that address, truncate to three" satisfies all of them. That argument is
// prose, and prose is exactly how a false witness survives review: an earlier
// revision carried `put_notfull` ("a write to a memory with a free cell leaves it
// with a free cell"), which reads fine and is false here, because the write that
// takes the last cell is the one that removes the free cell. So the witness is
// written as code and every axiom is checked, universally quantified, over all
// memories of at most three cells and all atoms from a small domain.
//
// WHAT IT PROVES, PRECISELY, AND IN WHICH DIRECTION. A program cannot quantify
// over an atom type, so this file does not try: it separates the two directions
// and neither of them depends on how many atoms the domain has.
//
//   *Refutations are domain-free.* A candidate rejected here is refuted by a
//   concrete memory over four atoms, and that same memory is a value of `Mem α`
//   for *every* α with at least four atoms once the atoms are injected into α.
//   The step that makes the transport valid — that the model's operations depend
//   on an atom only through `==`, so an injective renaming fixing `unset`
//   commutes with `put`/`get`/`held`/`notfull`/`keep` — is not assumed: the
//   `model_equivariant` row below checks it over every memory and every atom of
//   the domain, against a renaming into a strictly larger one. A refutation is
//   therefore a theorem about every α ⊇ {0..3}, not about a domain.
//
//   *Positive claims are proved elsewhere.* The `[hold]` rows below check the
//   model's *code* against each axiom at this domain — that the list model in
//   this file really is the model the header describes, and that the two
//   independently written artefacts (this one and `mm0/store_projection_toy.mm1`)
//   agree on their counts. Coverage over an arbitrary atom type is not a claim a
//   sweep can make at any size; it is proved, for an arbitrary `α`, in
//   `mm0/toy_witness/ToyWitness.lean`. So the domain size is an input to an
//   exploration, not a premise of anything: nothing here says "the axioms hold",
//   it says "they hold here" — and "here" is exactly where a refutation is
//   decided as well.
//
// Build and run through `scripts/toy-witness`, which compiles this with rustc.

const UNSET: u8 = 0;
// Four atoms by default: `at_drop` needs four *distinct addresses*, so a smaller
// domain makes the eviction axiom vacuous rather than cheap. The number is a
// choice of *instance set*, not a premise: refutations transport to every
// larger domain (see `model_equivariant` below), and the positive direction is
// covered by the Lean proof for an arbitrary atom type. `--domain 5` runs the
// same checks over ~30x the instances and reports the identical verdict.
const ATOMS_DEFAULT: u8 = 4;

/// A three-cell memory: `n` pairs, newest first. `Copy` so the checks below can
/// pass memories by value without allocating in the inner loops.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Mem {
    n: u8,
    cells: [(u8, u8); 3],
}

const INIT: Mem = Mem {
    n: 0,
    cells: [(0, 0), (0, 0), (0, 0)],
};

impl Mem {
    /// `put m a v`: the new pair goes first, any pair already at `a` is dropped,
    /// and the result keeps at most three cells. Dropping the old pair *before*
    /// truncating is what makes a write to a held address a refresh instead of
    /// a duplicate.
    fn put(self, a: u8, v: u8) -> Mem {
        let mut out = [(a, v), (0, 0), (0, 0)];
        let mut n = 1usize;
        let mut i = 0usize;
        while i < self.n as usize {
            let (b, w) = self.cells[i];
            if b != a && n < 3 {
                out[n] = (b, w);
                n += 1;
            }
            i += 1;
        }
        Mem {
            n: n as u8,
            cells: out,
        }
    }

    /// `get m a`: the value of the first pair whose address is `a`, else `unset`.
    fn get(self, a: u8) -> u8 {
        let mut i = 0usize;
        while i < self.n as usize {
            let (b, w) = self.cells[i];
            if b == a {
                return w;
            }
            i += 1;
        }
        UNSET
    }

    /// `held m a`: some pair has address `a`.
    fn held(self, a: u8) -> bool {
        let mut i = 0usize;
        while i < self.n as usize {
            if self.cells[i].0 == a {
                return true;
            }
            i += 1;
        }
        false
    }

    /// `isa_store_w8_keep m a v`: the second width-8 store, with the other
    /// refresh order — a write to a held address keeps the value the cell
    /// already has. The model is the identity, which is exactly what its two
    /// laws say: nothing readable moves, at the written address or anywhere
    /// else. The store being a no-op is the point — a definition with this
    /// ordering cannot make a refresh visible.
    fn keep(self, _a: u8, _v: u8) -> Mem {
        self
    }

    /// `notfull m`: fewer than three cells.
    fn notfull(self) -> bool {
        self.n < 3
    }

    /// The same memory with every atom renamed by `f`. Used by the equivariance
    /// check below: the model's operations consult an atom only through `==` and
    /// store it verbatim, so an injective renaming that fixes `unset` commutes
    /// with all of them.
    fn renamed(self, f: impl Fn(u8) -> u8) -> Mem {
        let mut cells = [(0u8, 0u8); 3];
        for i in 0..self.n as usize {
            cells[i] = (f(self.cells[i].0), f(self.cells[i].1));
        }
        Mem {
            n: self.n,
            cells,
        }
    }

    fn show(self) -> String {
        let mut parts = Vec::new();
        for i in 0..self.n as usize {
            parts.push(format!("({},{})", self.cells[i].0, self.cells[i].1));
        }
        format!("[{}]", parts.join(","))
    }
}

/// `narrow4 x`: a value narrowed to four bytes. Deliberately information-losing
/// (`x % 2`) rather than the identity, so the idempotence axiom is checked
/// against a real truncation and not against a function that does nothing.
fn narrow4(x: u8) -> u8 {
    x % 2
}

struct Row {
    name: &'static str,
    must_hold: bool,
    /// A property of the *model* rather than of the artifact: it has no axiom in
    /// the `.mm0`, so it is reported and counted separately and never appears in
    /// the `[hold]` list the gate compares against the artifact's axioms.
    model: bool,
    instances: u64,
    counterexample: Option<String>,
}

fn all_memories(atoms: u8) -> Vec<Mem> {
    let mut out = vec![INIT];
    for a in 0..atoms {
        for v in 0..atoms {
            out.push(Mem {
                n: 1,
                cells: [(a, v), (0, 0), (0, 0)],
            });
        }
    }
    // Multi-cell memories include the shapes `put` never produces (two pairs at
    // one address, or three pairs with a repeat). They are still values of `Mem`
    // that the axioms quantify over, so a witness that failed on one would be a
    // bad witness — and with three cells there are many more of them.
    for a1 in 0..atoms {
        for v1 in 0..atoms {
            for a2 in 0..atoms {
                for v2 in 0..atoms {
                    out.push(Mem {
                        n: 2,
                        cells: [(a1, v1), (a2, v2), (0, 0)],
                    });
                    for a3 in 0..atoms {
                        for v3 in 0..atoms {
                            out.push(Mem {
                                n: 3,
                                cells: [(a1, v1), (a2, v2), (a3, v3)],
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

fn row(
    name: &'static str,
    must_hold: bool,
    instances: u64,
    counterexample: Option<String>,
) -> Row {
    Row {
        name,
        must_hold,
        model: false,
        instances,
        counterexample,
    }
}

fn model_row(name: &'static str, instances: u64, counterexample: Option<String>) -> Row {
    Row {
        name,
        must_hold: true,
        model: true,
        instances,
        counterexample,
    }
}

fn main() {
    let mut atoms = ATOMS_DEFAULT;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--domain" {
            match args.next().and_then(|n| n.parse::<u8>().ok()) {
                Some(n) if n >= 2 => atoms = n,
                _ => {
                    eprintln!("toy-witness: --domain needs an integer >= 2");
                    std::process::exit(2);
                }
            }
        } else {
            eprintln!("toy-witness: unknown argument {arg}; usage: toy-witness [--domain N]");
            std::process::exit(2);
        }
    }

    let mems = all_memories(atoms);
    let atoms_v: Vec<u8> = (0..atoms).collect();
    println!(
        "toy-witness: {} atom(s) (0 = unset), {} memories (every shape of at most three cells)",
        atoms,
        mems.len()
    );

    let mut rows: Vec<Row> = Vec::new();

    // veq_refl (x): veq x x. With `veq` = equality this is reflexive.
    {
        let mut ce = None;
        let mut n = 0u64;
        for &x in &atoms_v {
            n += 1;
            if !(x == x) && ce.is_none() {
                ce = Some(format!("x={x}"));
            }
        }
        rows.push(row("veq_refl", true, n, ce));
    }

    // veq_trans (x y z) (h1: veq x y) (h2: veq y z): veq x z.
    {
        let mut ce = None;
        let mut n = 0u64;
        for &x in &atoms_v {
            for &y in &atoms_v {
                for &z in &atoms_v {
                    n += 1;
                    if x == y && y == z && !(x == z) && ce.is_none() {
                        ce = Some(format!("x={x} y={y} z={z}"));
                    }
                }
            }
        }
        rows.push(row("veq_trans", true, n, ce));
    }

    // vne_sym (x y) (h: vne x y): vne y x.
    {
        let mut ce = None;
        let mut n = 0u64;
        for &x in &atoms_v {
            for &y in &atoms_v {
                n += 1;
                if x != y && !(y != x) && ce.is_none() {
                    ce = Some(format!("x={x} y={y}"));
                }
            }
        }
        rows.push(row("vne_sym", true, n, ce));
    }

    // narrow4_cong (x y) (veq x y): veq (narrow4 x) (narrow4 y).
    {
        let mut ce = None;
        let mut n = 0u64;
        for &x in &atoms_v {
            for &y in &atoms_v {
                n += 1;
                if x == y && narrow4(x) != narrow4(y) && ce.is_none() {
                    ce = Some(format!("x={x} y={y} -> {} vs {}", narrow4(x), narrow4(y)));
                }
            }
        }
        rows.push(row("narrow4_cong", true, n, ce));
    }

    // narrow4_idem (x): veq (narrow4 (narrow4 x)) (narrow4 x).
    {
        let mut ce = None;
        let mut n = 0u64;
        for &x in &atoms_v {
            n += 1;
            if narrow4(narrow4(x)) != narrow4(x) && ce.is_none() {
                ce = Some(format!(
                    "x={x} -> narrow4(narrow4 x)={} narrow4 x={}",
                    narrow4(narrow4(x)),
                    narrow4(x)
                ));
            }
        }
        rows.push(row("narrow4_idem", true, n, ce));
    }

    // get_init (a): get init a = unset.
    {
        let mut ce = None;
        let mut n = 0u64;
        for &a in &atoms_v {
            n += 1;
            if INIT.get(a) != UNSET && ce.is_none() {
                ce = Some(format!("a={a} -> get={}", INIT.get(a)));
            }
        }
        rows.push(row("get_init", true, n, ce));
    }

    // init_notfull: notfull init.
    rows.push(row(
        "init_notfull",
        true,
        1,
        if INIT.notfull() {
            None
        } else {
            Some("init is full".to_string())
        },
    ));

    // put_init_notfull (a v): notfull (put init a v).
    {
        let mut ce = None;
        let mut n = 0u64;
        for &a in &atoms_v {
            for &v in &atoms_v {
                n += 1;
                let m = INIT.put(a, v);
                if !m.notfull() && ce.is_none() {
                    ce = Some(format!("a={a} v={v} -> put={} has no free cell", m.show()));
                }
            }
        }
        rows.push(row("put_init_notfull", true, n, ce));
    }

    // put2_init_notfull (a b va vb): notfull (put (put init a va) b vb).
    //
    // The counting step the third store needs, and the statement that would be
    // false at capacity two: it is what makes `toy_jit_third_store_kept` provable
    // instead of the old `third_store_drops_oldest`.
    {
        let mut ce = None;
        let mut n = 0u64;
        for &a in &atoms_v {
            for &b in &atoms_v {
                for &va in &atoms_v {
                    for &vb in &atoms_v {
                        n += 1;
                        let m = INIT.put(a, va).put(b, vb);
                        if !m.notfull() && ce.is_none() {
                            ce = Some(format!(
                                "a={a} b={b} -> two stores left {} with no free cell",
                                m.show()
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("put2_init_notfull", true, n, ce));
    }

    // at_put (m) (a v): get (put m a v) a = v.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &a in &atoms_v {
                for &v in &atoms_v {
                    n += 1;
                    let p = m.put(a, v);
                    if p.get(a) != v && ce.is_none() {
                        ce = Some(format!("m={} a={a} v={v} -> get={}", m.show(), p.get(a)));
                    }
                }
            }
        }
        rows.push(row("at_put", true, n, ce));
    }

    // put_held (m) (a v): held (put m a v) a.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &a in &atoms_v {
                for &v in &atoms_v {
                    n += 1;
                    let p = m.put(a, v);
                    if !p.held(a) && ce.is_none() {
                        ce = Some(format!("m={} a={a} v={v} -> put={}", m.show(), p.show()));
                    }
                }
            }
        }
        rows.push(row("put_held", true, n, ce));
    }

    // put_held_other (m) (a b v) (vne a b) (held m b) (notfull m): held (put m a v) b.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            if !m.notfull() {
                continue;
            }
            for &a in &atoms_v {
                for &b in &atoms_v {
                    if a == b || !m.held(b) {
                        continue;
                    }
                    for &v in &atoms_v {
                        n += 1;
                        let p = m.put(a, v);
                        if !p.held(b) && ce.is_none() {
                            ce = Some(format!(
                                "m={} a={a} b={b} v={v} -> put={} no longer holds {b}",
                                m.show(),
                                p.show()
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("put_held_other", true, n, ce));
    }

    // at_put_held_other (m) (x y w) (vne x y) (held m x): get (put m x w) y = get m y.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                if !m.held(x) {
                    continue;
                }
                for &y in &atoms_v {
                    if x == y {
                        continue;
                    }
                    for &w in &atoms_v {
                        n += 1;
                        let p = m.put(x, w);
                        if p.get(y) != m.get(y) && ce.is_none() {
                            ce = Some(format!(
                                "m={} x={x} y={y} w={w} -> get {y} was {} now {}",
                                m.show(),
                                m.get(y),
                                p.get(y)
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("at_put_held_other", true, n, ce));
    }

    // at_put_held_keeps (m) (x y w) (vne x y) (held m y) (held m x): held (put m x w) y.
    //
    // The two held-set facts are what carries "a refresh never evicts" in a toy
    // with no arithmetic: instead of a count, the two directions a size could
    // move in are each a membership statement.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                if !m.held(x) {
                    continue;
                }
                for &y in &atoms_v {
                    if x == y || !m.held(y) {
                        continue;
                    }
                    for &w in &atoms_v {
                        n += 1;
                        let p = m.put(x, w);
                        if !p.held(y) && ce.is_none() {
                            ce = Some(format!(
                                "m={} x={x} y={y} w={w} -> put={} lost the cell at {y}",
                                m.show(),
                                p.show()
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("at_put_held_keeps", true, n, ce));
    }

    // at_put_held_no_new (m) (x y w) (vne x y) (held (put m x w) y): held m y.
    //
    // No `held m x` premise, deliberately: `put` introduces no address but the
    // one it writes, so the statement is true of any write, and the guard a first
    // draft carried here was rejected by this checker as unnecessary.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                for &y in &atoms_v {
                    if x == y {
                        continue;
                    }
                    for &w in &atoms_v {
                        let p = m.put(x, w);
                        if !p.held(y) {
                            continue;
                        }
                        n += 1;
                        if !m.held(y) && ce.is_none() {
                            ce = Some(format!(
                                "m={} x={x} y={y} w={w} -> put={} holds {y} but m did not",
                                m.show(),
                                p.show()
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("at_put_held_no_new", true, n, ce));
    }

    // at_put_other (m) (a b v) (vne a b) (notfull m): get (put m a v) b = get m b.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            if !m.notfull() {
                continue;
            }
            for &a in &atoms_v {
                for &b in &atoms_v {
                    if a == b {
                        continue;
                    }
                    for &v in &atoms_v {
                        n += 1;
                        let p = m.put(a, v);
                        if p.get(b) != m.get(b) && ce.is_none() {
                            ce = Some(format!(
                                "m={} a={a} b={b} v={v} -> get {b} was {} now {}",
                                m.show(),
                                m.get(b),
                                p.get(b)
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("at_put_other", true, n, ce));
    }

    // at_drop (m) (x y z w) (vx vy vz vw) (six vne premises):
    // get (put (put (put (put m x vx) y vy) z vz) w vw) x = unset.
    //
    // Four distinct stores, because the capacity is three: with a third cell the
    // first eviction lands one store later than it used to, and the rejected
    // candidate at the bottom of this file is the three-store version, which must
    // now fail.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                for &y in &atoms_v {
                    if y == x {
                        continue;
                    }
                    for &z in &atoms_v {
                        if z == x || z == y {
                            continue;
                        }
                        for &w in &atoms_v {
                            if w == x || w == y || w == z {
                                continue;
                            }
                            for &vx in &atoms_v {
                                for &vy in &atoms_v {
                                    for &vz in &atoms_v {
                                        for &vw in &atoms_v {
                                            n += 1;
                                            let p = m
                                                .put(x, vx)
                                                .put(y, vy)
                                                .put(z, vz)
                                                .put(w, vw);
                                            if p.get(x) != UNSET && ce.is_none() {
                                                ce = Some(format!(
                                                    "m={} x={x} y={y} z={z} w={w} -> get x={} (not unset)",
                                                    m.show(),
                                                    p.get(x)
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        rows.push(row("at_drop", true, n, ce));
    }

    // at_keep_prev (m) (x y z w) (vx vy vz vw) (six vne premises):
    // get (put (put (put (put m x vx) y vy) z vz) w vw) y = vy.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                for &y in &atoms_v {
                    if y == x {
                        continue;
                    }
                    for &z in &atoms_v {
                        if z == x || z == y {
                            continue;
                        }
                        for &w in &atoms_v {
                            if w == x || w == y || w == z {
                                continue;
                            }
                            for &vx in &atoms_v {
                                for &vy in &atoms_v {
                                    for &vz in &atoms_v {
                                        for &vw in &atoms_v {
                                            n += 1;
                                            let p = m
                                                .put(x, vx)
                                                .put(y, vy)
                                                .put(z, vz)
                                                .put(w, vw);
                                            if p.get(y) != vy && ce.is_none() {
                                                ce = Some(format!(
                                                    "m={} x={x} y={y} z={z} w={w} vy={vy} -> get y={}",
                                                    m.show(),
                                                    p.get(y)
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        rows.push(row("at_keep_prev", true, n, ce));
    }

    // keep_at_put_held (m) (x w) (held m x): get (keep m x w) x = get m x.
    //
    // The second width-8 store's first law: a refresh under the other ordering
    // keeps the value the cell already had. The model is the identity, so this
    // is the statement that the ordering is observable only in what it does
    // *not* do — and the rejected candidate below is the same statement with the
    // new value on the right, which must fail.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                if !m.held(x) {
                    continue;
                }
                for &w in &atoms_v {
                    n += 1;
                    let k = m.keep(x, w);
                    if k.get(x) != m.get(x) && ce.is_none() {
                        ce = Some(format!(
                            "m={} x={x} w={w} -> keep={} read {x} as {} not {}",
                            m.show(),
                            k.show(),
                            k.get(x),
                            m.get(x)
                        ));
                    }
                }
            }
        }
        rows.push(row("keep_at_put_held", true, n, ce));
    }

    // keep_at_put_other (m) (x y w) (vne x y): get (keep m x w) y = get m y.
    //
    // The second law: no other address is disturbed. Unlike `put`, this needs no
    // `notfull` premise, because a store that keeps the old value adds no cell
    // and so can never evict.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                for &y in &atoms_v {
                    if x == y {
                        continue;
                    }
                    for &w in &atoms_v {
                        n += 1;
                        let k = m.keep(x, w);
                        if k.get(y) != m.get(y) && ce.is_none() {
                            ce = Some(format!(
                                "m={} x={x} y={y} w={w} -> get {y} was {} now {}",
                                m.show(),
                                m.get(y),
                                k.get(y)
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("keep_at_put_other", true, n, ce));
    }

    // ── Candidates the toy considered and rejected ──────────────────────────
    // Each of these *must* fail here; a candidate that holds would mean the
    // guard it was given in the `.mm0` is unnecessary, and the rejection was
    // wrong rather than the candidate.
    //
    // The `put_notfull` closure an earlier revision carried.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            if !m.notfull() {
                continue;
            }
            for &a in &atoms_v {
                for &v in &atoms_v {
                    n += 1;
                    let p = m.put(a, v);
                    if !p.notfull() && ce.is_none() {
                        ce = Some(format!(
                            "m={} a={a} v={v} -> put={} has no free cell",
                            m.show(),
                            p.show()
                        ));
                    }
                }
            }
        }
        rows.push(row("put_notfull", false, n, ce));
    }

    // `at_put_other` without its `notfull` guard.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &a in &atoms_v {
                for &b in &atoms_v {
                    if a == b {
                        continue;
                    }
                    for &v in &atoms_v {
                        n += 1;
                        let p = m.put(a, v);
                        if p.get(b) != m.get(b) && ce.is_none() {
                            ce = Some(format!(
                                "m={} a={a} b={b} v={v} -> get {b} was {} now {}",
                                m.show(),
                                m.get(b),
                                p.get(b)
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("at_put_other_unguarded", false, n, ce));
    }

    // `put_held_other` without its `notfull` guard.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &a in &atoms_v {
                for &b in &atoms_v {
                    if a == b || !m.held(b) {
                        continue;
                    }
                    for &v in &atoms_v {
                        n += 1;
                        let p = m.put(a, v);
                        if !p.held(b) && ce.is_none() {
                            ce = Some(format!(
                                "m={} a={a} b={b} v={v} -> put={} no longer holds {b}",
                                m.show(),
                                p.show()
                            ));
                        }
                    }
                }
            }
        }
        rows.push(row("put_held_other_unguarded", false, n, ce));
    }

    // The eviction axiom this file carried while the memory had two cells: after
    // *three* distinct stores the earliest address reads `unset`. With a third
    // cell it is false, and it has to be — that is the whole point of the third
    // cell, and a witness that still accepted it would not be modelling the
    // memory the `.mm0` now describes.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &x in &atoms_v {
                for &y in &atoms_v {
                    if y == x {
                        continue;
                    }
                    for &z in &atoms_v {
                        if z == x || z == y {
                            continue;
                        }
                        for &vx in &atoms_v {
                            for &vy in &atoms_v {
                                for &vz in &atoms_v {
                                    n += 1;
                                    let p = m.put(x, vx).put(y, vy).put(z, vz);
                                    if p.get(x) != UNSET && ce.is_none() {
                                        ce = Some(format!(
                                            "m={} x={x} y={y} z={z} -> get x={} (three stores did not evict)",
                                            m.show(),
                                            p.get(x)
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        rows.push(row("at_drop_at_two_cells", false, n, ce));
    }

    // "The second width-8 store is visible at the address it wrote" — the
    // projection instance the toy's other four theorems would need. It has to
    // fail: under the other refresh order a write leaves the old value there,
    // which is precisely where the projection breaks, and a checker that did not
    // reject this would be certifying a definition it never examined.
    {
        let mut ce = None;
        let mut n = 0u64;
        for m in &mems {
            for &a in &atoms_v {
                for &v in &atoms_v {
                    n += 1;
                    let k = m.keep(a, v);
                    if k.get(a) != v && ce.is_none() {
                        ce = Some(format!(
                            "m={} a={a} v={v} -> keep={} reads {a} as {}, not {v}",
                            m.show(),
                            k.show(),
                            k.get(a)
                        ));
                    }
                }
            }
        }
        rows.push(row("keep_store_visible", false, n, ce));
    }

    // "Narrowing to four bytes is the identity" — the discarded alternative that
    // would make the width-4 theorems vacuous. It has to fail, otherwise the
    // idempotence axiom says nothing about losing information.
    {
        let mut ce = None;
        let mut n = 0u64;
        for &x in &atoms_v {
            n += 1;
            if narrow4(x) != x && ce.is_none() {
                ce = Some(format!("x={x} -> narrow4 x={}", narrow4(x)));
            }
        }
        rows.push(row("narrow4_is_identity", false, n, ce));
    }

    // ── Why a refutation at this domain is a refutation everywhere ──────────
    //
    // `atoms` is not a premise anywhere above: a `[reject]` row's counterexample
    // is a memory over atoms `0..atoms`, and that memory embeds in every larger
    // atom type. What the transport needs is that renaming the atoms does not
    // change any result — i.e. that the model reads an atom only by comparing it
    // for equality and otherwise copies it. That is a property of this code, so
    // it is checked here rather than asserted: rename every atom by an injection
    // into a strictly larger domain (`unset` fixed, since it is a constant and
    // not a variable), and require `put`, `get`, `held`, `notfull` and `keep` to
    // commute with it, over every memory and every atom of the domain.
    {
        // Two injections, one order-preserving and one not: a model that compared
        // atoms by anything other than equality (parity, ordering, a hash) could
        // survive a single monotone renaming by accident, which is the kind of
        // control that passes for the wrong reason. Both are built from the domain
        // size rather than written out, because a fixed table is only correct for
        // one `--domain` — the first version of this row indexed a four-entry table
        // and panicked on `--domain 5`.
        let base = atoms;
        let shift = move |s: u8| -> u8 {
            if s == UNSET {
                UNSET
            } else {
                s.wrapping_add(base)
            }
        };
        let swap = move |s: u8| -> u8 {
            match s {
                0 => 0,
                1 => 2u8.wrapping_add(base),
                2 => 1u8.wrapping_add(base),
                t => t.wrapping_add(base),
            }
        };
        let mut ce = None;
        let mut n = 0u64;
        for (k, embed) in [&shift as &dyn Fn(u8) -> u8, &swap].iter().enumerate() {
            for m in &mems {
                let rm = m.renamed(embed);
                n += 1;
                if rm.n != m.n && ce.is_none() {
                    ce = Some(format!("m={} -> renaming {k} changed the cell count", m.show()));
                }
                n += 1;
                if rm.notfull() != m.notfull() && ce.is_none() {
                    ce = Some(format!("m={} -> renaming {k} changed notfull", m.show()));
                }
                for &a in &atoms_v {
                    n += 1;
                    if rm.get(embed(a)) != embed(m.get(a)) && ce.is_none() {
                        ce = Some(format!(
                            "m={} a={a} -> renaming {k}: get {} , renamed get {}",
                            m.show(),
                            rm.get(embed(a)),
                            embed(m.get(a))
                        ));
                    }
                    n += 1;
                    if rm.held(embed(a)) != m.held(a) && ce.is_none() {
                        ce = Some(format!("m={} a={a} -> renaming {k} changed held", m.show()));
                    }
                    for &v in &atoms_v {
                        n += 1;
                        if rm.put(embed(a), embed(v)) != m.put(a, v).renamed(embed)
                            && ce.is_none()
                        {
                            ce = Some(format!(
                                "m={} a={a} v={v} -> renaming {k}: put {} , renamed put {}",
                                m.show(),
                                rm.put(embed(a), embed(v)).show(),
                                m.put(a, v).renamed(embed).show()
                            ));
                        }
                        n += 1;
                        if rm.keep(embed(a), embed(v)) != m.keep(a, v).renamed(embed)
                            && ce.is_none()
                        {
                            ce = Some(format!(
                                "m={} a={a} v={v} -> renaming {k} changed keep",
                                m.show()
                            ));
                        }
                    }
                }
            }
        }
        rows.push(model_row("model_equivariant", n, ce));
    }

    // ── Report ──────────────────────────────────────────────────────────────
    let mut bad = 0usize;
    let mut held = 0usize;
    let mut rejected = 0usize;
    let mut modelled = 0usize;
    for r in &rows {
        let ok = if r.must_hold || r.model {
            r.counterexample.is_none()
        } else {
            r.counterexample.is_some()
        };
        if ok {
            if r.model {
                modelled += 1;
                println!("[model]  {:<26} {:>10} instance(s)", r.name, r.instances);
            } else if r.must_hold {
                held += 1;
                println!("[hold]   {:<26} {:>10} instance(s)", r.name, r.instances);
            } else {
                rejected += 1;
                println!(
                    "[reject] {:<26} {:>10} instance(s) — {}",
                    r.name,
                    r.instances,
                    r.counterexample.as_deref().unwrap_or("")
                );
            }
        } else {
            bad += 1;
            match (&r.counterexample, r.must_hold) {
                (Some(ce), true) => {
                    println!("[FAIL]   {:<26} counterexample: {ce}", r.name);
                }
                (_, false) => {
                    println!("[FAIL]   {:<26} holds in the witness, so the .mm0 rejects it for no reason", r.name);
                }
                _ => {}
            }
        }
    }

    let axioms = rows.iter().filter(|r| r.must_hold && !r.model).count();
    let candidates = rows.iter().filter(|r| !r.must_hold).count();
    let _ = modelled;
    if bad > 0 {
        println!(
            "toy-witness: {held}/{axioms} axiom(s) hold, {rejected}/{candidates} rejected candidate(s) fail — {bad} wrong"
        );
        std::process::exit(1);
    }
    // Stated plainly, because the numbers above sit next to a domain size: the
    // `[reject]` rows are about every larger atom domain, the `[hold]` rows are
    // about this one, and the arbitrary-domain proof is a Lean file. Printed
    // *before* the summary line, so the line the gate quotes stays the summary.
    println!(
        "toy-witness: {rejected}/{candidates} refutation(s) transport to every larger atom domain (model commutes with injective renaming); the axioms are proved for an arbitrary atom type in mm0/toy_witness/ToyWitness.lean"
    );
    println!(
        "toy-witness: {held}/{axioms} axioms hold in the witness; {rejected}/{candidates} rejected candidates fail as they must"
    );
}
