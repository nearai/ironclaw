use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ExtensionId, RuntimeKind};
use serde::{Deserialize, Serialize};

use crate::{
    CapabilityActivityId, LoopExitId, LoopGateRef, LoopMessageRef, TurnCheckpointId, TurnId,
    TurnRunId, TurnScope,
};

use super::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, BatchPolicyKind, CapabilityFailureKind,
    CapabilitySurfaceVersion, LoopCheckpointKind, LoopDriverNoteKind, LoopGateKind,
    LoopPromptBundleRef, LoopRunContext, LoopSafeSummary, PromptMode,
};
use super::refs::{LoopDriverId, ModelProfileId};
use crate::{LoopCompletionKind, LoopFailureKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopHostMilestone {
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub loop_driver_id: LoopDriverId,
    pub kind: LoopHostMilestoneKind,
}

impl LoopHostMilestone {
    fn from_context(context: &LoopRunContext, kind: LoopHostMilestoneKind) -> Self {
        Self {
            scope: context.scope.clone(),
            turn_id: context.turn_id,
            run_id: context.run_id,
            loop_driver_id: context.loop_driver_id.clone(),
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSkillContextMetadata {
    pub ordinal: usize,
    pub source_name: String,
    pub trust_level: String,
}

/// Public wire shape for host-loop milestones.
///
/// Milestones may be serialized into traces or delivered across process
/// boundaries. Consumers must treat this enum as extensible and prefer
/// [`LoopHostMilestoneKind::kind_name`] plus a catch-all branch rather than
/// assuming the historical closed set. `PromptBundleBuilt` was added as an
/// additive wire-format variant for prompt-bundle construction; it carries only
/// refs, mode, optional surface version, counts, and active-skill metadata,
/// never raw prompt/model content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopHostMilestoneKind {
    IterationStarted {
        iteration: u32,
    },
    PromptBundleBuilt {
        bundle_ref: LoopPromptBundleRef,
        mode: PromptMode,
        surface_version: Option<CapabilitySurfaceVersion>,
        message_count: usize,
        #[serde(default)]
        skill_context: Vec<PromptSkillContextMetadata>,
    },
    ModelStarted {
        requested_model_profile_id: Option<ModelProfileId>,
    },
    ModelCompleted {
        effective_model_profile_id: ModelProfileId,
    },
    ModelFailed {
        reason_kind: AgentLoopHostErrorKind,
    },
    CapabilityInvoked {
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
    },
    CapabilityCompleted {
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        output_bytes: u64,
    },
    CapabilityFailed {
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        reason_kind: CapabilityFailureKind,
    },
    CapabilityBatchStarted {
        iteration: u32,
        call_count: u32,
        policy: BatchPolicyKind,
    },
    CapabilityBatchCompleted {
        iteration: u32,
        result_count: u32,
        denied_count: u32,
        gated_count: u32,
        failed_count: u32,
    },
    GateBlocked {
        iteration: u32,
        gate_kind: LoopGateKind,
    },
    CheckpointCreated {
        checkpoint_id: TurnCheckpointId,
        checkpoint_kind: LoopCheckpointKind,
    },
    AssistantReplyFinalized {
        message_ref: LoopMessageRef,
    },
    Blocked {
        gate_ref: LoopGateRef,
        checkpoint_id: TurnCheckpointId,
    },
    Completed {
        completion_kind: LoopCompletionKind,
        exit_id: LoopExitId,
    },
    Failed {
        reason_kind: LoopFailureKind,
        exit_id: LoopExitId,
    },
    DriverNote {
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    },
    /// A hook was dispatched at a hook point. Emitted before the hook runs.
    ///
    /// `hook_id` is the hex form of the hook's blake3-derived identity (see
    /// `ironclaw_hooks::HookId::to_hex`). The hook crate cannot be imported
    /// here without breaking the architecture-enforced dependency direction
    /// (`ironclaw_turns -> ironclaw_hooks` is forbidden), so the hook id is
    /// carried as a `String` across this seam. The hooks crate's
    /// `telemetry` module produces the value.
    HookDispatched {
        hook_id: String,
        point: String,
        trust_class: String,
        /// The extension that authored this hook, when applicable.
        /// Populated for `Installed` hooks; `None` for `Builtin`, `Trusted`,
        /// and `SelfAuthored` hooks (which have no owning extension).
        /// Carried into [`ironclaw_events::RuntimeEvent::provider`] so
        /// event-triggered subscriptions scoped to `OwnCapabilities` can
        /// match hook milestone events from the same extension.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owning_extension: Option<ExtensionId>,
    },
    /// A hook produced a decision (or explicitly passed) for a dispatch.
    HookDecisionEmitted {
        hook_id: String,
        decision: HookDecisionSummary,
        /// Audit-only free-form reason. Distinct from any reason embedded in
        /// `decision` (which is the closed-vocab, model-visible label). This
        /// field carries the manifest-supplied operator context behind a
        /// closed-vocab label like `hook_rate_limit` and flows only to
        /// audit/SSE consumers — never to the model. `None` for hooks that
        /// did not record an audit reason (Builtin/Trusted gate hooks, or
        /// any `Pass` outcome).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audit_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owning_extension: Option<ExtensionId>,
    },
    /// A hook misbehaved during dispatch. Captures the failure category and
    /// the dispatcher's disposition (fail-closed vs fail-isolated).
    HookFailed {
        hook_id: String,
        category: String,
        disposition: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owning_extension: Option<ExtensionId>,
    },
}

/// Closed-vocabulary summary of a hook decision suitable for telemetry. This
/// mirrors the shape of the hooks crate's gate decision but stringifies the
/// sanitized reason (the actual `SanitizedReason` type lives in the hooks
/// crate and cannot be imported here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookDecisionSummary {
    Allow,
    Deny { reason: String },
    PauseApproval { reason: String },
    PauseAuth { reason: String },
    Pass,
    Patch,
}

impl HookDecisionSummary {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny { .. } => "deny",
            Self::PauseApproval { .. } => "pause_approval",
            Self::PauseAuth { .. } => "pause_auth",
            Self::Pass => "pass",
            Self::Patch => "patch",
        }
    }
}

