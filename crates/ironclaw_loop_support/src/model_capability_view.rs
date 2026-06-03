use std::collections::BTreeSet;

use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::LoopModelCapabilityView;

pub(crate) struct ModelCapabilityViewIntersection {
    pub(crate) view: LoopModelCapabilityView,
    pub(crate) dropped_capabilities: Vec<CapabilityId>,
}

/// Keep the intersection logic beside subagent prompt composition.
///
/// `LoopModelCapabilityView` is defined in `ironclaw_turns::run_profile` as the
/// shared prompt contract, but the narrowing policy and dropped-capability
/// logging live in `ironclaw_loop_support` because this crate owns the prompt
/// materialization boundary that consumes the result.
pub(crate) fn intersect_model_capability_view(
    mut visible_capability_ids: BTreeSet<CapabilityId>,
    existing_view: Option<LoopModelCapabilityView>,
) -> ModelCapabilityViewIntersection {
    let mut dropped_capabilities = Vec::new();
    if let Some(existing_view) = existing_view {
        let existing = existing_view
            .visible_capability_ids
            .into_iter()
            .collect::<BTreeSet<_>>();
        visible_capability_ids.retain(|capability| {
            let keep = existing.contains(capability);
            if !keep {
                dropped_capabilities.push(capability.clone());
            }
            keep
        });
    }
    ModelCapabilityViewIntersection {
        view: LoopModelCapabilityView {
            visible_capability_ids: visible_capability_ids.into_iter().collect(),
        },
        dropped_capabilities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("valid test capability")
    }

    #[test]
    fn returns_full_visible_set_when_existing_view_is_absent() {
        let intersection = intersect_model_capability_view(
            BTreeSet::from([cap("demo.write"), cap("demo.read")]),
            None,
        );

        assert_eq!(
            intersection
                .view
                .visible_capability_ids
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["demo.read", "demo.write"]
        );
        assert!(intersection.dropped_capabilities.is_empty());
    }

    #[test]
    fn intersects_visible_set_with_existing_view() {
        let intersection = intersect_model_capability_view(
            BTreeSet::from([cap("demo.write"), cap("demo.read")]),
            Some(LoopModelCapabilityView {
                visible_capability_ids: vec![cap("demo.read"), cap("demo.other")],
            }),
        );

        assert_eq!(
            intersection
                .view
                .visible_capability_ids
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["demo.read"]
        );
        assert_eq!(
            intersection
                .dropped_capabilities
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["demo.write"]
        );
    }
}
