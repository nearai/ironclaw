//! Multi-leg intent bundling.
//!
//! Legs come back from the solver with an optional `depends_on`
//! pointer to another leg id. The solver already orders them
//! topologically in practice, but we re-sort defensively so a
//! malformed response can't put a deposit before its bridge.
//!
//! The ordering rule: any leg with `depends_on = X` must appear
//! after leg X in the final slice. Cycles and dangling references
//! are hard errors.

use crate::types::IntentLeg;

pub fn order_legs(legs: Vec<IntentLeg>) -> Result<Vec<IntentLeg>, String> {
    // Build an id -> leg map and an id list preserving insertion.
    use std::collections::BTreeMap;
    let mut by_id: BTreeMap<String, IntentLeg> = BTreeMap::new();
    let mut order: Vec<String> = Vec::with_capacity(legs.len());
    for leg in legs {
        if by_id.contains_key(&leg.id) {
            return Err(format!("duplicate leg id '{}'", leg.id));
        }
        order.push(leg.id.clone());
        by_id.insert(leg.id.clone(), leg);
    }

    // Validate depends_on references — must run even for single-leg
    // bundles so a dangling `depends_on` doesn't slip through.
    for leg in by_id.values() {
        if let Some(dep) = &leg.depends_on {
            if !by_id.contains_key(dep) {
                return Err(format!("leg '{}' depends_on unknown leg '{dep}'", leg.id));
            }
            if dep == &leg.id {
                return Err(format!("leg '{}' depends on itself", leg.id));
            }
        }
    }

    if by_id.len() <= 1 {
        return Ok(by_id.into_values().collect());
    }

    // Kahn topological sort over the depends_on edges, using the
    // original insertion order as the tiebreak so deterministic
    // output.
    let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
    for id in &order {
        indegree.insert(id.clone(), 0);
    }
    for leg in by_id.values() {
        if leg.depends_on.is_some() {
            let entry = indegree
                .get_mut(&leg.id)
                .ok_or_else(|| format!("internal: indegree missing leg '{}'", leg.id))?;
            *entry += 1;
        }
    }

    let mut ready: Vec<String> = order
        .iter()
        .filter(|id| indegree.get(*id).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();
    let mut out_ids: Vec<String> = Vec::with_capacity(order.len());
    while let Some(id) = ready.pop() {
        out_ids.push(id.clone());
        // Decrement indegree for anything depending on `id`.
        let dependents: Vec<String> = by_id
            .values()
            .filter(|l| l.depends_on.as_deref() == Some(&id))
            .map(|l| l.id.clone())
            .collect();
        for dep in dependents {
            if let Some(n) = indegree.get_mut(&dep) {
                *n -= 1;
                if *n == 0 {
                    ready.push(dep);
                }
            }
        }
    }

    if out_ids.len() != order.len() {
        return Err("dependency cycle in intent bundle legs".to_string());
    }

    // Preserve original insertion order among independent legs by
    // sorting the result against `order` index.
    let pos: BTreeMap<&String, usize> = order.iter().enumerate().map(|(i, id)| (id, i)).collect();
    let mut ordered = out_ids.clone();
    ordered.sort_by_key(|id| pos.get(id).copied().unwrap_or(usize::MAX));

    // But sorting by insertion order alone can violate topological
    // constraints. The topo output is the authority; we only fall
    // back to insertion-order sort when the topo stage gives us a
    // set of equally-valid orderings. Cheapest valid check: verify
    // the insertion-order sort still respects depends_on.
    let ordered_pos: BTreeMap<&String, usize> =
        ordered.iter().enumerate().map(|(i, id)| (id, i)).collect();
    let valid = by_id.values().all(|leg| match &leg.depends_on {
        None => true,
        Some(dep) => {
            ordered_pos.get(dep).copied().unwrap_or(usize::MAX)
                < ordered_pos.get(&leg.id).copied().unwrap_or(0)
        }
    });
    let final_ids = if valid { ordered } else { out_ids };

    final_ids
        .into_iter()
        .map(|id| {
            by_id
                .remove(&id)
                .ok_or_else(|| format!("internal: leg '{id}' missing from by_id map"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TokenAmount;

    fn leg(id: &str, depends_on: Option<&str>) -> IntentLeg {
        IntentLeg {
            id: id.to_string(),
            kind: "swap".to_string(),
            chain: "base".to_string(),
            near_intent_payload: serde_json::Value::Null,
            depends_on: depends_on.map(String::from),
            min_out: TokenAmount {
                symbol: "USDC".to_string(),
                address: None,
                chain: "base".to_string(),
                amount: "0".to_string(),
                value_usd: "0".to_string(),
            },
            quoted_by: "test".to_string(),
        }
    }

    #[test]
    fn empty_legs_returns_empty() {
        let out = order_legs(vec![]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn single_leg_passes_through() {
        let out = order_legs(vec![leg("a", None)]).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn orders_dependent_legs_after_prerequisite() {
        let out = order_legs(vec![
            leg("c", Some("b")),
            leg("b", Some("a")),
            leg("a", None),
        ])
        .unwrap();
        assert_eq!(out[0].id, "a");
        assert_eq!(out[1].id, "b");
        assert_eq!(out[2].id, "c");
    }

    #[test]
    fn rejects_cycle() {
        let err = order_legs(vec![leg("a", Some("b")), leg("b", Some("a"))]).unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn rejects_dangling_reference() {
        let err = order_legs(vec![leg("a", Some("nowhere"))]).unwrap_err();
        assert!(err.contains("depends_on unknown"));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let err = order_legs(vec![leg("a", None), leg("a", None)]).unwrap_err();
        assert!(err.contains("duplicate"));
    }
}