impl LoopHostMilestoneKind {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::IterationStarted { .. } => "iteration_started",
            Self::PromptBundleBuilt { .. } => "prompt_bundle_built",
            Self::ModelStarted { .. } => "model_started",
            Self::ModelCompleted { .. } => "model_completed",
            Self::ModelFailed { .. } => "model_failed",
            Self::CapabilityInvoked { .. } => "capability_invoked",
            Self::CapabilityCompleted { .. } => "capability_completed",
            Self::CapabilityFailed { .. } => "capability_failed",
            Self::CapabilityBatchStarted { .. } => "capability_batch_started",
            Self::CapabilityBatchCompleted { .. } => "capability_batch_completed",
            Self::GateBlocked { .. } => "gate_blocked",
            Self::CheckpointCreated { .. } => "checkpoint_created",
            Self::AssistantReplyFinalized { .. } => "assistant_reply_finalized",
            Self::Blocked { .. } => "blocked",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::DriverNote { .. } => "driver_note",
            Self::HookDispatched { .. } => "hook_dispatched",
            Self::HookDecisionEmitted { .. } => "hook_decision_emitted",
            Self::HookFailed { .. } => "hook_failed",
        }
    }
}

#[async_trait]
pub trait LoopHostMilestoneSink: Send + Sync {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError>;
}

/// Lightweight sink for hook-dispatcher telemetry. The hook dispatcher in
/// `ironclaw_hooks` is a process-wide shared object (`Arc<HookDispatcher>`)
/// that does not own a `LoopRunContext`. It therefore cannot construct a full
/// [`LoopHostMilestone`] on its own. Instead, the dispatcher emits the
/// hook-specific *kind* into a [`HookMilestoneSink`], and host composition in
/// `ironclaw_reborn` wraps the real [`LoopHostMilestoneSink`] in an adapter
/// that injects the active run's context before forwarding.
///
/// The kinds emitted through this sink are always one of:
/// [`LoopHostMilestoneKind::HookDispatched`],
/// [`LoopHostMilestoneKind::HookDecisionEmitted`], or
/// [`LoopHostMilestoneKind::HookFailed`]. Other variants are not valid here
/// — adapters should ignore them or treat them as a host-side bug.
#[async_trait]
pub trait HookMilestoneSink: Send + Sync {
    async fn publish_hook_milestone(&self, kind: LoopHostMilestoneKind);
}

