//! A prototype, not a patch: what a hash-consed node arena costs per node on a
//! real MM1 artifact, measured against the representation the elaborator builds.
//!
//! The question comes from `australVM/docs/lean4mm0.md` §13.22: elaborating
//! these artifacts needs 385 B per node of proof text to stay under a 12 GiB
//! arena, and a shared representation is supposed to be far cheaper. Cheap by
//! how much is measurable on the artifact itself.
//!
//! What a node is here matters. Counting `(` in the text counts nodes inside
//! `$ … $` formulas: the lisp parser does not expand those (a `Formula` is a
//! span), so the tree that grows during elaboration is the math expression
//! `QExpr` tree the elaborator builds per formula. This prototype builds the
//! same trees from the same spans, first in the tree shape the elaborator uses
//! (a `span` + a kind + a boxed child slice per node), then interned into an
//! arena keyed by kind + child ids. A counting global allocator reports the
//! heap each one actually holds, so the numbers are measurements, not
//! `size_of` arithmetic.
//!
//! ```text
//! cargo run --release --bin arena_proto -- <file.mm1>
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

use mm0_rs::ast::{SExpr, SExprKind, StmtKind, Type};
use mm0_rs::{LinedString, Span};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
  unsafe fn alloc(&self, l: Layout) -> *mut u8 {
    let p = System.alloc(l);
    if !p.is_null() {
      let n = LIVE.fetch_add(l.size(), Relaxed) + l.size();
      PEAK.fetch_max(n, Relaxed);
    }
    p
  }
  unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
    LIVE.fetch_sub(l.size(), Relaxed);
    System.dealloc(p, l)
  }
  unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
    let q = System.realloc(p, l, n);
    if !q.is_null() {
      let live = LIVE.fetch_add(n.wrapping_sub(l.size()), Relaxed);
      PEAK.fetch_max(live, Relaxed);
    }
    q
  }
}

#[global_allocator]
static A: Counting = Counting;

fn live() -> usize { LIVE.load(Relaxed) }
fn peak() -> usize { PEAK.load(Relaxed) }

/// One math-expression node in the shape `QExpr` has: the span it was parsed
/// from, a kind tag (a term/identifier index), and its children.
struct TreeNode {
  span: Span,
  kind: u32,
  kids: Box<[TreeNode]>,
}

impl TreeNode {
  fn nodes(&self) -> usize {
    1 + self.kids.iter().map(TreeNode::nodes).sum::<usize>()
  }
}

/// One token of a `$ … $` formula: a parenthesis, or a maximal run of
/// non-space bytes (an atom, which is a leaf node).
enum Tok {
  Open(usize),
  Close(usize),
  Atom(usize, usize),
}

struct Tokens<'a> {
  s: &'a [u8],
  i: usize,
}

impl<'a> Tokens<'a> {
  fn new(s: &'a [u8]) -> Self { Self { s, i: 0 } }
  fn next(&mut self) -> Option<Tok> {
    while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
      self.i += 1;
    }
    let at = self.i;
    match *self.s.get(self.i)? {
      b'(' => {
        self.i += 1;
        Some(Tok::Open(at))
      }
      b')' => {
        self.i += 1;
        Some(Tok::Close(at))
      }
      _ => {
        while self.i < self.s.len()
          && !self.s[self.i].is_ascii_whitespace()
          && !matches!(self.s[self.i], b'(' | b')')
        {
          self.i += 1;
        }
        Some(Tok::Atom(at, self.i))
      }
    }
  }
}

/// A tree node's kind is 0 for an application, 1 for an atom; the id of the
/// atom (its text span) is what interning must distinguish, so atoms carry a
/// hash of their text in the arena.
fn span_hash(src: &LinedString, s: Span) -> u32 {
  let mut h: u32 = 2_166_136_261;
  for &b in &src.as_bytes()[s.start..s.end] {
    h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
  }
  h
}

/// Build the formula trees for every span, dropping them at the end: this is
/// the representation the elaborator holds while it elaborates one statement.
fn tree_pass(src: &Arc<LinedString>, spans: &[Span]) -> (usize, usize, usize) {
  let before = live();
  let mut total = 0usize;
  // All of them at once: the cost of the representation is what it holds while
  // more than one statement is live, which is the whole file for a compile.
  let mut held_trees: Vec<TreeNode> = Vec::with_capacity(spans.len());
  let mut biggest = 0usize;
  for &sp in spans {
    let t = parse_tree(src, sp);
    let n = t.nodes();
    total += n;
    biggest = biggest.max(n);
    held_trees.push(t);
  }
  let held = live().saturating_sub(before);
  drop(held_trees);
  (total, held, biggest)
}

