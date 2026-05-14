use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;

use crate::state::LoopExecutionState;

/// Decides which capabilities are visible to the model this iteration.
///
/// Pure policy: returns a filter the executor passes to the host when
/// requesting the visible capability surface. Does NOT mutate state.
///
/// The host is the source of truth for the catalog and applies its own
/// scope/grant/auth filters AFTER the strategy filter; the strategy can only
/// narrow, never expand.
///
/// See `docs/reborn/agent-loop-skeleton.md` section 6.
#[async_trait]
pub(crate) trait CapabilityStrategy: Send + Sync {
    async fn filter(&self, state: &LoopExecutionState) -> CapabilityFilter;
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn CapabilityStrategy) {}

/// Strategy-side narrowing of the visible capability surface.
///
/// Variants are mutually exclusive. The host always applies its own
/// scope/grant/auth filters on top; this filter only narrows.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityFilter {
    /// Allow everything the host would otherwise expose.
    #[default]
    All,
    /// Only the capabilities whose IDs appear in the set.
    AllowOnly(Vec<CapabilityId>),
    /// Everything except the capabilities whose IDs appear in the set.
    Deny(Vec<CapabilityId>),
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::CapabilityId;

    use super::CapabilityFilter;

    #[test]
    fn default_filter_allows_all() {
        assert_eq!(CapabilityFilter::default(), CapabilityFilter::All);
    }

    #[test]
    fn filter_round_trips_through_json() {
        let capability_id = CapabilityId::new("test.echo").expect("valid capability id");
        let filters = vec![
            CapabilityFilter::All,
            CapabilityFilter::AllowOnly(vec![capability_id.clone()]),
            CapabilityFilter::Deny(vec![capability_id]),
        ];

        for filter in filters {
            let encoded = serde_json::to_string(&filter).expect("serialize filter");
            let decoded: CapabilityFilter =
                serde_json::from_str(&encoded).expect("deserialize filter");
            assert_eq!(decoded, filter);
        }
    }

    #[test]
    fn filter_serializes_with_snake_case_wire_form() {
        let capability_id = CapabilityId::new("test.echo").expect("valid capability id");

        assert_eq!(
            serde_json::to_string(&CapabilityFilter::All).expect("serialize all"),
            "\"all\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityFilter::AllowOnly(vec![capability_id.clone()]))
                .expect("serialize allow_only"),
            "{\"allow_only\":[\"test.echo\"]}"
        );
        assert_eq!(
            serde_json::to_string(&CapabilityFilter::Deny(vec![capability_id]))
                .expect("serialize deny"),
            "{\"deny\":[\"test.echo\"]}"
        );
    }
}
