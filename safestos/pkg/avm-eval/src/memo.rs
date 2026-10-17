//! The Typst/comemo-style memoization layer (Sprint 2, Task 2.3).
//!
//! Two tiers, both of which can only ever *skip work*, never change a
//! result (evaluation stays referentially transparent):
//!
//! * **L1 — session memo**: name -> id within one process; a second warm
//!   evaluation of the same target does no hashing at all.
//! * **L2 — store presence (id-keyed)**: after a node's id is known, ask
//!   the cold CAS whether a built cell for that exact content-derived id
//!   already exists. Reported as a `store_hits` entry: an executor skips
//!   the *build*, never the derivation of the id.
//!
//! **Why L2 is id-keyed and does not adopt stored ids** (the obvious
//! "look up by name and skip the subtree" shape is unsound): adopting an
//! id under a name means adopting whatever inputs the store happened to
//! build. If the plan's sources or dependencies changed while the name
//! stayed put, warm evaluation would return a *different target id than
//! cold evaluation* — a referential-transparency violation that would
//! poison every downstream derivation. A content-derived id carries its
//! own freshness proof, so the check is `contains_id(id)` after
//! computation. The literal traversal-skip from the plan text needs a
//! freshness-carrying manifest (record the input closure beside
//! `drv_hash`) and is recorded as deferred in PLAN_avm_pkg.md.
//!
//! Cost note: skipping *hashing* was never where the seconds go — hashing
//! 5,000 nodes is milliseconds (measured in
//! `acceptance_warm_eval_5000_nodes_under_20ms`). The cache tier that
//! matters for build time is exactly the one this reports.

use crate::graph::{EvalError, Plan};
use crate::hash::DerivationId;
use dashmap::DashMap;
use std::collections::BTreeMap;
use std::sync::Arc;

/// "The cold CAS already holds a built cell for this exact derivation id"
/// oracle. Ids are content-derived, so there is no freshness hazard in
/// this contract; implemented by the CLI over the store's manifests.
pub trait Presence: Send + Sync {
    fn contains_id(&self, id: &DerivationId) -> bool;
}

/// What one evaluation did (and, crucially, what it did *not* do).
#[derive(Debug, Clone)]
pub struct EvalReport {
    pub target_id: DerivationId,
    pub resolved: BTreeMap<String, DerivationId>,
    /// Dependency-first order of everything reached this run.
    pub order: Vec<String>,
    /// Nodes whose hash was actually computed this run (the L1 miss set).
    pub visited: Vec<String>,
    /// Nodes answered by the session memo (L1).
    pub session_hits: Vec<String>,
    /// Nodes whose built cell already exists in the store (L2): a future
    /// executor skips these builds.
    pub store_hits: Vec<String>,
    /// Nodes the *remote* binary cache already holds pre-built (L3): an
    /// executor hydrates these instead of building them (spec §3.3).
    pub remote_hits: Vec<String>,
}

/// The memo table. `presence: None` disables L2 (pure in-memory memo);
/// `remote: None` disables L3 (no remote binary cache configured).
pub struct MemoTable {
    l1: DashMap<String, DerivationId>,
    presence: Option<Arc<dyn Presence>>,
    remote: Option<Arc<dyn Presence>>,
}

impl MemoTable {
    pub fn new(presence: Option<Arc<dyn Presence>>) -> Self {
        MemoTable {
            l1: DashMap::new(),
            presence,
            remote: None,
        }
    }

    /// L3 variant: also consult a remote binary-cache oracle (spec §3.3).
    /// L2 is checked first — a local store hit never pays a network
    /// consult, and each node is recorded in exactly one tier.
    pub fn with_remote(
        presence: Option<Arc<dyn Presence>>,
        remote: Option<Arc<dyn Presence>>,
    ) -> Self {
        MemoTable {
            l1: DashMap::new(),
            presence,
            remote,
        }
    }