/// Adapter that wraps a [`LoopHostMilestoneSink`] with a fixed
/// [`LoopRunContext`] and exposes the [`HookMilestoneSink`] surface. Use this
/// to plumb hook-dispatch telemetry through the same backend that receives
/// the rest of the loop's milestones.
pub struct RunScopedHookMilestoneSink {
    context: LoopRunContext,
    inner: Arc<dyn LoopHostMilestoneSink>,
}

impl RunScopedHookMilestoneSink {
    pub fn new(context: LoopRunContext, inner: Arc<dyn LoopHostMilestoneSink>) -> Self {
        Self { context, inner }
    }
}

#[async_trait]
impl HookMilestoneSink for RunScopedHookMilestoneSink {
    async fn publish_hook_milestone(&self, kind: LoopHostMilestoneKind) {
        let milestone = LoopHostMilestone::from_context(&self.context, kind);
        if let Err(error) = self.inner.publish_loop_milestone(milestone).await {
            // The dispatcher cannot meaningfully recover from a milestone-sink
            // failure (audit data is best-effort). We log and drop so hook
            // dispatch itself stays observable-only — never user-facing.
            tracing::debug!(
                error = %error.safe_summary,
                "hook milestone publish failed; dropping telemetry record"
            );
        }
    }
}

#[derive(Default)]
pub struct InMemoryLoopHostMilestoneSink {
    milestones: Mutex<Vec<LoopHostMilestone>>,
}

impl InMemoryLoopHostMilestoneSink {
    pub fn milestones(&self) -> Vec<LoopHostMilestone> {
        match self.milestones.lock() {
            Ok(milestones) => milestones.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait]
impl LoopHostMilestoneSink for InMemoryLoopHostMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        let mut milestones = self.milestones.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "loop milestone sink mutex poisoned",
            )
        })?;
        milestones.push(milestone);
        Ok(())
    }
}

/// In-memory recording sink for hook-dispatch telemetry tests. Stores every
/// emitted [`LoopHostMilestoneKind`] in publish order.
#[derive(Default)]
pub struct InMemoryHookMilestoneSink {
    kinds: Mutex<Vec<LoopHostMilestoneKind>>,
}

impl InMemoryHookMilestoneSink {
    pub fn kinds(&self) -> Vec<LoopHostMilestoneKind> {
        match self.kinds.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait]
impl HookMilestoneSink for InMemoryHookMilestoneSink {
    async fn publish_hook_milestone(&self, kind: LoopHostMilestoneKind) {
        let mut guard = match self.kinds.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.push(kind);
    }
}

#[derive(Clone)]
pub struct LoopHostMilestoneEmitter<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    context: LoopRunContext,
    sink: Arc<S>,
}