fn parse_tree(src: &Arc<LinedString>, sp: Span) -> TreeNode {
  let text = &src.as_bytes()[sp.start..sp.end];
  let mut toks = Tokens::new(text);
  let mut stack: Vec<(Span, Vec<TreeNode>)> = Vec::new();
  let mut roots: Vec<TreeNode> = Vec::new();
  while let Some(t) = toks.next() {
    match t {
      Tok::Open(i) => stack.push((Span { start: sp.start + i, end: sp.start + i + 1 }, Vec::new())),
      Tok::Close(i) => {
        let (open, kids) = stack.pop().expect("unbalanced `)`");
        let node = group((open.start..sp.start + i + 1).into(), kids);
        match stack.last_mut() {
          Some((_, ks)) => ks.push(node),
          None => roots.push(node),
        }
      }
      Tok::Atom(a, b) => {
        let at: Span = (sp.start + a..sp.start + b).into();
        let node = TreeNode { span: at, kind: 1 + span_hash(src, at), kids: Box::new([]) };
        match stack.last_mut() {
          Some((_, ks)) => ks.push(node),
          None => roots.push(node),
        }
      }
    }
  }
  assert!(stack.is_empty(), "unbalanced formula");
  group(sp, roots)
}

/// A parenthesised group is one application node; a level with a single item is
/// that item, because MM0 writes a formula's own application by juxtaposition
/// (`$ tty ctx e ty $` is one node over four atoms, not four roots).
fn group(span: Span, mut kids: Vec<TreeNode>) -> TreeNode {
  if kids.len() == 1 {
    return kids.pop().unwrap();
  }
  TreeNode { span, kind: 0, kids: kids.into() }
}

/// The interned form of `group`: a single child is itself, several children are
/// one application node over them, and an empty group is the empty application.
fn group_ids(arena: &mut Arena, kids: &[u32]) -> u32 {
  if kids.len() == 1 { kids[0] } else { arena.intern(0, kids) }
}

/// One interned node: `kind`, a slice into the shared child pool, and the hash
/// chain link for collisions.
#[derive(Clone, Copy)]
struct Node {
  kind: u32,
  arity: u32,
  args: u32,
  next: u32,
}

struct Arena {
  nodes: Vec<Node>,
  pool: Vec<u32>,
  map: HashMap<u64, u32>,
}

impl Arena {
  fn new() -> Self { Self { nodes: Vec::new(), pool: Vec::new(), map: HashMap::new() } }

  fn equal(&self, id: u32, kind: u32, args: &[u32]) -> bool {
    let n = &self.nodes[id as usize];
    n.kind == kind
      && n.arity as usize == args.len()
      && self.pool[n.args as usize..n.args as usize + args.len()] == *args
  }

  fn intern(&mut self, kind: u32, args: &[u32]) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ u64::from(kind);
    for &a in args {
      h ^= u64::from(a);
      h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if let Some(&id) = self.map.get(&h) {
      if self.equal(id, kind, args) {
        return id;
      }
    }
    let id = self.nodes.len() as u32;
    let args_at = self.pool.len() as u32;
    self.pool.extend_from_slice(args);
    self.nodes.push(Node { kind, arity: args.len() as u32, args: args_at, next: 0 });
    self.map.insert(h, id);
    id
  }
}

/// Intern every formula in one pass, without ever holding a tree: interning is
/// bottom-up, so the parse stack is the only temporary.
fn intern_pass(src: &Arc<LinedString>, spans: &[Span]) -> (usize, usize, usize) {
  let before = live();
  let mut arena = Arena::new();
  let mut total = 0usize;
  let mut scratch: Vec<u32> = Vec::new();
  for &sp in spans {
    let text = &src.as_bytes()[sp.start..sp.end];
    let mut toks = Tokens::new(text);
    let mut stack: Vec<(Span, usize, Vec<u32>)> = Vec::new();
    let mut roots: Vec<u32> = Vec::new();
    while let Some(t) = toks.next() {
      match t {
        Tok::Open(i) => {
          total += 1;
          stack.push((Span { start: sp.start + i, end: sp.start + i + 1 }, scratch.len(), Vec::new()));
        }
        Tok::Close(_) => {
          let (_open, _base, kids) = stack.pop().expect("unbalanced `)`");
          let id = group_ids(&mut arena, &kids);
          match stack.last_mut() {
            Some((_, _, ks)) => ks.push(id),
            None => roots.push(id),
          }
        }
        Tok::Atom(a, b) => {
          total += 1;
          let kind = 1 + span_hash(src, (sp.start + a..sp.start + b).into());
          let id = arena.intern(kind, &[]);
          match stack.last_mut() {
            Some((_, _, ks)) => ks.push(id),
            None => roots.push(id),
          }
        }
      }
    }
    assert!(stack.is_empty(), "unbalanced formula");
    if roots.len() > 1 {
      total += 1;
    }
    group_ids(&mut arena, &roots);
  }
  scratch.clear();
  (total, arena.nodes.len(), live().saturating_sub(before))
}