    /// Evaluate `target` with memoized short-circuits. `target: None`
    /// resolves the whole plan deterministically (sorted name order).
    pub fn evaluate(&self, plan: &Plan, target: Option<&str>) -> Result<EvalReport, EvalError> {
        let mut report = EvalReport {
            target_id: [0u8; 32],
            resolved: BTreeMap::new(),
            order: Vec::new(),
            visited: Vec::new(),
            session_hits: Vec::new(),
            store_hits: Vec::new(),
            remote_hits: Vec::new(),
        };
        let roots: Vec<String> = match target {
            Some(t) => {
                if plan.get(t).is_none() {
                    return Err(EvalError::Unknown(t.to_string()));
                }
                vec![t.to_string()]
            }
            None => plan.all_names(),
        };

        let mut stack: Vec<String> = Vec::new();
        let mut last_id: DerivationId = [0u8; 32];
        for r in roots {
            last_id = self.resolve(plan, &r, &mut report, &mut stack)?;
        }
        report.target_id = match target {
            Some(t) => *report.resolved.get(t).expect("root was resolved"),
            None => last_id,
        };
        Ok(report)
    }

    fn resolve(
        &self,
        plan: &Plan,
        name: &str,
        report: &mut EvalReport,
        stack: &mut Vec<String>,
    ) -> Result<DerivationId, EvalError> {
        // L1 fast path: a node already hashed in this session. Hit lists
        // record each node at most once per run (first reach), so reports
        // stay set-like instead of counting every repeated consult.
        if let Some(id) = self.l1.get(name).map(|e| *e) {
            if report.resolved.insert(name.to_string(), id).is_none() {
                report.session_hits.push(name.to_string());
                report.order.push(name.to_string());
                self.check_store(name, &id, report);
            }
            return Ok(id);
        }
        if stack.iter().any(|n| n == name) {
            let start = stack.iter().position(|n| n == name).unwrap_or(0);
            let mut loop_names = stack[start..].to_vec();
            loop_names.push(name.to_string());
            return Err(EvalError::Cycle(loop_names));
        }
        let drv = plan
            .get(name)
            .ok_or_else(|| EvalError::Unknown(name.to_string()))?;
        stack.push(name.to_string());
        let mut dep_ids = Vec::with_capacity(drv.dependencies.len());
        for dep in &drv.dependencies {
            dep_ids.push(self.resolve(plan, dep, report, stack)?);
        }
        stack.pop();

        let id = crate::hash::calculate_derivation_hash(drv, &dep_ids);
        self.l1.insert(name.to_string(), id);
        report.visited.push(name.to_string());
        if report.resolved.insert(name.to_string(), id).is_none() {
            report.order.push(name.to_string());
        }
        self.check_store(name, &id, report);
        Ok(id)
    }

