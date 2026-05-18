use async_trait::async_trait;
use ironclaw_turns::run_profile::{LoopPromptBundleRequest, PromptMode};

use crate::state::LoopExecutionState;

/// Decides what context the host should materialize for the next model call.
///
/// Pure policy: returns the request value the executor will pass to
/// `LoopPromptPort::build_prompt_bundle`. Does NOT mutate state.
///
/// Inline messages flow through the `inline_messages` field of
/// `LoopPromptBundleRequest`. There is no separate nudge strategy; loop
/// families that need nudges extend their context strategy to populate this
/// field from `state`.
#[async_trait]
pub(crate) trait ContextStrategy: Send + Sync {
    async fn plan_context_request(&self, state: &LoopExecutionState) -> LoopPromptBundleRequest;
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn ContextStrategy) {}

/// Reference baseline `ContextStrategy` implementation.
///
/// Requests `PromptMode::TextOnly` with at most [`Self::DEFAULT_MAX_MESSAGES`]
/// transcript messages and no inline nudges. Loop families that want
/// CodeAct-shaped prompts or want to inject nudges swap this strategy
/// rather than mutating state.
#[derive(Debug, Clone, Copy)]
pub struct DefaultContextStrategy {
    /// Max messages to ask the host to include in the bundle. Default
    /// [`Self::DEFAULT_MAX_MESSAGES`].
    pub max_messages: u32,
}

impl DefaultContextStrategy {
    /// Default ceiling on transcript messages requested per turn.
    pub const DEFAULT_MAX_MESSAGES: u32 = 16;
}

impl Default for DefaultContextStrategy {
    fn default() -> Self {
        Self {
            max_messages: Self::DEFAULT_MAX_MESSAGES,
        }
    }
}

#[async_trait]
impl ContextStrategy for DefaultContextStrategy {
    async fn plan_context_request(&self, _state: &LoopExecutionState) -> LoopPromptBundleRequest {
        // `max(1)` keeps the host's "zero is rejected" invariant from
        // `LoopPromptBundleRequest` even if a loop family overrides
        // `max_messages` to zero by accident.
        LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(self.max_messages.max(1)),
            inline_messages: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
        run_profile::{
            CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
            ConcurrencyClass, ContextProfileId, LoopDriverId, LoopRunContext, ModelProfileId,
            PromptMode, RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy,
            ResourceBudgetTier, RunClassId, RunProfileFingerprint, RuntimeProfileConstraints,
            SchedulingClass, SteeringPolicy,
        },
    };

    use super::{ContextStrategy, DefaultContextStrategy};
    use crate::state::LoopExecutionState;

    #[allow(dead_code)]
    fn _check(_: &dyn ContextStrategy) {}

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-default-context").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-context").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_context_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_context_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_context_test_class").expect("valid"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor
                .checkpoint_schema_id
                .clone()
                .expect("descriptor checkpoint id"),
            checkpoint_schema_version: descriptor
                .checkpoint_schema_version
                .expect("descriptor checkpoint version"),
            model_profile_id: ModelProfileId::new("default_context_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_context_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_context_test_context")
                .expect("valid"),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: false,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("default_context_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("default-context-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    #[test]
    fn default_max_messages_is_sixteen() {
        assert_eq!(DefaultContextStrategy::default().max_messages, 16);
    }

    #[tokio::test]
    async fn plan_context_request_returns_text_only_bundle() {
        let strategy = DefaultContextStrategy::default();
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        let request = strategy.plan_context_request(&state).await;

        assert_eq!(request.mode, PromptMode::TextOnly);
        assert_eq!(request.max_messages, Some(16));
        assert!(request.inline_messages.is_empty());
        assert!(request.context_cursor.is_none());
        assert!(request.surface_version.is_none());
        assert!(request.checkpoint_state_ref.is_none());
    }

    #[tokio::test]
    async fn plan_context_request_clamps_zero_to_one() {
        let strategy = DefaultContextStrategy { max_messages: 0 };
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        let request = strategy.plan_context_request(&state).await;

        assert_eq!(request.max_messages, Some(1));
    }
}
