use std::sync::Arc;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily};
use crate::planner::AgentLoopPlanner;
use crate::strategies::{
    DEFAULT_ITERATION_BACKSTOP, DefaultBudgetStrategy, DefaultRecoveryStrategy,
};

mod subagent;

pub use subagent::{SUBAGENT_FAMILY_DIGEST, subagent};

/// Replay-relevant fingerprint of the default family composition, with the
/// two override-able knobs substituted in.
///
/// [`DEFAULT_FAMILY_DIGEST`] is the BLAKE3-256 of this fingerprint at the
/// production defaults; override-built families hash the same fingerprint
/// with their resolved values so a family's [`ComponentIdentity`] digest
/// always identifies the configuration it actually runs with
/// (see `family.rs` component-identity contract).
fn default_family_fingerprint(iteration_limit: u32, model_availability_attempts: u32) -> String {
    format!(
        "ironclaw_agent_loop.default_family.v1:\
        family_id=default;\
        identity=component_identity_v1;\
        planner=DefaultPlanner;\
        strategies=\
        context:DefaultContextStrategy(max_messages=128),\
        compaction:ActiveTaskPreservingCompactionStrategy(context_limit=128000,reserve=20000,preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000,ineffective_trip_limit=3),\
        capability:DefaultCapabilityStrategy(all),\
        model:DefaultModelStrategy(primary_or_fallback_index),\
        batch:DefaultBatchPolicyStrategy(exclusive_sequential),\
        gate:DefaultGateHandlingStrategy(block),\
        recovery:DefaultRecoveryStrategy(max_attempts_per_class=2,model_availability_attempts={model_availability_attempts}),\
        reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),\
        stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3,rejected_reply=invalid_model_output),\
        drain:DefaultInputDrainStrategy(steering=true,followup=true),\
        budget:DefaultBudgetStrategy(iteration_limit={iteration_limit},wall_clock_limit=none)"
    )
}

/// Stable digest: BLAKE3-256 of [`default_family_fingerprint`] at the
/// production defaults.
///
/// Update this digest when the default family composition, planner behavior, or
/// identity schema changes in a replay-relevant way.
pub const DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0xd7, 0xc0, 0xe7, 0xdd, 0xc4, 0x17, 0xe9, 0xe4, 0xe2, 0xcb, 0xa1, 0x4b, 0xab, 0xc4, 0x42, 0x40,
    0x7d, 0x0d, 0x38, 0x64, 0xff, 0xe3, 0x85, 0xaa, 0xbe, 0x6a, 0x7b, 0xc1, 0xc7, 0xcc, 0x1f, 0xad,
]);

/// The default loop family: the text-tool-use baseline.
pub fn default() -> LoopFamily {
    let planner = DefaultPlanner::compose_default();
    let id = planner.id().clone();
    let version = planner.version().clone();

    LoopFamily::new(id, version, Arc::new(planner))
}

/// The default loop family with a caller-supplied iteration limit.
///
/// Intended for test and local harnesses that need to exercise the hard budget
/// path without waiting for the production backstop of 1024 iterations.
pub fn default_with_iteration_limit(iteration_limit: u32) -> LoopFamily {
    default_with_overrides(FamilyOverrides::default().set_iteration_limit(iteration_limit))
}

/// Optional overrides for the default family's replay-relevant knobs. `None`
/// keeps the production default for that knob.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FamilyOverrides {
    /// Hard iteration ceiling; defaults to [`DEFAULT_ITERATION_BACKSTOP`].
    pub iteration_limit: Option<u32>,
    /// Availability-class model retry budget
    /// (`DefaultRecoveryStrategy::max_model_availability_attempts`).
    pub model_availability_attempts: Option<u32>,
}

impl FamilyOverrides {
    pub fn set_iteration_limit(mut self, iteration_limit: u32) -> Self {
        self.iteration_limit = Some(iteration_limit);
        self
    }

    pub fn set_model_availability_attempts(mut self, attempts: u32) -> Self {
        self.model_availability_attempts = Some(attempts);
        self
    }
}

/// The default loop family with optional iteration-limit and model
/// availability-retry overrides.
///
/// The availability override shrinks (or deepens) how long the loop rides out
/// provider outages before aborting — test harnesses that script provider
/// failures set it low so a deliberately failed run reaches `Failed` in
/// seconds instead of retrying for minutes.
///
/// Overrides are replay-relevant configuration, so an overridden composition
/// carries a configuration-specific [`ComponentIdentity`] digest derived from
/// the resolved values; only the pure-default composition keeps the static
/// [`DEFAULT_FAMILY_DIGEST`], so existing replay identities are unchanged.
pub fn default_with_overrides(overrides: FamilyOverrides) -> LoopFamily {
    if overrides == FamilyOverrides::default() {
        return default();
    }
    let iteration_limit = overrides
        .iteration_limit
        .unwrap_or(DEFAULT_ITERATION_BACKSTOP);
    let max_model_availability_attempts = overrides
        .model_availability_attempts
        .unwrap_or(DefaultRecoveryStrategy::default().max_model_availability_attempts);
    let digest = ComponentDigest::from_blake3(default_family_fingerprint(
        iteration_limit,
        max_model_availability_attempts,
    ));
    let planner = DefaultPlanner::compose_default()
        .with_version(ComponentIdentity::new("default", digest))
        .with_budget(Arc::new(DefaultBudgetStrategy {
            iteration_limit,
            wall_clock_limit: None,
        }))
        .with_recovery(Arc::new(DefaultRecoveryStrategy {
            max_model_availability_attempts,
            ..DefaultRecoveryStrategy::default()
        }));
    let id = planner.id().clone();
    let version = planner.version().clone();

    LoopFamily::new(id, version, Arc::new(planner))
}