    fn check_store(&self, name: &str, id: &DerivationId, report: &mut EvalReport) {
        if let Some(presence) = &self.presence {
            if presence.contains_id(id) {
                report.store_hits.push(name.to_string());
                return;
            }
        }
        // L3 (spec §3.3): only ids the local store *lacks* consult the
        // remote oracle, and a hit lands in its own tier bucket.
        if let Some(remote) = &self.remote {
            if remote.contains_id(id) {
                report.remote_hits.push(name.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Derivation;
    use std::collections::HashSet;

    fn leaf(name: &str) -> Derivation {
        Derivation {
            name: name.into(),
            version: "1".into(),
            builder_symbol: "stdenv.build".into(),
            dependencies: vec![],
            sources: vec![],
            env_vars: vec![],
            totality: None,
        }
    }

    fn node(name: &str, deps: &[&str]) -> Derivation {
        Derivation {
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            ..leaf(name)
        }
    }

    fn plan3() -> Plan {
        let mut p = Plan::new();
        p.insert(leaf("base")).unwrap();
        p.insert(node("mid", &["base"])).unwrap();
        p.insert(node("app", &["mid"])).unwrap();
        p
    }

    struct SetPresence(HashSet<DerivationId>);
    impl Presence for SetPresence {
        fn contains_id(&self, id: &DerivationId) -> bool {
            self.0.contains(id)
        }
    }

    #[test]
    fn cold_run_visits_all_warm_run_visits_none() {
        let plan = plan3();
        let memo = MemoTable::new(None);
        let cold = memo.evaluate(&plan, Some("app")).unwrap();
        assert_eq!(cold.visited, vec!["base", "mid", "app"]);
        assert!(cold.session_hits.is_empty());

        let warm = memo.evaluate(&plan, Some("app")).unwrap();
        assert!(warm.visited.is_empty(), "second run must do no hashing");
        // The memo answers at the root, so the traversal never descends:
        // `app` is consulted (and answered) from the session tier, while
        // `mid`/`base` are not consulted at all — that is the point.
        assert_eq!(warm.session_hits, vec!["app"]);
        assert!(warm.store_hits.is_empty());
        assert_eq!(
            warm.target_id, cold.target_id,
            "memo may not change results"
        );
    }

    /// Task 2.3: the store tier reports already-built nodes by their
    /// content-derived id, without changing a single resolved id.
    #[test]
    fn store_presence_reports_built_nodes_soundly() {
        let plan = plan3();
        let full = MemoTable::new(None).evaluate(&plan, Some("app")).unwrap();

        // Store holds `mid`'s cell (and nothing else).
        let presence = SetPresence(std::iter::once(full.resolved["mid"]).collect());
        let memo = MemoTable::new(Some(Arc::new(presence)));
        let warm = memo.evaluate(&plan, Some("app")).unwrap();
        assert_eq!(
            warm.store_hits,
            vec!["mid"],
            "built node reported, id-keyed"
        );
        assert_eq!(
            warm.target_id, full.target_id,
            "warm and cold evaluation must agree bit-for-bit"
        );
        // A stale/hostile oracle (ids that are not in this plan) cannot
        // produce hits: nothing resolves to those ids.
        let hostile = SetPresence(std::iter::once([0xEEu8; 32]).collect());
        let memo2 = MemoTable::new(Some(Arc::new(hostile)));
        let h = memo2.evaluate(&plan, Some("app")).unwrap();
        assert!(h.store_hits.is_empty());
        assert_eq!(h.target_id, full.target_id);
    }

    /// Spec §3.3: the L3 (remote binary-cache) oracle is consulted only
    /// where the local store misses, its hits land in `remote_hits`, and
    /// resolved ids stay bit-identical whichever tier answers.
    #[test]
    fn remote_tier_reports_hits_only_when_store_misses() {
        let plan = plan3();
        let full = MemoTable::new(None).evaluate(&plan, Some("app")).unwrap();

        // Store holds `mid`, remote holds `base` — disjoint tiers.
        let store = SetPresence(std::iter::once(full.resolved["mid"]).collect());
        let remote = SetPresence(std::iter::once(full.resolved["base"]).collect());
        let memo = MemoTable::with_remote(Some(Arc::new(store)), Some(Arc::new(remote)));
        let warm = memo.evaluate(&plan, Some("app")).unwrap();
        assert_eq!(warm.store_hits, vec!["mid"], "L2 wins where it applies");
        assert_eq!(warm.remote_hits, vec!["base"], "L3 catches the store miss");
        assert_eq!(warm.target_id, full.target_id, "tiers never change ids");

        // No remote oracle configured: the tier is simply silent.
        let only_store = SetPresence(std::iter::once(full.resolved["mid"]).collect());
        let memo2 = MemoTable::new(Some(Arc::new(only_store)));
        let w2 = memo2.evaluate(&plan, Some("app")).unwrap();
        assert!(w2.remote_hits.is_empty());
    }

    #[test]
    fn cycle_still_detected_on_cold_path() {
        let mut p = Plan::new();
        p.insert(node("a", &["b"])).unwrap();
        p.insert(node("b", &["a"])).unwrap();
        let memo = MemoTable::new(None);
        assert!(matches!(
            memo.evaluate(&p, Some("a")),
            Err(EvalError::Cycle(_))
        ));
    }

    #[test]
    fn whole_plan_evaluation_is_deterministic() {
        let plan = plan3();
        let r1 = MemoTable::new(None).evaluate(&plan, None).unwrap();
        let r2 = MemoTable::new(None).evaluate(&plan, None).unwrap();
        assert_eq!(r1.order, r2.order);
        assert_eq!(r1.resolved, r2.resolved);
        assert_eq!(
            r1.order,
            vec!["base", "mid", "app"],
            "each node exactly once, dependency-first, roots walked in sorted name order"
        );
    }

    /// Spec §5 acceptance: warm evaluation of a 5,000-node derivation DAG
    /// resolves in < 20 ms. "Warm" in its harshest form: a *fresh* memo
    /// table (new process shape) over a store that already holds every
    /// cell (full L2). Run explicitly:
    /// `cargo test -p avm-eval --release -- --ignored warm_eval`.
    #[test]
    #[ignore]
    fn acceptance_warm_eval_5000_nodes_under_20ms() {
        // Layered DAG: 100 layers x 50 nodes; each node depends on three
        // deterministic earlier-layer nodes (acyclic by construction).
        let mut plan = Plan::new();
        for layer in 0..100 {
            for col in 0..50 {
                let name = format!("n{layer:03}-{col:02}");
                let drv = if layer == 0 {
                    leaf(&name)
                } else {
                    let d1 = format!("n{:03}-{:02}", layer - 1, col);
                    let d2 = format!("n{:03}-{:02}", layer - 1, (col + 7) % 50);
                    let d3 = format!("n{:03}-{:02}", layer - 1, (col + 23) % 50);
                    let mut deps: Vec<&str> = vec![&d1, &d2, &d3];
                    deps.sort();
                    deps.dedup();
                    node(&name, &deps)
                };
                plan.insert(drv).unwrap();
            }
        }
        assert_eq!(plan.len(), 5000);

        // Cold pass (fresh memo, no store): computes every id. Whole-plan
        // evaluation (the package set), which is what "warm evaluation of a
        // 5,000-node DAG" measures.
        let cold = MemoTable::new(None).evaluate(&plan, None).unwrap();
        assert_eq!(cold.visited.len(), 5000);

        // Fully warm store: every id built. Fresh memo table = new-process
        // warm evaluation — the measurement the spec asks for.
        let all: HashSet<DerivationId> = cold.resolved.values().copied().collect();
        let presence = Arc::new(SetPresence(all));
        let warm_memo = MemoTable::new(Some(presence));
        let start = std::time::Instant::now();
        let warm = warm_memo.evaluate(&plan, None).unwrap();
        let warm_dt = start.elapsed();
        assert_eq!(warm.target_id, cold.target_id);
        assert_eq!(
            warm.visited.len(),
            5000,
            "ids stay sound (computed), builds skipped"
        );
        assert_eq!(
            warm.store_hits.len(),
            5000,
            "store already holds everything"
        );

        // Second call on the same memo = in-process L1 warm.
        let start2 = std::time::Instant::now();
        let l1warm = warm_memo.evaluate(&plan, None).unwrap();
        let l1_dt = start2.elapsed();
        assert_eq!(l1warm.target_id, cold.target_id);
        assert!(l1warm.visited.is_empty());

        eprintln!(
            "warm eval 5000 nodes: store-warm {:?}, session-warm {:?} (budget 20ms)",
            warm_dt, l1_dt
        );
        assert!(
            warm_dt < std::time::Duration::from_millis(20),
            "store-warm evaluation took {:?}, budget 20ms",
            warm_dt
        );
    }
}
