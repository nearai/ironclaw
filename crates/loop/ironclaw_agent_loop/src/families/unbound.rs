use std::sync::Arc;

use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_api::prepared_context::STRUCTURED_RESULT_CAPABILITY_ID;

use crate::default_planner::DefaultPlanner;
use crate::family::{ComponentDigest, ComponentIdentity, LoopFamily, LoopFamilyId};
use crate::planner::AgentLoopPlanner;
use crate::strategies::{
    GateNotSupportedStrategy, StructuredOutputReplyAdmissionStrategy, StructuredResultStopStrategy,
};

#[cfg(test)]
const UNBOUND_DEFAULT_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.unbound_default_family.v1:",
    "family_id=unbound_default;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=128),",
    "compaction:ActiveTaskPreservingCompactionStrategy(context_limit=128000,reserve=20000,preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000,ineffective_trip_limit=3),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:DefaultBatchPolicyStrategy(parallel_unless_exclusive),",
    "gate:GateNotSupportedStrategy(abort_gate_not_supported_except_external_tool),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2,model_availability_attempts=12,availability=retry_then_observe,stale_request=iteration_retry_then_observe,output_truncated=observe_then_continue,unauthorized=user_visible_terminal,checkpoint_rejected=abort,transcript_write_failed=user_visible_terminal),",
    "reply_admission:DefaultReplyAdmissionStrategy(reject_empty_and_provider_transcript_artifacts),",
    "stop:DefaultStopConditionStrategy(window=5,repeat=3,failure_run=3,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=1024,wall_clock_limit=none)"
)
.as_bytes();

#[cfg(test)]
const UNBOUND_STRUCTURED_FAMILY_FINGERPRINT: &[u8] = concat!(
    "ironclaw_agent_loop.unbound_structured_family.v1:",
    "family_id=unbound_structured;",
    "identity=component_identity_v1;",
    "planner=DefaultPlanner;",
    "strategies=",
    "context:DefaultContextStrategy(max_messages=128),",
    "compaction:ActiveTaskPreservingCompactionStrategy(context_limit=128000,reserve=20000,preserve_tail=8000,min_compacted=3,min_tail=3,deadline_ms=30000,ineffective_trip_limit=3),",
    "capability:DefaultCapabilityStrategy(all),",
    "model:DefaultModelStrategy(primary_or_fallback_index),",
    "batch:DefaultBatchPolicyStrategy(parallel_unless_exclusive),",
    "gate:GateNotSupportedStrategy(abort_gate_not_supported_except_external_tool),",
    "recovery:DefaultRecoveryStrategy(max_attempts_per_class=2,model_availability_attempts=12,availability=retry_then_observe,stale_request=iteration_retry_then_observe,output_truncated=observe_then_continue,unauthorized=user_visible_terminal,checkpoint_rejected=abort,transcript_write_failed=user_visible_terminal),",
    "reply_admission:StructuredOutputReplyAdmissionStrategy(reject_all_text_finals),",
    "stop:StructuredResultStopStrategy(result_capability=builtin.structured_result,all_failed_batches=invalid_model_output,rejected_reply=invalid_model_output),",
    "drain:DefaultInputDrainStrategy(steering=true,followup=true),",
    "budget:DefaultBudgetStrategy(iteration_limit=1024,wall_clock_limit=none)"
)
.as_bytes();

/// Stable digest: BLAKE3-256 of the unbound-default family fingerprint.
pub const UNBOUND_DEFAULT_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x18, 0x9c, 0xd5, 0x51, 0xc7, 0xa9, 0x40, 0x3f, 0x2a, 0xfe, 0x35, 0xfd, 0x32, 0x7a, 0xc5, 0xc4,
    0x50, 0xa5, 0x14, 0xb5, 0xea, 0x18, 0xf0, 0x68, 0xc4, 0x5d, 0x9a, 0x35, 0xd2, 0xed, 0xc3, 0xa0,
]);

/// Stable digest: BLAKE3-256 of the unbound-structured family fingerprint.
pub const UNBOUND_STRUCTURED_FAMILY_DIGEST: ComponentDigest = ComponentDigest([
    0x44, 0xbf, 0xc2, 0x27, 0xfb, 0x8b, 0x66, 0xbf, 0xf6, 0xc1, 0x8a, 0x00, 0xe8, 0xfd, 0xad, 0xd8,
    0xb0, 0x84, 0xb0, 0x74, 0x60, 0x50, 0xdd, 0x80, 0x00, 0xea, 0xde, 0x5a, 0x51, 0x73, 0x4f, 0xae,
]);