#[cfg(test)]
mod tests {
    use crate::family::LoopFamilyId;

    use super::*;

    #[test]
    fn default_family_has_default_identity() {
        let family = default();

        assert_eq!(family.id(), &LoopFamilyId::DEFAULT);
        assert_eq!(family.version().id, "default");
        assert_ne!(family.version().digest, ComponentDigest([0; 32]));
        assert_eq!(family.version().digest, DEFAULT_FAMILY_DIGEST);
    }

    #[test]
    fn default_family_iteration_limit_can_be_overridden_for_harnesses() {
        let family = default_with_iteration_limit(5);
        let context = crate::test_support::test_run_context("default-family-budget-override");
        let state = crate::state::LoopExecutionState::initial_for_run(&context);

        assert_eq!(family.planner().budget().iteration_limit(&state), 5);
    }

    #[test]
    fn default_family_digest_matches_blake3_fingerprint() {
        assert_eq!(
            DEFAULT_FAMILY_DIGEST,
            ComponentDigest::from_blake3(default_family_fingerprint(
                DEFAULT_ITERATION_BACKSTOP,
                DefaultRecoveryStrategy::default().max_model_availability_attempts,
            ))
        );
    }

    #[test]
    fn override_built_families_carry_configuration_specific_digests() {
        // The component-identity contract (family.rs): the digest identifies
        // replay-relevant configuration. Overriding a budget or retry knob is
        // replay-relevant, so it must change the digest.
        let attempts_override =
            default_with_overrides(FamilyOverrides::default().set_model_availability_attempts(1));
        assert_ne!(attempts_override.version().digest, DEFAULT_FAMILY_DIGEST);
        assert_eq!(attempts_override.version().id, "default");

        let iteration_override =
            default_with_overrides(FamilyOverrides::default().set_iteration_limit(5));
        assert_ne!(iteration_override.version().digest, DEFAULT_FAMILY_DIGEST);
        assert_ne!(
            iteration_override.version().digest,
            attempts_override.version().digest
        );

        // The digest is a pure function of the resolved configuration.
        let attempts_override_again =
            default_with_overrides(FamilyOverrides::default().set_model_availability_attempts(1));
        assert_eq!(
            attempts_override_again.version().digest,
            attempts_override.version().digest
        );

        // No overrides keeps the static default replay identity.
        let unchanged = default_with_overrides(FamilyOverrides::default());
        assert_eq!(unchanged.version().digest, DEFAULT_FAMILY_DIGEST);

        // Overrides that spell out the production defaults hash to the same
        // digest as the static const — identity is config-addressed.
        let explicit_defaults = default_with_overrides(
            FamilyOverrides::default()
                .set_iteration_limit(DEFAULT_ITERATION_BACKSTOP)
                .set_model_availability_attempts(
                    DefaultRecoveryStrategy::default().max_model_availability_attempts,
                ),
        );
        assert_eq!(explicit_defaults.version().digest, DEFAULT_FAMILY_DIGEST);
    }

    #[tokio::test]
    async fn availability_attempts_override_gives_one_retry_then_abort() {
        use crate::state::{LoopExecutionState, RecoveryAttemptClass, RecoveryStrategyState};
        use crate::strategies::{
            ModelErrorClass, ModelErrorSummary, RecoveryOutcome, SanitizedStrategySummary,
        };

        let family =
            default_with_overrides(FamilyOverrides::default().set_model_availability_attempts(1));
        let context = crate::test_support::test_run_context("default-family-attempts-override");
        let err = ModelErrorSummary {
            class: ModelErrorClass::Unavailable,
            safe_summary: SanitizedStrategySummary::from_trusted_static("test"),
            diagnostic_ref: None,
        };

        let state = LoopExecutionState::initial_for_run(&context);
        let outcome = family
            .planner()
            .recovery()
            .on_model_error(&state, &err)
            .await;
        assert!(
            matches!(outcome, RecoveryOutcome::Retry { .. }),
            "first availability failure should retry, got {outcome:?}"
        );

        let mut exhausted = LoopExecutionState::initial_for_run(&context);
        exhausted.recovery_state =
            RecoveryStrategyState::with_attempts_for(RecoveryAttemptClass::ModelUnavailable, 1);
        let outcome = family
            .planner()
            .recovery()
            .on_model_error(&exhausted, &err)
            .await;
        assert!(
            matches!(outcome, RecoveryOutcome::Abort { .. }),
            "second availability failure should abort at attempts=1, got {outcome:?}"
        );
    }
}
