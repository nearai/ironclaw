//! Self-authored hooks — the fourth trust class.
//!
//! These hooks are authored at runtime by the agent itself, typically in
//! response to an observed near-miss or a repetition pattern that the agent
//! wants to constrain on subsequent turns. Two structural properties keep
//! the surface narrow enough that self-authorship cannot be used to
//! exfiltrate authority:
//!
//! 1. **Monotonic-restriction only.** A self-authored hook can only
//!    *restrict* future behavior. The sink trait carries no `allow()`, no
//!    trusted-snippet path, and no `Effect`-class constructor. The
//!    underlying decision vocabulary is `deny` / `pause_approval` /
//!    `pause_auth` / `pass`.
//! 2. **Closed declarative vocabulary.** The spec the agent emits is
//!    typed via [`SelfAuthoredHookSpec`], whose reasons are
//!    [`SelfAuthoredReason`] enum variants — not free-text strings. The
//!    agent can compose constraints but cannot smuggle adversarial reason
//!    strings into the audit log or the user prompt.
//!
//! ## Run-scoped only (today)
//!
//! Per the design in #3567, self-authored hooks are *only* registered for
//! the current turn run. Durable self-authorship requires an unforgeable
//! channel between the agent's reasoning step and the registry, tracked by
//! #3564. The types here model the run-scoped slice; persistence will land
//! in a follow-up alongside that channel.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::SanitizedReason;
use crate::identity::HookId;
use crate::kinds::gate::BeforeCapabilityHookDecision;
use crate::points::BeforeCapabilityHookContext;
use crate::predicate::CapabilityPredicate;
use crate::sink::GateSinkState;
use ironclaw_turns::{TurnId, TurnRunId};

/// Closed vocabulary of static labels the agent may use as a reason when
/// authoring a hook. Free-text reasons are intentionally not allowed — the
/// constrained surface prevents adversarial reason content from leaking
/// into model-visible audit, and it makes downstream analytics tractable
/// (the set of labels is bounded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfAuthoredReason {
    /// Agent observed a near-miss while attempting a sensitive capability
    /// and wants to deny similar attempts for the rest of the run.
    AgentObservedNearMiss,
    /// Agent observed repeated identical capability invocations and wants
    /// to throttle or block further repetitions.
    AgentObservedRepetition,
    /// Agent observed scope drift (capability outside the user-stated goal)
    /// and wants to require approval before continuing.
    AgentObservedScopeDrift,
    /// Agent inferred a policy boundary from prior user direction and
    /// wants to enforce it for the remainder of the run.
    AgentInferredUserPolicy,
}

impl SelfAuthoredReason {
    /// The closed-vocabulary label this variant emits to the sink.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AgentObservedNearMiss => "self_authored_near_miss",
            Self::AgentObservedRepetition => "self_authored_repetition",
            Self::AgentObservedScopeDrift => "self_authored_scope_drift",
            Self::AgentInferredUserPolicy => "self_authored_inferred_user_policy",
        }
    }
}

/// A self-authored hook spec. Shape mirrors [`HookPredicateSpec`] but the
/// reason field is a closed enum, not free text — and the spec
/// deliberately omits any rate / value / numeric-cap surface (no shared
/// state across runs, no per-run state machine).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SelfAuthoredHookSpec {
    /// Deny capability invocations that match `when`.
    DenyCapability {
        when: CapabilityPredicate,
        reason: SelfAuthoredReason,
    },
    /// Pause for approval when `when` matches.
    PauseApproval {
        when: CapabilityPredicate,
        reason: SelfAuthoredReason,
    },
}

impl SelfAuthoredHookSpec {
    /// Stable 32-byte digest of the spec, suitable for provenance
    /// bookkeeping. The digest covers the canonical JSON representation so
    /// semantically-identical specs produce identical digests across
    /// processes.
    pub fn digest(&self) -> [u8; 32] {
        // serde_json with our derived Serialize impls is canonical for our
        // closed vocabularies (no maps with non-deterministic iteration).
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        let mut hasher = blake3::Hasher::new();
        hasher.update(&bytes);
        hasher.finalize().into()
    }
}

/// Opaque pointer to the agent reasoning trace that produced a
/// self-authored hook. The pointer is a content-addressed reference into
/// the run's reasoning ledger — not the trace contents — so that audit
/// consumers can reproduce the authorship chain without inlining the
/// reasoning into the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationTraceRef(String);

