//! `DefaultPlanner` — the reference composition of the built-in strategies.
//!
//! Construction is crate-private. Public callers get a sealed
//! `AgentLoopPlanner` through `families::*` and `LoopFamilyRegistry`; they
//! cannot instantiate or mutate planner strategy slots.

use std::sync::Arc;

use crate::families::DEFAULT_FAMILY_DIGEST;
use crate::family::{ComponentIdentity, LoopFamilyId};
use crate::planner::{AgentLoopPlanner, AgentLoopPlannerInternal};
use crate::strategies::{
    ActiveTaskPreservingCompactionStrategy, BatchPolicyStrategy, BudgetStrategy,
    CapabilityStrategy, CompactionStrategy, ContextStrategy, DefaultBatchPolicyStrategy,
    DefaultBudgetStrategy, DefaultCapabilityStrategy, DefaultContextStrategy,
    DefaultGateHandlingStrategy, DefaultInputDrainStrategy, DefaultModelStrategy,
    DefaultRecoveryStrategy, DefaultReplyAdmissionStrategy, DefaultStopConditionStrategy,
    GateHandlingStrategy, InputDrainStrategy, ModelStrategy, RecoveryStrategy,
    ReplyAdmissionStrategy, StopConditionStrategy,
};

/// The reference planner: a concrete, Builtin-only strategy composition.
#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct DefaultPlanner {
    id: LoopFamilyId,
    version: ComponentIdentity,
    context: Arc<dyn ContextStrategy>,
    compaction: Arc<dyn CompactionStrategy>,
    capability: Arc<dyn CapabilityStrategy>,
    model: Arc<dyn ModelStrategy>,
    batch: Arc<dyn BatchPolicyStrategy>,
    gate: Arc<dyn GateHandlingStrategy>,
    recovery: Arc<dyn RecoveryStrategy>,
    reply_admission: Arc<dyn ReplyAdmissionStrategy>,
    stop: Arc<dyn StopConditionStrategy>,
    drain: Arc<dyn InputDrainStrategy>,
    budget: Arc<dyn BudgetStrategy>,
}

#[allow(dead_code)]
impl DefaultPlanner {
    /// Crate-private constructor for the all-default strategy composition.
    pub(crate) fn compose_default() -> Self {
        Self::compose(
            LoopFamilyId::DEFAULT,
            ComponentIdentity::from_static("default", DEFAULT_FAMILY_DIGEST),
            DefaultStrategySlots::default(),
        )
    }

    /// Crate-private constructor used by future family factories that need to
    /// provide an already-selected strategy set.
    pub(crate) fn compose(
        id: LoopFamilyId,
        version: ComponentIdentity,
        slots: DefaultStrategySlots,
    ) -> Self {
        Self {
            id,
            version,
            context: slots.context,
            compaction: slots.compaction,
            capability: slots.capability,
            model: slots.model,
            batch: slots.batch,
            gate: slots.gate,
            recovery: slots.recovery,
            reply_admission: slots.reply_admission,
            stop: slots.stop,
            drain: slots.drain,
            budget: slots.budget,
        }
    }

    pub(crate) fn with_id(mut self, id: LoopFamilyId) -> Self {
        self.id = id;
        self
    }

    pub(crate) fn with_version(mut self, version: ComponentIdentity) -> Self {
        self.version = version;
        self
    }

    pub(crate) fn with_context(mut self, strategy: Arc<dyn ContextStrategy>) -> Self {
        self.context = strategy;
        self
    }

    pub(crate) fn with_compaction(mut self, strategy: Arc<dyn CompactionStrategy>) -> Self {
        self.compaction = strategy;
        self
    }

    pub(crate) fn with_capability(mut self, strategy: Arc<dyn CapabilityStrategy>) -> Self {
        self.capability = strategy;
        self
    }

    pub(crate) fn with_model(mut self, strategy: Arc<dyn ModelStrategy>) -> Self {
        self.model = strategy;
        self
    }

    pub(crate) fn with_batch(mut self, strategy: Arc<dyn BatchPolicyStrategy>) -> Self {
        self.batch = strategy;
        self
    }