impl<S> LoopHostMilestoneEmitter<S>
where
    S: LoopHostMilestoneSink + ?Sized,
{
    pub fn new(context: LoopRunContext, sink: Arc<S>) -> Self {
        Self { context, sink }
    }

    pub async fn iteration_started(&self, iteration: u32) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::IterationStarted { iteration })
            .await
    }

    pub async fn prompt_bundle_built(
        &self,
        bundle_ref: LoopPromptBundleRef,
        mode: PromptMode,
        surface_version: Option<CapabilitySurfaceVersion>,
        message_count: usize,
        skill_context: Vec<PromptSkillContextMetadata>,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::PromptBundleBuilt {
            bundle_ref,
            mode,
            surface_version,
            message_count,
            skill_context,
        })
        .await
    }

    pub async fn model_started(
        &self,
        requested_model_profile_id: Option<ModelProfileId>,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id,
        })
        .await
    }

    pub async fn model_completed(
        &self,
        effective_model_profile_id: ModelProfileId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::ModelCompleted {
            effective_model_profile_id,
        })
        .await
    }

    pub async fn model_failed(
        &self,
        reason_kind: AgentLoopHostErrorKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::ModelFailed { reason_kind })
            .await
    }

    pub async fn capability_invoked(
        &self,
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityInvoked {
            activity_id,
            capability_id,
        })
        .await
    }

    pub async fn capability_completed(
        &self,
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        output_bytes: u64,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityCompleted {
            activity_id,
            capability_id,
            provider,
            runtime,
            output_bytes,
        })
        .await
    }

    pub async fn capability_failed(
        &self,
        activity_id: CapabilityActivityId,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        reason_kind: CapabilityFailureKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityFailed {
            activity_id,
            capability_id,
            provider,
            runtime,
            reason_kind,
        })
        .await
    }

    pub async fn capability_batch_started(
        &self,
        iteration: u32,
        call_count: u32,
        policy: BatchPolicyKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityBatchStarted {
            iteration,
            call_count,
            policy,
        })
        .await
    }

    pub async fn capability_batch_completed(
        &self,
        iteration: u32,
        result_count: u32,
        denied_count: u32,
        gated_count: u32,
        failed_count: u32,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CapabilityBatchCompleted {
            iteration,
            result_count,
            denied_count,
            gated_count,
            failed_count,
        })
        .await
    }

    pub async fn gate_blocked(
        &self,
        iteration: u32,
        gate_kind: LoopGateKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::GateBlocked {
            iteration,
            gate_kind,
        })
        .await
    }

    pub async fn checkpoint_created(
        &self,
        checkpoint_id: TurnCheckpointId,
        checkpoint_kind: LoopCheckpointKind,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::CheckpointCreated {
            checkpoint_id,
            checkpoint_kind,
        })
        .await
    }

    pub async fn assistant_reply_finalized(
        &self,
        message_ref: LoopMessageRef,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::AssistantReplyFinalized { message_ref })
            .await
    }

    pub async fn blocked(
        &self,
        gate_ref: LoopGateRef,
        checkpoint_id: TurnCheckpointId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::Blocked {
            gate_ref,
            checkpoint_id,
        })
        .await
    }

    pub async fn completed(
        &self,
        completion_kind: LoopCompletionKind,
        exit_id: LoopExitId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::Completed {
            completion_kind,
            exit_id,
        })
        .await
    }

    pub async fn failed(
        &self,
        reason_kind: LoopFailureKind,
        exit_id: LoopExitId,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::Failed {
            reason_kind,
            exit_id,
        })
        .await
    }

    pub async fn driver_note(
        &self,
        kind: LoopDriverNoteKind,
        safe_summary: LoopSafeSummary,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::DriverNote { kind, safe_summary })
            .await
    }

    pub async fn hook_dispatched(
        &self,
        hook_id: String,
        point: String,
        trust_class: String,
        owning_extension: Option<ExtensionId>,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::HookDispatched {
            hook_id,
            point,
            trust_class,
            owning_extension,
        })
        .await
    }

    pub async fn hook_decision_emitted(
        &self,
        hook_id: String,
        decision: HookDecisionSummary,
        audit_reason: Option<String>,
        owning_extension: Option<ExtensionId>,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id,
            decision,
            audit_reason,
            owning_extension,
        })
        .await
    }

    pub async fn hook_failed(
        &self,
        hook_id: String,
        category: String,
        disposition: String,
        owning_extension: Option<ExtensionId>,
    ) -> Result<(), AgentLoopHostError> {
        self.publish(LoopHostMilestoneKind::HookFailed {
            hook_id,
            category,
            disposition,
            owning_extension,
        })
        .await
    }

    async fn publish(&self, kind: LoopHostMilestoneKind) -> Result<(), AgentLoopHostError> {
        self.sink
            .publish_loop_milestone(LoopHostMilestone::from_context(&self.context, kind))
            .await
    }
}

#[cfg(test)]
mod hook_milestone_schema_snapshots {
    //! L3 schema-snapshot tests for hook milestone variants.
    //!
    //! These tests pin the JSON wire shape of each hook-related
    //! [`LoopHostMilestoneKind`] variant against a frozen string fixture.
    //! Downstream consumers (audit trails, trace replay, external dashboards)
    //! parse this JSON; an accidental field rename, enum-tag rename, or type
    //! change would silently break them. If any of these tests fail, the wire
    //! format has changed — verify every consumer has been updated before
    //! re-pinning the fixture.
    //!
    //! Fixtures are inlined as `&str` constants so a reviewer can read the
    //! exact shape being pinned in the diff. We compare against
    //! [`serde_json::to_string_pretty`] output to keep the fixtures legible.
    use super::{HookDecisionSummary, LoopHostMilestoneKind};

