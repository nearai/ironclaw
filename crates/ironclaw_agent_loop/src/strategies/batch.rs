//! `BatchPolicyStrategy` - decides whether a capability batch executes
//! sequentially or in parallel.
//!
//! Pure policy and synchronous: the strategy never consults the host and
//! mutates nothing. Per-capability concurrency hints from descriptors are
//! authoritative for "this specific call must run alone"; this strategy
//! decides only the batch-level default.
//!
//! See `docs/reborn/agent-loop-skeleton.md` section 6 ("Strategy decomposition"
//! -> batch policy) and `contracts/turns-agent-loop.md` section 6 (the loop
//! never sees raw tool input - only the sanitized projection).

use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::ConcurrencyHint;

use crate::state::LoopExecutionState;

/// Decides whether a capability batch executes sequentially or in parallel.
///
/// `&self` only - the strategy is value-immutable. The host's per-capability
/// concurrency hints (from descriptors) override this batch-level default
/// for any individual call that declares itself [`ConcurrencyHint::Exclusive`].
pub(crate) trait BatchPolicyStrategy: Send + Sync {
    fn policy(&self, state: &LoopExecutionState, calls: &[CapabilityCallSummary]) -> BatchPolicy;
}

/// Compile-time object-safety check. `BatchPolicyStrategy` is pure-sync
/// policy, but we still want it usable behind a trait object so the
/// executor can hold a heterogeneous strategy stack.
#[allow(dead_code)]
fn _batch_policy_strategy_object_safe(_: &dyn BatchPolicyStrategy) {}

/// Batch-level execution mode. Wire-stable: serialized into checkpoints and
/// emitted on observability events, so the snake_case names are part of the
/// public contract.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BatchPolicy {
    Sequential,
    Parallel,
}

/// Loop-side projection of one entry in a `CapabilityCalls` batch - name plus
/// concurrency hint only. The strategy never sees raw args (per
/// `contracts/turns-agent-loop.md` section 6 - sanitization happens at the host
/// port boundary).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityCallSummary {
    pub(crate) name: CapabilityId,
    pub(crate) concurrency_hint: ConcurrencyHint,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn batch_policy_round_trips_snake_case() {
        for (variant, wire) in [
            (BatchPolicy::Sequential, "sequential"),
            (BatchPolicy::Parallel, "parallel"),
        ] {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, json!(wire));
            let restored: BatchPolicy = serde_json::from_value(value).expect("deserialize");
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn capability_call_summary_round_trips() {
        let summary = CapabilityCallSummary {
            name: CapabilityId::new("demo.echo").expect("valid"),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
        };
        let value = serde_json::to_value(&summary).expect("serialize");
        let restored: CapabilityCallSummary = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, summary);
    }
}