impl GenerationTraceRef {
    pub fn new(reference: String) -> Self {
        Self(reference)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque pointer to a user-ratification artifact. Self-authored hooks may
/// optionally be ratified by the user (e.g., "yes, never let me run shell
/// without approval again"); when present, ratification upgrades the hook's
/// authority for downstream registries to consider durable persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRatificationProof(String);

impl UserRatificationProof {
    pub fn new(proof: String) -> Self {
        Self(proof)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Provenance for a self-authored hook. Captures who, when, and what
/// reasoning chain produced the hook so that audit consumers can trace any
/// run-scoped self-authored decision back to its authoring turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfAuthorshipProvenance {
    pub authored_by_run: TurnRunId,
    pub authored_by_turn: TurnId,
    pub authored_at: DateTime<Utc>,
    pub spec_digest: [u8; 32],
    pub user_ratification: Option<UserRatificationProof>,
    pub generation_trace_ref: GenerationTraceRef,
}

/// Sink surface exposed to self-authored hooks. Deliberately narrower than
/// [`crate::sink::RestrictedGateSink`]: there is no `allow`, no trusted-
/// snippet path, and no effect-class constructor. Reasons are passed as
/// `SelfAuthoredReason` values, not free text, to keep the audit
/// vocabulary closed.
pub trait SelfAuthoredHookSink: Send {
    fn deny(&mut self, reason: SelfAuthoredReason);
    fn pause_approval(&mut self, reason: SelfAuthoredReason);
    fn pause_auth(&mut self, reason: SelfAuthoredReason);
    /// Record that the hook has no opinion on this invocation.
    fn pass(&mut self);
}

/// Dispatcher-internal sink for self-authored hooks. Records the decision
/// (if any) into a [`GateSinkState`] so the dispatcher can plug into the
/// existing `before_capability` composition with no extra plumbing. Made
/// `pub(crate)` so the dispatcher slice that wires self-authored hooks
/// into the gate composer can construct it directly without rebuilding
/// the GateSinkState mapping. Tests construct it via the same path.
#[allow(dead_code)] // dispatcher wiring lands alongside #3564
pub(crate) struct RecordingSelfAuthoredSink {
    pub(crate) state: GateSinkState,
}

impl RecordingSelfAuthoredSink {
    #[allow(dead_code)] // see struct-level note
    pub(crate) fn new() -> Self {
        Self {
            state: GateSinkState::Unset,
        }
    }
}

impl SelfAuthoredHookSink for RecordingSelfAuthoredSink {
    fn deny(&mut self, reason: SelfAuthoredReason) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::deny(
            SanitizedReason::from_static(reason.label()),
        ));
    }

    fn pause_approval(&mut self, reason: SelfAuthoredReason) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::pause_approval(
            SanitizedReason::from_static(reason.label()),
        ));
    }

    fn pause_auth(&mut self, reason: SelfAuthoredReason) {
        self.state = GateSinkState::Decided(BeforeCapabilityHookDecision::pause_auth(
            SanitizedReason::from_static(reason.label()),
        ));
    }

    fn pass(&mut self) {
        self.state = GateSinkState::Passed;
    }
}

/// Stateless evaluator for self-authored specs. Unlike the
/// [`crate::evaluator::PredicateEvaluator`], the self-authored evaluator
/// holds no sliding-window state: the run-scoped slice supports only
/// deny / pause / pass decisions over the immediate capability context.
/// Rate-cap-style self-authorship lands alongside the unforgeable channel
/// from #3564.
#[derive(Debug, Default)]
pub struct SelfAuthoredEvaluator;

impl SelfAuthoredEvaluator {
    pub fn new() -> Self {
        Self
    }

    fn matches(&self, spec: &SelfAuthoredHookSpec, ctx: &BeforeCapabilityHookContext) -> bool {
        match spec {
            SelfAuthoredHookSpec::DenyCapability { when, .. }
            | SelfAuthoredHookSpec::PauseApproval { when, .. } => predicate_matches(when, ctx),
        }
    }
}

fn predicate_matches(predicate: &CapabilityPredicate, ctx: &BeforeCapabilityHookContext) -> bool {
    match predicate {
        CapabilityPredicate::Always => true,
        CapabilityPredicate::NameEquals { name } => &ctx.capability_name == name,
        CapabilityPredicate::NameStartsWith { prefix } => ctx.capability_name.starts_with(prefix),
        CapabilityPredicate::All { predicates } => {
            predicates.iter().all(|p| predicate_matches(p, ctx))
        }
        CapabilityPredicate::Any { predicates } => {
            predicates.iter().any(|p| predicate_matches(p, ctx))
        }
    }
}

/// A `before_capability` hook authored at runtime by the agent itself.
/// Always [`HookTrustClass::SelfAuthored`](crate::trust::HookTrustClass::SelfAuthored)
/// at the binding level; the impl here is run-scoped.
pub struct SelfAuthoredBeforeCapabilityHook {
    #[allow(dead_code)] // surfaced via provenance once binding wiring lands
    hook_id: HookId,
    spec: SelfAuthoredHookSpec,
    evaluator: SelfAuthoredEvaluator,
    #[allow(dead_code)] // serialized into audit by a follow-up slice
    provenance: SelfAuthorshipProvenance,
}