fn structured_result_capability() -> CapabilityId {
    CapabilityId::new(STRUCTURED_RESULT_CAPABILITY_ID)
        .expect("static unbound result capability id is a valid dotted id")
    // safety: compile-time constant validated by the pin test below.
}

/// The unbound-default family (unbound-turn design §4.2): the default
/// text-tool-use composition with the unbound gate posture — approval,
/// auth, resource, and dependent-run gates abort with the typed
/// `gate_not_supported` failure; only the external-tool gate parks.
pub fn unbound_default() -> LoopFamily {
    let planner = DefaultPlanner::compose_default()
        .with_id(LoopFamilyId::UNBOUND_DEFAULT)
        .with_version(ComponentIdentity::from_static(
            "unbound_default",
            UNBOUND_DEFAULT_FAMILY_DIGEST,
        ))
        .with_gate(Arc::new(GateNotSupportedStrategy));
    let id = planner.id().clone();
    let version = planner.version().clone();

    LoopFamily::new(id, version, Arc::new(planner))
}

/// The unbound-structured family (design §4.5): the unbound gate posture
/// plus strict structured-output enforcement — every plain-text final is
/// rejected with a repair hint directing the model to the synthetic result
/// tool, and the run completes when that tool records a validated result.
pub fn unbound_structured() -> LoopFamily {
    let planner = DefaultPlanner::compose_default()
        .with_id(LoopFamilyId::UNBOUND_STRUCTURED)
        .with_version(ComponentIdentity::from_static(
            "unbound_structured",
            UNBOUND_STRUCTURED_FAMILY_DIGEST,
        ))
        .with_gate(Arc::new(GateNotSupportedStrategy))
        .with_reply_admission(Arc::new(StructuredOutputReplyAdmissionStrategy))
        .with_stop(Arc::new(StructuredResultStopStrategy::new(
            structured_result_capability(),
        )));
    let id = planner.id().clone();
    let version = planner.version().clone();

    LoopFamily::new(id, version, Arc::new(planner))
}

#[cfg(test)]
mod tests {
    use crate::families::DEFAULT_FAMILY_DIGEST;

    use super::*;

    #[test]
    fn structured_result_capability_id_is_valid() {
        assert_eq!(
            structured_result_capability().as_str(),
            STRUCTURED_RESULT_CAPABILITY_ID
        );
    }

    #[test]
    fn unbound_families_carry_their_identities() {
        let default_family = unbound_default();
        assert_eq!(default_family.id(), &LoopFamilyId::UNBOUND_DEFAULT);
        assert_eq!(default_family.version().id, "unbound_default");

        let structured = unbound_structured();
        assert_eq!(structured.id(), &LoopFamilyId::UNBOUND_STRUCTURED);
        assert_eq!(structured.version().id, "unbound_structured");
    }

    #[test]
    fn unbound_default_family_digest_matches_blake3_fingerprint() {
        assert_eq!(
            UNBOUND_DEFAULT_FAMILY_DIGEST,
            ComponentDigest::from_blake3(UNBOUND_DEFAULT_FAMILY_FINGERPRINT),
            "update the static digest when the composition changes: {:?}",
            ComponentDigest::from_blake3(UNBOUND_DEFAULT_FAMILY_FINGERPRINT)
        );
    }

    #[test]
    fn unbound_structured_family_digest_matches_blake3_fingerprint() {
        assert_eq!(
            UNBOUND_STRUCTURED_FAMILY_DIGEST,
            ComponentDigest::from_blake3(UNBOUND_STRUCTURED_FAMILY_FINGERPRINT),
            "update the static digest when the composition changes: {:?}",
            ComponentDigest::from_blake3(UNBOUND_STRUCTURED_FAMILY_FINGERPRINT)
        );
    }

    #[test]
    fn unbound_family_digests_are_distinct() {
        assert_ne!(UNBOUND_DEFAULT_FAMILY_DIGEST, DEFAULT_FAMILY_DIGEST);
        assert_ne!(
            UNBOUND_STRUCTURED_FAMILY_DIGEST,
            UNBOUND_DEFAULT_FAMILY_DIGEST
        );
    }
}