    pub(crate) fn with_gate(mut self, strategy: Arc<dyn GateHandlingStrategy>) -> Self {
        self.gate = strategy;
        self
    }

    pub(crate) fn with_recovery(mut self, strategy: Arc<dyn RecoveryStrategy>) -> Self {
        self.recovery = strategy;
        self
    }

    pub(crate) fn with_reply_admission(
        mut self,
        strategy: Arc<dyn ReplyAdmissionStrategy>,
    ) -> Self {
        self.reply_admission = strategy;
        self
    }

    pub(crate) fn with_stop(mut self, strategy: Arc<dyn StopConditionStrategy>) -> Self {
        self.stop = strategy;
        self
    }

    pub(crate) fn with_drain(mut self, strategy: Arc<dyn InputDrainStrategy>) -> Self {
        self.drain = strategy;
        self
    }

    pub(crate) fn with_budget(mut self, strategy: Arc<dyn BudgetStrategy>) -> Self {
        self.budget = strategy;
        self
    }
}

impl AgentLoopPlanner for DefaultPlanner {
    fn id(&self) -> &LoopFamilyId {
        &self.id
    }

    fn version(&self) -> &ComponentIdentity {
        &self.version
    }
}

impl AgentLoopPlannerInternal for DefaultPlanner {
    fn context(&self) -> &dyn ContextStrategy {
        &*self.context
    }

    fn compaction(&self) -> &dyn CompactionStrategy {
        &*self.compaction
    }

    fn capability(&self) -> &dyn CapabilityStrategy {
        &*self.capability
    }

    fn model(&self) -> &dyn ModelStrategy {
        &*self.model
    }

    fn batch(&self) -> &dyn BatchPolicyStrategy {
        &*self.batch
    }

    fn gate(&self) -> &dyn GateHandlingStrategy {
        &*self.gate
    }

    fn recovery(&self) -> &dyn RecoveryStrategy {
        &*self.recovery
    }

    fn reply_admission(&self) -> &dyn ReplyAdmissionStrategy {
        &*self.reply_admission
    }

    fn stop(&self) -> &dyn StopConditionStrategy {
        &*self.stop
    }

    fn drain(&self) -> &dyn InputDrainStrategy {
        &*self.drain
    }

    fn budget(&self) -> &dyn BudgetStrategy {
        &*self.budget
    }
}

/// Strategy slots accepted by `DefaultPlanner::compose`.
///
/// The type is crate-private so future family factories can compose slot sets
/// without making strategy traits constructible outside this crate.
pub(crate) struct DefaultStrategySlots {
    context: Arc<dyn ContextStrategy>,
    compaction: Arc<dyn CompactionStrategy>,
    capability: Arc<dyn CapabilityStrategy>,
    model: Arc<dyn ModelStrategy>,
    batch: Arc<dyn BatchPolicyStrategy>,
    gate: Arc<dyn GateHandlingStrategy>,
    recovery: Arc<dyn RecoveryStrategy>,
    reply_admission: Arc<dyn ReplyAdmissionStrategy>,
    stop: Arc<dyn StopConditionStrategy>,
    drain: Arc<dyn InputDrainStrategy>,
    budget: Arc<dyn BudgetStrategy>,
}