impl SelfAuthoredBeforeCapabilityHook {
    pub fn new(
        hook_id: HookId,
        spec: SelfAuthoredHookSpec,
        provenance: SelfAuthorshipProvenance,
    ) -> Self {
        Self {
            hook_id,
            spec,
            evaluator: SelfAuthoredEvaluator::new(),
            provenance,
        }
    }

    /// Evaluate against `ctx` and emit the result into `sink`. Pure: no
    /// internal state mutates between calls.
    pub fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn SelfAuthoredHookSink) {
        if !self.evaluator.matches(&self.spec, ctx) {
            sink.pass();
            return;
        }
        match &self.spec {
            SelfAuthoredHookSpec::DenyCapability { reason, .. } => sink.deny(*reason),
            SelfAuthoredHookSpec::PauseApproval { reason, .. } => sink.pause_approval(*reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId, HookVersion};

    fn tenant() -> ironclaw_host_api::TenantId {
        ironclaw_host_api::TenantId::new("alpha").expect("tenant")
    }

    fn hook_id() -> HookId {
        HookId::derive(
            &ExtensionId::new("self").expect("valid ExtensionId in test"),
            "run",
            &HookLocalId::new("h").expect("valid HookLocalId in test"),
            HookVersion::ONE,
        )
    }

    fn provenance(spec: &SelfAuthoredHookSpec) -> SelfAuthorshipProvenance {
        SelfAuthorshipProvenance {
            authored_by_run: TurnRunId::new(),
            authored_by_turn: TurnId::new(),
            authored_at: Utc::now(),
            spec_digest: spec.digest(),
            user_ratification: None,
            generation_trace_ref: GenerationTraceRef::new("trace://run/turn/step".to_string()),
        }
    }

    #[test]
    fn self_authored_sink_has_no_allow_method() {
        // Compile-time check: the trait surface has no `allow`. If `allow`
        // were added, this method body would have to call it explicitly —
        // we never write that call here, so the property is structural.
        // The runtime check below confirms that only deny/pause/pass paths
        // mutate the sink state.
        fn assert_surface<S: SelfAuthoredHookSink>(sink: &mut S) {
            sink.pass();
            sink.deny(SelfAuthoredReason::AgentObservedNearMiss);
            sink.pause_approval(SelfAuthoredReason::AgentObservedRepetition);
            sink.pause_auth(SelfAuthoredReason::AgentObservedScopeDrift);
        }
        let mut sink = RecordingSelfAuthoredSink::new();
        assert_surface(&mut sink);
        // After exercising every method, the recorded state matches the
        // last call (pause_auth). The point is that none of these calls
        // produced an Allow decision.
        match &sink.state {
            GateSinkState::Decided(d) => assert!(!d.permits()),
            other => panic!("expected decided, got {other:?}"),
        }
    }

    #[test]
    fn self_authored_spec_uses_closed_vocabulary() {
        let spec = SelfAuthoredHookSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: SelfAuthoredReason::AgentObservedNearMiss,
        };
        // Digest is deterministic across constructions.
        let a = spec.digest();
        let b = spec.digest();
        assert_eq!(a, b);

        // Different reasons produce different digests — the closed
        // vocabulary still differentiates structurally.
        let other = SelfAuthoredHookSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: SelfAuthoredReason::AgentObservedRepetition,
        };
        assert_ne!(spec.digest(), other.digest());
    }

    #[test]
    fn self_authored_hook_evaluates_to_deny_on_match() {
        let spec = SelfAuthoredHookSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: SelfAuthoredReason::AgentObservedNearMiss,
        };
        let prov = provenance(&spec);
        let hook = SelfAuthoredBeforeCapabilityHook::new(hook_id(), spec, prov);

        let ctx = BeforeCapabilityHookContext::new_unresolved(
            tenant(),
            "shell.exec".to_string(),
            [0u8; 32],
        );
        let mut sink = RecordingSelfAuthoredSink::new();
        hook.evaluate(&ctx, &mut sink);
        match &sink.state {
            GateSinkState::Decided(d) => assert!(!d.permits()),
            other => panic!("expected deny decision, got {other:?}"),
        }

        // A non-matching capability passes.
        let ctx_other = BeforeCapabilityHookContext::new_unresolved(
            tenant(),
            "memory.read".to_string(),
            [0u8; 32],
        );
        let mut sink_other = RecordingSelfAuthoredSink::new();
        hook.evaluate(&ctx_other, &mut sink_other);
        assert_eq!(sink_other.state, GateSinkState::Passed);
    }

    #[test]
    fn provenance_round_trips_through_serde() {
        let spec = SelfAuthoredHookSpec::PauseApproval {
            when: CapabilityPredicate::Always,
            reason: SelfAuthoredReason::AgentInferredUserPolicy,
        };
        let prov = provenance(&spec);
        let json = serde_json::to_string(&prov).expect("ser");
        let back: SelfAuthorshipProvenance = serde_json::from_str(&json).expect("de");
        assert_eq!(prov, back);
    }
}