fn count_sexpr(e: &SExpr) -> usize {
  1 + match &e.k {
    SExprKind::List(xs) => xs.iter().map(count_sexpr).sum(),
    SExprKind::DottedList(xs, t) => xs.iter().map(count_sexpr).sum::<usize>() + count_sexpr(t),
    SExprKind::DocComment(_, i) => count_sexpr(i),
    _ => 0,
  }
}

fn main() {
  let mut args = env::args().skip(1);
  let path = args.next().expect("usage: arena_proto <file.mm1> [stride]");
  // A stride samples every k-th formula for the passes that hold a tree. The
  // per-node numbers are scale-free, and a 800 MB artifact's whole tree does not
  // fit in the box that is supposed to be measuring it.
  let stride: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
  let text = fs::read_to_string(&path).expect("read");
  let bytes = text.len();
  let src: Arc<LinedString> = Arc::new(text.into());

  let before = live();
  let (errors, ast) = mm1_parser::parse(src.clone(), None);
  let parse_live = live().saturating_sub(before);

  // Every `$ … $` in the file: the decl's return type, its binders' types, and
  // any formula that appears inside an s-expression.
  let mut spans: Vec<Span> = Vec::new();
  let mut lisp_nodes = 0usize;
  for st in &ast.stmts {
    if let StmtKind::Decl(d) = &st.k {
      for b in &d.bis {
        if let Some(Type::Formula(f)) = &b.ty {
          spans.push(f.0);
        }
      }
      if let Some(Type::Formula(f)) = &d.ty {
        spans.push(f.0);
      }
      if let Some(v) = &d.val {
        lisp_nodes += count_sexpr(v);
      }
    }
  }

  println!("file        : {path}");
  println!("text        : {:.1} MB", bytes as f64 / 1e6);
  println!("parse errors: {errors}");
  println!("statements  : {}", ast.stmts.len());
  println!("formulas    : {} ({} lisp nodes)", spans.len(), lisp_nodes);
  println!("heap parsed : {:.1} MB, peak {:.1} MB", parse_live as f64 / 1e6, peak() as f64 / 1e6);

  let sample: Vec<Span> = spans.iter().copied().step_by(stride.max(1)).collect();
  if stride > 1 {
    println!("sample      : {} of {} formulas (stride {stride})", sample.len(), spans.len());
  }
  if env::var_os("ARENA_DEBUG").is_some() {
    for &sp in sample.iter().take(5) {
      let text = &src.as_bytes()[sp.start..sp.end];
      let toks = text.split(|b| b.is_ascii_whitespace()).filter(|s| !s.is_empty()).count();
      println!("debug span {sp:?} bytes {} tokens {toks} nodes {}",
        sp.end - sp.start, parse_tree(&src, sp).nodes());
    }
  }
  let (tree_nodes, tree_live, biggest) = tree_pass(&src, &sample);
  println!("tree nodes  : {tree_nodes} (largest single formula {biggest})");
  println!("tree heap   : {:.1} MB = {} B/node (peak {:.1} MB)",
    tree_live as f64 / 1e6, tree_live.checked_div(tree_nodes).unwrap_or(0),
    peak() as f64 / 1e6);

  let (it_nodes, distinct, arena_live) = intern_pass(&src, &sample);
  println!("interned    : {distinct} distinct / {it_nodes} nodes ({:.2}%)",
    distinct as f64 * 100.0 / (it_nodes.max(1) as f64));
  println!("node size   : TreeNode {} B, mm0-rs ExprNode {} B",
    std::mem::size_of::<TreeNode>(), std::mem::size_of::<mm0_rs::ExprNode>());
  println!("arena heap  : {:.2} MB = {} B per distinct node, {:.3} B per node",
    arena_live as f64 / 1e6,
    arena_live.checked_div(distinct).unwrap_or(0),
    arena_live as f64 / (it_nodes.max(1) as f64));
}
