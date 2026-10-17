//! The derivation Merkle DAG (Sprint 2, Task 2.2).
//!
//! A [`Plan`] is a named set of pure derivations; resolution walks it
//! bottom-up, detecting cycles, and produces each node's Merkle identity
//! plus a dependency-first (topological) evaluation order. All iteration is
//! over `BTreeMap`s, so reports and orders are byte-stable across runs.

use crate::hash::{calculate_derivation_hash, DerivationId};
use std::collections::{BTreeMap, BTreeSet};

/// One pure package derivation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Derivation {
    pub name: String,
    pub version: String,
    pub builder_symbol: String,
    /// Names of other derivations *in the same plan*.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Store paths (hex digests) of external source inputs.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Environment pairs.
    #[serde(default)]
    pub env_vars: Vec<(String, String)>,
}

#[derive(Debug)]
pub enum EvalError {
    Unknown(String),
    Cycle(Vec<String>),
    Duplicate(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Unknown(n) => write!(f, "unknown derivation: {n}"),
            EvalError::Cycle(loop_names) => {
                write!(f, "dependency cycle: {}", loop_names.join(" -> "))
            }
            EvalError::Duplicate(n) => write!(f, "duplicate derivation name: {n}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Resolved view of a plan: every node's Merkle id, in dependency-first order.
#[derive(Debug, Clone)]
pub struct ResolvedGraph {
    pub ids: BTreeMap<String, DerivationId>,
    pub order: Vec<String>,
}

/// A named set of derivations (a package set fragment).
#[derive(Debug, Clone, Default)]
pub struct Plan {
    derivations: BTreeMap<String, Derivation>,
}

impl Plan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, drv: Derivation) -> Result<(), EvalError> {
        if self.derivations.contains_key(&drv.name) {
            return Err(EvalError::Duplicate(drv.name));
        }
        self.derivations.insert(drv.name.clone(), drv);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Derivation> {
        self.derivations.get(name)
    }

    /// Sorted names (the plan is a BTreeMap, so this is stable).
    pub fn all_names(&self) -> Vec<String> {
        self.derivations.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.derivations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.derivations.is_empty()
    }

    /// Resolve every node reachable from `target` (or the whole plan when
    /// `target` is `None`): Merkle ids bottom-up, cycle detection via the
    /// recursion stack, deterministic dependency-first order.
    pub fn resolve(&self, target: Option<&str>) -> Result<ResolvedGraph, EvalError> {
        let mut ids: BTreeMap<String, DerivationId> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut stack: Vec<String> = Vec::new();
        let mut on_stack: BTreeSet<String> = BTreeSet::new();

        let roots: Vec<String> = match target {
            Some(t) => {
                if !self.derivations.contains_key(t) {
                    return Err(EvalError::Unknown(t.to_string()));
                }
                vec![t.to_string()]
            }
            None => self.derivations.keys().cloned().collect(),
        };

        for r in roots {
            self.resolve_node(&r, &mut ids, &mut order, &mut stack, &mut on_stack)?;
        }
        Ok(ResolvedGraph { ids, order })
    }

    fn resolve_node(
        &self,
        name: &str,
        ids: &mut BTreeMap<String, DerivationId>,
        order: &mut Vec<String>,
        stack: &mut Vec<String>,
        on_stack: &mut BTreeSet<String>,
    ) -> Result<DerivationId, EvalError> {
        if let Some(id) = ids.get(name) {
            return Ok(*id);
        }
        let drv = self
            .derivations
            .get(name)
            .ok_or_else(|| EvalError::Unknown(name.to_string()))?;
        if !on_stack.insert(name.to_string()) {
            let start = stack.iter().position(|n| n == name).unwrap_or(0);
            let mut loop_names: Vec<String> = stack[start..].to_vec();
            loop_names.push(name.to_string());
            return Err(EvalError::Cycle(loop_names));
        }
        stack.push(name.to_string());

        let mut dep_ids = Vec::with_capacity(drv.dependencies.len());
        for dep in &drv.dependencies {
            dep_ids.push(self.resolve_node(dep, ids, order, stack, on_stack)?);
        }
        let id = calculate_derivation_hash(drv, &dep_ids);

        stack.pop();
        on_stack.remove(name);
        order.push(name.to_string());
        ids.insert(name.to_string(), id);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(name: &str) -> Derivation {
        Derivation {
            name: name.into(),
            version: "1".into(),
            builder_symbol: "stdenv.build".into(),
            dependencies: vec![],
            sources: vec![],
            env_vars: vec![],
        }
    }

    fn node(name: &str, deps: &[&str]) -> Derivation {
        Derivation {
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            ..leaf(name)
        }
    }

    /// Task 2.2 acceptance, end to end: swapping *declaration order* of
    /// independent derivations must yield an identical resolved id for the
    /// root.
    #[test]
    fn declaration_order_is_irrelevant() {
        let mut p1 = Plan::new();
        p1.insert(leaf("x")).unwrap();
        p1.insert(leaf("y")).unwrap();
        p1.insert(node("root", &["x", "y"])).unwrap();

        let mut p2 = Plan::new();
        p2.insert(node("root", &["y", "x"])).unwrap();
        p2.insert(leaf("y")).unwrap();
        p2.insert(leaf("x")).unwrap();

        let g1 = p1.resolve(Some("root")).unwrap();
        let g2 = p2.resolve(Some("root")).unwrap();
        assert_eq!(g1.ids["root"], g2.ids["root"]);
        assert_ne!(g1.ids["root"], g1.ids["x"]);
    }

    #[test]
    fn topo_order_is_dependency_first() {
        let mut p = Plan::new();
        p.insert(leaf("base")).unwrap();
        p.insert(node("mid", &["base"])).unwrap();
        p.insert(node("app", &["mid"])).unwrap();
        let g = p.resolve(Some("app")).unwrap();
        assert_eq!(g.order, vec!["base", "mid", "app"]);
    }

    #[test]
    fn cycle_detection_names_the_loop() {
        let mut p = Plan::new();
        p.insert(node("a", &["b"])).unwrap();
        p.insert(node("b", &["a"])).unwrap();
        match p.resolve(Some("a")) {
            Err(EvalError::Cycle(names)) => {
                assert!(names.contains(&"a".to_string()) && names.contains(&"b".to_string()));
            }
            other => panic!("expected cycle, got {other:?}"),
        }
    }

    #[test]
    fn unknown_dependency_is_an_error() {
        let mut p = Plan::new();
        p.insert(node("a", &["ghost"])).unwrap();
        assert!(matches!(p.resolve(Some("a")), Err(EvalError::Unknown(_))));
    }

    #[test]
    fn duplicate_name_rejected() {
        let mut p = Plan::new();
        p.insert(leaf("a")).unwrap();
        assert!(matches!(p.insert(leaf("a")), Err(EvalError::Duplicate(_))));
    }
}
