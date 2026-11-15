// scripts/toy-witness.rs — run the list witness that `mm0/store_projection_toy.mm0`
// claims, and check every one of its axioms against it.
//
// WHY THIS EXISTS. The toy's header says its axioms are mutually consistent
// because a two-cell list memory with `put` = "cons the pair, drop any pair at
// that address, truncate to two" satisfies all of them. That argument is prose,
// and prose is exactly how a false witness survives review: an earlier revision
// carried `put_notfull` ("a write to a memory with a free cell leaves it with a
// free cell"), which reads fine and is false here, because the write that takes
// the second cell is the one that removes the free cell. So the witness is
// written as code and every axiom is checked, universally quantified, over all
// memories of at most two cells and all atoms from a small domain.
//
// WHAT IT PROVES, PRECISELY. It is exhaustive over this finite domain, not a
// proof over an arbitrary `Val`: an axiom instance that needs more atoms than
// the domain has is the same equality pattern with atoms renamed, which is an
// argument, not something this file establishes. What it does establish is that
// the witness in the header is executed rather than asserted, and that the three
// axioms that were considered and rejected are rejected *for a reason* — each
// comes with the counterexample that kills it.
//
// Build and run through `scripts/toy-witness`, which compiles this with rustc.

const UNSET: u8 = 0;
const ATOMS_DEFAULT: u8 = 5;

/// A two-cell memory: `n` pairs, newest first. `Copy` so the checks below can
/// pass memories by value without allocating in the inner loops.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Mem {
    n: u8,
    cells: [(u8, u8); 2],
}

const INIT: Mem = Mem {
    n: 0,
    cells: [(0, 0), (0, 0)],
};

impl Mem {
    /// `put m a v`: the new pair goes first, any pair already at `a` is dropped,
    /// and the result keeps at most two cells. Dropping the old pair *before*
    /// truncating is what makes a write to a held address a refresh instead of
    /// a duplicate.
    fn put(self, a: u8, v: u8) -> Mem {
        let mut out = [(a, v), (0, 0)];
        let mut n = 1usize;
        let mut i = 0usize;
        while i < self.n as usize {
            let (b, w) = self.cells[i];
            if b != a && n < 2 {
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

    /// `narrow4 x`: a value narrowed to four bytes. Chosen to *lose* information
/// (`x % 2`), not as the identity, so that the idempotence axiom is checked
/// against a real truncation rather than against a function that does nothing.
fn narrow4(x: u8) -> u8 {
    x % 2
}    /// `isa_store_w8_keep m a v`: the second width-8 store, with the other
    /// refresh order — a write to a held address keeps the value the cell
    /// already has. The model is the identity, which is exactly what its two
    /// laws say: nothing readable moves, at the written address or anywhere
    /// else. The store being a no-op is the point — a definition with this
    /// ordering cannot make a refresh visible.
    fn keep(self, _a: u8, _v: u8) -> Mem {
        self
    }

    /// `notfull m`: fewer than two cells.
    fn notfull(self) -> bool {
        self.n < 2
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
    instances: u64,
    counterexample: Option<String>,
}

fn all_memories(atoms: u8) -> Vec<Mem> {
    let mut out = vec![INIT];
    for a in 0..atoms {
        for v in 0..atoms {
            out.push(Mem {
                n: 1,
                cells: [(a, v), (0, 0)],
            });
        }
    }
    // Two-cell memories include the shapes `put` never produces (both pairs at
    // one address). They are still values of `Mem` that the axioms quantify
    // over, so a witness that failed on one would be a bad witness.
    for a1 in 0..atoms {
        for v1 in 0..atoms {
            for a2 in 0..atoms {
                for v2 in 0..atoms {
                    out.push(Mem {
                        n: 2,
                        cells: [(a1, v1), (a2, v2)],
                    });
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
        "toy-witness: {} atom(s) (0 = unset), {} memories (every shape of at most two cells)",
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

    // at_drop (m) (x y z) (vx vy vz) (vne x y) (vne x z) (vne y z):
    // get (put (put (put m x vx) y vy) z vz) x = unset.
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
                                            "m={} x={x} y={y} z={z} -> get x={} (not unset)",
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
        rows.push(row("at_drop", true, n, ce));
    }

    // at_keep_prev (m) (x y z) (vx vy vz) (vne x y) (vne x z) (vne y z):
    // get (put (put (put m x vx) y vy) z vz) y = vy.
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
                                    if p.get(y) != vy && ce.is_none() {
                                        ce = Some(format!(
                                            "m={} x={x} y={y} z={z} vy={vy} -> get y={}",
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

    // ── Report ──────────────────────────────────────────────────────────────
    let mut bad = 0usize;
    let mut held = 0usize;
    let mut rejected = 0usize;
    for r in &rows {
        let ok = if r.must_hold {
            r.counterexample.is_none()
        } else {
            r.counterexample.is_some()
        };
        if ok {
            if r.must_hold {
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

    let axioms = rows.iter().filter(|r| r.must_hold).count();
    let candidates = rows.iter().filter(|r| !r.must_hold).count();
    if bad > 0 {
        println!(
            "toy-witness: {held}/{axioms} axiom(s) hold, {rejected}/{candidates} rejected candidate(s) fail — {bad} wrong"
        );
        std::process::exit(1);
    }
    println!(
        "toy-witness: {held}/{axioms} axioms hold in the witness; {rejected}/{candidates} rejected candidates fail as they must"
    );
}