    fn pretty(kind: &LoopHostMilestoneKind) -> String {
        match serde_json::to_string_pretty(kind) {
            Ok(s) => s,
            Err(e) => panic!("failed to serialize milestone kind for snapshot: {e}"),
        }
    }

    #[test]
    fn hook_dispatched_milestone_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookDispatched {
            hook_id: "abcdef0123456789".to_string(),
            point: "before_capability".to_string(),
            trust_class: "installed".to_string(),
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_dispatched": {
    "hook_id": "abcdef0123456789",
    "point": "before_capability",
    "trust_class": "installed"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_decision_emitted_allow_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: "abcdef0123456789".to_string(),
            decision: HookDecisionSummary::Allow,
            audit_reason: None,
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_decision_emitted": {
    "hook_id": "abcdef0123456789",
    "decision": "allow"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_decision_emitted_deny_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: "abcdef0123456789".to_string(),
            decision: HookDecisionSummary::Deny {
                reason: "blocked by policy".to_string(),
            },
            audit_reason: None,
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_decision_emitted": {
    "hook_id": "abcdef0123456789",
    "decision": {
      "deny": {
        "reason": "blocked by policy"
      }
    }
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_decision_emitted_pause_approval_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: "abcdef0123456789".to_string(),
            decision: HookDecisionSummary::PauseApproval {
                reason: "user approval required".to_string(),
            },
            audit_reason: None,
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_decision_emitted": {
    "hook_id": "abcdef0123456789",
    "decision": {
      "pause_approval": {
        "reason": "user approval required"
      }
    }
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_decision_emitted_pause_auth_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: "abcdef0123456789".to_string(),
            decision: HookDecisionSummary::PauseAuth {
                reason: "re-authentication required".to_string(),
            },
            audit_reason: None,
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_decision_emitted": {
    "hook_id": "abcdef0123456789",
    "decision": {
      "pause_auth": {
        "reason": "re-authentication required"
      }
    }
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_decision_emitted_pass_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: "abcdef0123456789".to_string(),
            decision: HookDecisionSummary::Pass,
            audit_reason: None,
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_decision_emitted": {
    "hook_id": "abcdef0123456789",
    "decision": "pass"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_decision_emitted_patch_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookDecisionEmitted {
            hook_id: "abcdef0123456789".to_string(),
            decision: HookDecisionSummary::Patch,
            audit_reason: None,
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_decision_emitted": {
    "hook_id": "abcdef0123456789",
    "decision": "patch"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_failed_timeout_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookFailed {
            hook_id: "abcdef0123456789".to_string(),
            category: "timeout".to_string(),
            disposition: "fail_closed".to_string(),
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_failed": {
    "hook_id": "abcdef0123456789",
    "category": "timeout",
    "disposition": "fail_closed"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_failed_panic_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookFailed {
            hook_id: "abcdef0123456789".to_string(),
            category: "panic".to_string(),
            disposition: "fail_closed".to_string(),
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_failed": {
    "hook_id": "abcdef0123456789",
    "category": "panic",
    "disposition": "fail_closed"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_failed_malformed_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookFailed {
            hook_id: "abcdef0123456789".to_string(),
            category: "malformed".to_string(),
            disposition: "fail_closed".to_string(),
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_failed": {
    "hook_id": "abcdef0123456789",
    "category": "malformed",
    "disposition": "fail_closed"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }

    #[test]
    fn hook_failed_attenuation_violation_serialization_is_stable() {
        let value = LoopHostMilestoneKind::HookFailed {
            hook_id: "abcdef0123456789".to_string(),
            category: "attenuation_violation".to_string(),
            disposition: "fail_isolated".to_string(),
            owning_extension: None,
        };
        const EXPECTED: &str = r#"{
  "hook_failed": {
    "hook_id": "abcdef0123456789",
    "category": "attenuation_violation",
    "disposition": "fail_isolated"
  }
}"#;
        assert_eq!(pretty(&value), EXPECTED);
    }
}