impl Default for DefaultStrategySlots {
    fn default() -> Self {
        Self {
            context: Arc::new(DefaultContextStrategy::default()),
            compaction: Arc::new(ActiveTaskPreservingCompactionStrategy::default()),
            capability: Arc::new(DefaultCapabilityStrategy),
            model: Arc::new(DefaultModelStrategy),
            batch: Arc::new(DefaultBatchPolicyStrategy),
            gate: Arc::new(DefaultGateHandlingStrategy),
            recovery: Arc::new(DefaultRecoveryStrategy::default()),
            reply_admission: Arc::new(DefaultReplyAdmissionStrategy),
            stop: Arc::new(DefaultStopConditionStrategy::default()),
            drain: Arc::new(DefaultInputDrainStrategy),
            budget: Arc::new(DefaultBudgetStrategy::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
        run_profile::{
            CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
            ConcurrencyClass, ContextProfileId, LoopDriverId, LoopPromptBundleRequest,
            LoopRunContext, ModelProfileId, PromptContextTokenBudget, PromptMode,
            RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy,
            ResourceBudgetTier, RunClassId, RunProfileFingerprint, RuntimeProfileConstraints,
            SchedulingClass, SteeringPolicy,
        },
    };

    use crate::family::{ComponentDigest, LoopFamilyId};
    use crate::state::{
        CompactionPromptSnapshot, IndexedMessageKind, LoopExecutionState, MessageIndexEntry,
    };
    use crate::strategies::{
        ActiveTaskPreservingCompactionStrategy, BatchPolicy, CapabilityFilter, CompactionDecision,
        ContextPlan, ContextStrategy, DefaultCompactionStrategy,
    };

    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_clone<T: Clone>() {}

    #[allow(dead_code)]
    fn _check(_: &dyn AgentLoopPlanner) {}

    #[test]
    fn default_planner_is_send_sync_and_clone() {
        assert_send_sync::<DefaultPlanner>();
        assert_clone::<DefaultPlanner>();
    }

    #[test]
    fn compose_default_uses_default_family_identity() {
        let planner = DefaultPlanner::compose_default();

        assert_eq!(planner.id(), &LoopFamilyId::DEFAULT);
        assert_eq!(planner.version().id, "default");
        assert_eq!(planner.version().digest, DEFAULT_FAMILY_DIGEST);
    }

    #[tokio::test]
    async fn builder_chain_overrides_identity_and_context() {
        #[derive(Default)]
        struct CustomContext;

        #[async_trait]
        impl ContextStrategy for CustomContext {
            async fn plan_context_request(&self, _state: &LoopExecutionState) -> ContextPlan {
                ContextPlan {
                    request: LoopPromptBundleRequest {
                        mode: PromptMode::TextOnly,
                        context_cursor: None,
                        surface_version: None,
                        checkpoint_state_ref: None,
                        max_messages: Some(7),
                        inline_messages: Vec::new(),
                        capability_view: None,
                    },
                    emitted_admission_control: false,
                    emitted_repeated_call_warning: false,
                }
            }
        }

        let id = LoopFamilyId::new("custom").expect("valid custom family id");
        let version = ComponentIdentity::from_static("custom", ComponentDigest([1; 32]));
        let planner = DefaultPlanner::compose_default()
            .with_id(id.clone())
            .with_version(version.clone())
            .with_context(Arc::new(CustomContext));

        assert_eq!(planner.id(), &id);
        assert_eq!(planner.version(), &version);

        let state = LoopExecutionState::initial_for_run(&test_run_context());
        let request = planner.context().plan_context_request(&state).await;
        assert_eq!(request.request.max_messages, Some(7));
    }

    #[tokio::test]
    async fn crate_private_internal_accessors_are_wired() {
        let planner = DefaultPlanner::compose_default();
        let state = LoopExecutionState::initial_for_run(&test_run_context());

        assert_eq!(planner.budget().iteration_limit(&state), 32);
        assert_eq!(planner.batch().policy(&state, &[]), BatchPolicy::Parallel);

        let filter = planner.capability().filter(&state).await;
        assert_eq!(filter, CapabilityFilter::All);
    }

    #[test]
    fn builder_chain_overrides_compaction_strategy() {
        let planner = DefaultPlanner::compose_default().with_compaction(Arc::new(
            ActiveTaskPreservingCompactionStrategy::from(DefaultCompactionStrategy {
                prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }),
        ));
        let context = test_run_context();
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 5,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 6,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 7,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 8,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
        ]);

        assert_eq!(
            planner.compaction().should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 5,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }

    #[allow(clippy::too_many_lines)]
    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-default-planner").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-planner").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("default_planner_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("default_planner_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("default_planner_test_class").expect("valid"),
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
            model_profile_id: ModelProfileId::new("default_planner_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "default_planner_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("default_planner_test_context")
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
                tier: ResourceBudgetTier::new("default_planner_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("default-planner-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }
}
