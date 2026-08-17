use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use async_trait::async_trait;
use ironclaw_event_log::{
    MAX_RUNTIME_EVENT_DURATION_MS, NonBlockingEventSink, RuntimeEvent, RuntimeEventId,
};
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, InvocationId, MissionId, ProjectId, TenantId, ThreadId, UserId},
    resource::ResourceScope,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, HookDecisionSummary, LoopHostMilestone,
    LoopHostMilestoneKind, LoopHostMilestoneSink,
};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::TurnRunId;

const MODEL_CAPABILITY_ID: &str = "loop.model";
const ASSISTANT_REPLY_CAPABILITY_ID: &str = "loop.assistant_reply";
const LOOP_RUN_CAPABILITY_ID: &str = "loop.run";
const HOOK_CAPABILITY_ID: &str = "loop.hook";
const RECOVERY_CAPABILITY_ID: &str = "loop.recovery";
const RECOVERY_EVENT_ID_DOMAIN: &[u8] = b"ironclaw:loop-recovery-event:v1";

fn recovery_event_id(run_id: TurnRunId, sequence: u64) -> RuntimeEventId {
    // Purpose: stable logical deduplication across the event-append/checkpoint
    // interruption window. This is identity derivation, not authentication.
    let mut hasher = blake3::Hasher::new();
    hasher.update(RECOVERY_EVENT_ID_DOMAIN);
    hasher.update(run_id.as_uuid().as_bytes());
    hasher.update(&sequence.to_be_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    // Mark the value as an RFC 9562 UUIDv8 while preserving the derived bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    RuntimeEventId::from_bytes(bytes)
}

/// Scope authority bound into the sink at construction time.
///
/// Building this from a canonical thread scope prevents callers from stitching
/// runtime events together from an unrelated user or mission scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableLoopHostMilestoneScope {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
    mission_id: Option<MissionId>,
    thread_id: Option<ThreadId>,
    run_id: Option<TurnRunId>,
}

impl DurableLoopHostMilestoneScope {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        mission_id: Option<MissionId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
            mission_id,
            thread_id: None,
            run_id: None,
        }
    }

    pub fn from_thread_scope(thread_scope: &ThreadScope) -> Result<Self, AgentLoopHostError> {
        let Some(user_id) = thread_scope.owner_user_id.clone() else {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "loop milestone event scope requires a thread owner user",
            ));
        };
        Ok(Self {
            tenant_id: thread_scope.tenant_id.clone(),
            user_id,
            agent_id: Some(thread_scope.agent_id.clone()),
            project_id: thread_scope.project_id.clone(),
            mission_id: thread_scope.mission_id.clone(),
            thread_id: None,
            run_id: None,
        })
    }

    pub fn from_thread_scope_for_run(
        thread_scope: &ThreadScope,
        thread_id: ThreadId,
        run_id: TurnRunId,
    ) -> Result<Self, AgentLoopHostError> {
        let mut scope = Self::from_thread_scope(thread_scope)?;
        scope.thread_id = Some(thread_id);
        scope.run_id = Some(run_id);
        Ok(scope)
    }

    fn resource_scope(
        &self,
        milestone: &LoopHostMilestone,
    ) -> Result<ResourceScope, AgentLoopHostError> {
        if milestone.scope.tenant_id != self.tenant_id
            || milestone.scope.agent_id != self.agent_id
            || milestone.scope.project_id != self.project_id
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "loop milestone scope does not match durable event scope",
            ));
        }
        match &self.thread_id {
            Some(thread_id) if milestone.scope.thread_id != *thread_id => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::ScopeMismatch,
                    "loop milestone thread does not match durable event scope",
                ));
            }
            _ => {}
        }
        match &self.run_id {
            Some(run_id) if milestone.run_id != *run_id => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::ScopeMismatch,
                    "loop milestone run does not match durable event scope",
                ));
            }
            _ => {}
        }
        Ok(ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            project_id: self.project_id.clone(),
            mission_id: self.mission_id.clone(),
            thread_id: Some(milestone.scope.thread_id.clone()),
            invocation_id: InvocationId::from_uuid(milestone.run_id.as_uuid()),
        })
    }
}

/// Durable projection adapter for public AgentLoopHost milestones.
///
/// The adapter writes only metadata-only loop lifecycle milestones into the
/// runtime event log. Progress milestones that carry useful counters or typed
/// checkpoint/prompt metadata stay in the milestone-sink substrate rather than
/// being collapsed into lossy `RuntimeEvent` rows. Raw prompts, assistant
/// content, provider errors, message refs, host paths, and secrets stay in
/// their owning stores and never enter runtime events.
#[derive(Clone)]
pub struct DurableLoopHostMilestoneSink {
    event_sink: Arc<dyn NonBlockingEventSink>,
    scope: DurableLoopHostMilestoneScope,
    model_started_at: Arc<Mutex<HashMap<TurnRunId, Instant>>>,
}

impl DurableLoopHostMilestoneSink {
    pub fn new(
        event_sink: Arc<dyn NonBlockingEventSink>,
        scope: DurableLoopHostMilestoneScope,
    ) -> Self {
        Self {
            event_sink,
            scope,
            model_started_at: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn event_sink(&self) -> Arc<dyn NonBlockingEventSink> {
        Arc::clone(&self.event_sink)
    }

    fn note_model_started(&self, run_id: TurnRunId) {
        match self.model_started_at.lock() {
            Ok(mut started) => {
                started.insert(run_id, Instant::now());
            }
            Err(poisoned) => {
                poisoned.into_inner().insert(run_id, Instant::now());
            }
        }
    }

    fn take_model_duration_ms(&self, run_id: TurnRunId) -> Option<u64> {
        let started = match self.model_started_at.lock() {
            Ok(mut started) => started.remove(&run_id),
            Err(poisoned) => poisoned.into_inner().remove(&run_id),
        };
        started.map(|started| {
            u64::try_from(started.elapsed().as_millis())
                .unwrap_or(u64::MAX)
                .min(MAX_RUNTIME_EVENT_DURATION_MS)
        })
    }

    fn forget_model_started(&self, run_id: TurnRunId) {
        match self.model_started_at.lock() {
            Ok(mut started) => {
                started.remove(&run_id);
            }
            Err(poisoned) => {
                poisoned.into_inner().remove(&run_id);
            }
        }
    }

    fn resource_scope(
        &self,
        milestone: &LoopHostMilestone,
    ) -> Result<ResourceScope, AgentLoopHostError> {
        self.scope.resource_scope(milestone)
    }
}

impl std::fmt::Debug for DurableLoopHostMilestoneSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableLoopHostMilestoneSink")
            .field("event_sink", &"<event_sink>")
            .field("scope", &self.scope)
            .finish()
    }
}

#[async_trait]
impl LoopHostMilestoneSink for DurableLoopHostMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        let Some(event) = self.runtime_event_for_milestone(&milestone)? else {
            return Ok(());
        };
        if let Err(error) = self.event_sink.try_emit(event) {
            tracing::debug!(
                error = %error,
                "loop milestone runtime event was not emitted"
            );
        }
        Ok(())
    }
}

impl DurableLoopHostMilestoneSink {
    fn runtime_event_for_milestone(
        &self,
        milestone: &LoopHostMilestone,
    ) -> Result<Option<RuntimeEvent>, AgentLoopHostError> {
        let scope = self.resource_scope(milestone)?;
        let event = match &milestone.kind {
            LoopHostMilestoneKind::ModelStarted { .. } => {
                self.note_model_started(milestone.run_id);
                return Ok(None);
            }
            LoopHostMilestoneKind::ModelCompleted { .. } => {
                let capability_id = capability_id(MODEL_CAPABILITY_ID)?;
                match self.take_model_duration_ms(milestone.run_id) {
                    Some(duration_ms) => RuntimeEvent::model_completed_with_duration(
                        scope,
                        capability_id,
                        duration_ms,
                    ),
                    None => RuntimeEvent::model_completed(scope, capability_id),
                }
            }
            LoopHostMilestoneKind::ModelFailed { reason_kind } => {
                let capability_id = capability_id(MODEL_CAPABILITY_ID)?;
                match self.take_model_duration_ms(milestone.run_id) {
                    Some(duration_ms) => RuntimeEvent::model_failed_with_duration(
                        scope,
                        capability_id,
                        reason_kind.as_str(),
                        duration_ms,
                    ),
                    None => RuntimeEvent::model_failed(scope, capability_id, reason_kind.as_str()),
                }
            }
            LoopHostMilestoneKind::CapabilityInvoked {
                activity_id,
                capability_id,
            } => {
                let mut scope = scope;
                scope.invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
                let mut event =
                    RuntimeEvent::capability_activity_requested(scope, capability_id.clone());
                event.parent_invocation_id =
                    Some(InvocationId::from_uuid(milestone.run_id.as_uuid()));
                event
            }
            LoopHostMilestoneKind::CapabilityCompleted {
                activity_id,
                capability_id,
                provider,
                runtime,
                output_bytes,
            } => {
                let mut scope = scope;
                scope.invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
                let mut event = RuntimeEvent::capability_activity_succeeded(
                    scope,
                    capability_id.clone(),
                    provider.clone(),
                    *runtime,
                    *output_bytes,
                );
                event.parent_invocation_id =
                    Some(InvocationId::from_uuid(milestone.run_id.as_uuid()));
                event
            }
            LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id,
                provider,
                runtime,
                reason_kind,
                safe_summary,
            } => {
                let mut scope = scope;
                scope.invocation_id = InvocationId::from_uuid(activity_id.as_uuid());
                let mut event = RuntimeEvent::capability_activity_failed(
                    scope,
                    capability_id.clone(),
                    provider.clone(),
                    *runtime,
                    reason_kind.as_str(),
                );
                if let Some(summary) = safe_summary {
                    event = event.with_error_summary(summary.as_str());
                }
                event.parent_invocation_id =
                    Some(InvocationId::from_uuid(milestone.run_id.as_uuid()));
                event
            }
            LoopHostMilestoneKind::FailureRecovered {
                sequence,
                stage,
                class,
                disposition,
            } => RuntimeEvent::failure_recovered(
                recovery_event_id(milestone.run_id, *sequence),
                scope,
                capability_id(RECOVERY_CAPABILITY_ID)?,
                stage.as_str(),
                class.as_str(),
                disposition.as_str(),
            ),
            LoopHostMilestoneKind::AssistantReplyFinalized { .. } => {
                RuntimeEvent::assistant_reply_finalized(
                    scope,
                    capability_id(ASSISTANT_REPLY_CAPABILITY_ID)?,
                )
            }
            LoopHostMilestoneKind::Completed { .. } => {
                self.forget_model_started(milestone.run_id);
                RuntimeEvent::loop_completed(scope, capability_id(LOOP_RUN_CAPABILITY_ID)?)
            }
            LoopHostMilestoneKind::Failed { reason_kind, .. } => {
                self.forget_model_started(milestone.run_id);
                RuntimeEvent::loop_failed(
                    scope,
                    capability_id(LOOP_RUN_CAPABILITY_ID)?,
                    reason_kind.as_str(),
                )
            }
            LoopHostMilestoneKind::Blocked { .. } => {
                self.forget_model_started(milestone.run_id);
                return Ok(None);
            }
            // Hook telemetry is projected into the durable event log so audit
            // consumers can replay the same hook dispatched/decision/failed
            // trail that SSE observers see live. Only closed-vocabulary labels
            // and the blake3-hex hook identity cross into the event;
            // sanitized reasons stay in the hook milestone stream and do not
            // enter durable storage through this seam.
            LoopHostMilestoneKind::HookDispatched {
                hook_id,
                point,
                trust_class,
                owning_extension,
            } => RuntimeEvent::hook_dispatched(
                scope,
                capability_id(HOOK_CAPABILITY_ID)?,
                hook_id.clone(),
                point.clone(),
                trust_class.clone(),
                owning_extension.clone(),
            ),
            LoopHostMilestoneKind::HookDecisionEmitted {
                hook_id,
                decision,
                owning_extension,
                // `audit_reason` is intentionally NOT projected into the
                // durable event log: durable events are model-visible audit
                // surface; the free-form manifest reason is operator-visible
                // SSE/audit content delivered via the in-memory milestone
                // sink, not the cross-process event channel.
                audit_reason: _,
            } => RuntimeEvent::hook_decision_emitted(
                scope,
                capability_id(HOOK_CAPABILITY_ID)?,
                hook_id.clone(),
                hook_decision_label(decision),
                owning_extension.clone(),
            ),
            LoopHostMilestoneKind::HookFailed {
                hook_id,
                category,
                disposition,
                owning_extension,
            } => RuntimeEvent::hook_failed(
                scope,
                capability_id(HOOK_CAPABILITY_ID)?,
                hook_id.clone(),
                category.clone(),
                disposition.clone(),
                owning_extension.clone(),
            ),
            // PromptBundleBuilt and CheckpointCreated are suppressed here intentionally.
            // Checkpoint durability is owned by LoopCheckpointPort::write_checkpoint; the
            // CheckpointCreated runtime-event milestone is emitted there with the authoritative
            // durable payload. The CheckpointWritten progress event is an advisory echo only —
            // emitting it here would create a duplicate weaker record. Resume must rely
            // on the checkpoint-port milestone, NOT this progress echo.
            // Similarly, PromptBundleBuilt is emitted by LoopPromptPort with richer context.
            LoopHostMilestoneKind::IterationStarted { .. }
            | LoopHostMilestoneKind::PromptBundleBuilt { .. }
            | LoopHostMilestoneKind::ModelReasoningDelta { .. }
            | LoopHostMilestoneKind::ModelTextDelta { .. }
            | LoopHostMilestoneKind::CapabilityBatchStarted { .. }
            | LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
            | LoopHostMilestoneKind::GateBlocked { .. }
            | LoopHostMilestoneKind::CheckpointCreated { .. }
            | LoopHostMilestoneKind::CompactionStarted { .. }
            | LoopHostMilestoneKind::CompactionCompleted { .. }
            | LoopHostMilestoneKind::CompactionFailed { .. }
            | LoopHostMilestoneKind::CompactionLeakDetected { .. }
            | LoopHostMilestoneKind::DriverNote { .. } => return Ok(None),
        };
        Ok(Some(event))
    }
}

/// Render a [`HookDecisionSummary`] as the closed-vocabulary kind label
/// expected by [`RuntimeEvent::hook_decision_emitted`]. Sanitized reasons live
/// in the in-memory hook milestone stream only — durable runtime events carry
/// the kind label alone so that audit replay never depends on free-form reason
/// text.
fn hook_decision_label(decision: &HookDecisionSummary) -> &'static str {
    decision.kind_name()
}

fn capability_id(value: &'static str) -> Result<CapabilityId, AgentLoopHostError> {
    CapabilityId::new(value).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "loop milestone event capability id is invalid",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_event_log::{
        DurableEventLog, EventCursor, EventError, EventLogEntry, EventReplay, EventSink,
        EventStreamKey, InMemoryDurableEventLog, InMemoryEventSink, ReadScope, RuntimeEventKind,
    };
    use ironclaw_event_store::{CoalescingEventSink, EventBatchConfig};
    use ironclaw_host_api::{
        ids::{AgentId, ExtensionId, InvocationId, ProjectId, TenantId, ThreadId, UserId},
        result_meta::FailureKind,
        runtime::RuntimeKind,
        turn::{LoopExitId, LoopGateRef, TurnCheckpointId},
    };
    use ironclaw_loop_contracts::{
        HookDecisionSummary, LoopCompletionKind, LoopDriverId, LoopFailureKind, LoopHostMilestone,
        LoopRecoveryClass, LoopRecoveryDisposition, LoopRecoveryStage, LoopSafeSummary,
    };
    use ironclaw_threads::ThreadScope;
    use ironclaw_turns::{CapabilityActivityId, TurnId, TurnScope};
    use tokio::sync::Semaphore;

    struct StalledEventLog {
        inner: InMemoryDurableEventLog,
        append_started: Semaphore,
        append_release: Semaphore,
    }

    impl StalledEventLog {
        fn new() -> Self {
            Self {
                inner: InMemoryDurableEventLog::new(),
                append_started: Semaphore::new(0),
                append_release: Semaphore::new(0),
            }
        }

        async fn wait_for_append(&self) {
            self.append_started
                .acquire()
                .await
                .expect("append-start semaphore stays open")
                .forget();
        }

        fn release_appends(&self, count: usize) {
            self.append_release.add_permits(count);
        }
    }

    #[async_trait]
    impl DurableEventLog for StalledEventLog {
        async fn append(
            &self,
            event: RuntimeEvent,
        ) -> Result<EventLogEntry<RuntimeEvent>, EventError> {
            self.inner.append(event).await
        }

        async fn append_batch(
            &self,
            events: Vec<RuntimeEvent>,
        ) -> Vec<Result<EventLogEntry<RuntimeEvent>, EventError>> {
            self.append_started.add_permits(1);
            self.append_release
                .acquire()
                .await
                .expect("append-release semaphore stays open")
                .forget();
            self.inner.append_batch(events).await
        }

        async fn read_after_cursor(
            &self,
            stream: &EventStreamKey,
            filter: &ReadScope,
            after: Option<EventCursor>,
            limit: usize,
        ) -> Result<EventReplay<RuntimeEvent>, EventError> {
            self.inner
                .read_after_cursor(stream, filter, after, limit)
                .await
        }

        async fn head_cursor(
            &self,
            stream: &EventStreamKey,
            after: EventCursor,
        ) -> Result<EventCursor, EventError> {
            self.inner.head_cursor(stream, after).await
        }
    }

    const HOOK_HEX_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn fixture_thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-hook-projection").unwrap(),
            agent_id: AgentId::new("agent-hook-projection").unwrap(),
            project_id: Some(ProjectId::new("project-hook-projection").unwrap()),
            owner_user_id: Some(UserId::new("user-hook-projection").unwrap()),
            mission_id: None,
        }
    }

    fn fixture_milestone(kind: LoopHostMilestoneKind) -> (LoopHostMilestone, ThreadId, TurnRunId) {
        let thread_id = ThreadId::new("thread-hook-projection").unwrap();
        let run_id = TurnRunId::new();
        let scope = TurnScope::new(
            TenantId::new("tenant-hook-projection").unwrap(),
            Some(AgentId::new("agent-hook-projection").unwrap()),
            Some(ProjectId::new("project-hook-projection").unwrap()),
            thread_id.clone(),
        );
        let milestone = LoopHostMilestone {
            scope,
            actor: None,
            turn_id: TurnId::new(),
            run_id,
            loop_driver_id: LoopDriverId::new("hook-projection-driver").unwrap(),
            kind,
        };
        (milestone, thread_id, run_id)
    }

    fn projector_for(thread_id: ThreadId, run_id: TurnRunId) -> DurableLoopHostMilestoneSink {
        projector_with_events(thread_id, run_id).0
    }

    fn projector_with_events(
        thread_id: ThreadId,
        run_id: TurnRunId,
    ) -> (DurableLoopHostMilestoneSink, Arc<InMemoryEventSink>) {
        let recorded = Arc::new(InMemoryEventSink::new());
        let event_sink: Arc<dyn NonBlockingEventSink> = recorded.clone();
        let milestone_scope = DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &fixture_thread_scope(),
            thread_id,
            run_id,
        )
        .expect("durable milestone scope requires owner user — fixture supplies one");
        (
            DurableLoopHostMilestoneSink::new(event_sink, milestone_scope),
            recorded,
        )
    }

    #[test]
    fn replayed_recovery_milestone_keeps_one_logical_event_identity() {
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::FailureRecovered {
                sequence: 1,
                stage: LoopRecoveryStage::Model,
                class: LoopRecoveryClass::ModelUnavailable,
                disposition: LoopRecoveryDisposition::Retried,
            });
        let sink = projector_for(thread_id, run_id);

        let first = sink
            .runtime_event_for_milestone(&milestone)
            .expect("first projection succeeds")
            .expect("recovery milestone projects");
        let replay = sink
            .runtime_event_for_milestone(&milestone)
            .expect("replay projection succeeds")
            .expect("recovery milestone replay projects");
        let mut next = milestone;
        next.kind = LoopHostMilestoneKind::FailureRecovered {
            sequence: 2,
            stage: LoopRecoveryStage::Model,
            class: LoopRecoveryClass::ModelUnavailable,
            disposition: LoopRecoveryDisposition::Retried,
        };
        let next = sink
            .runtime_event_for_milestone(&next)
            .expect("next recovery projection succeeds")
            .expect("next recovery milestone projects");

        assert_eq!(
            first.event_id, replay.event_id,
            "append/checkpoint replay must preserve logical event identity"
        );
        assert_ne!(
            first.event_id, next.event_id,
            "separate applied recoveries must retain distinct numerator identities"
        );
    }

    #[test]
    fn hook_dispatched_milestone_projects_to_runtime_event() {
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::HookDispatched {
                hook_id: HOOK_HEX_ID.to_string(),
                point: "before_capability".to_string(),
                trust_class: "builtin".to_string(),
                owning_extension: None,
            });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("hook dispatched milestone now projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::HookDispatched);
        assert_eq!(
            event.capability_id,
            CapabilityId::new(HOOK_CAPABILITY_ID).unwrap()
        );
        assert_eq!(event.hook_id.as_deref(), Some(HOOK_HEX_ID));
        assert_eq!(event.hook_point.as_deref(), Some("before_capability"));
        assert_eq!(event.hook_trust_class.as_deref(), Some("builtin"));
        assert!(event.hook_decision.is_none());
        assert!(event.hook_failure_category.is_none());
    }

    #[test]
    fn capability_invoked_milestone_projects_to_dispatch_requested() {
        let capability_id = CapabilityId::new("demo.echo").unwrap();
        let activity_id = CapabilityActivityId::new();
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::CapabilityInvoked {
                activity_id,
                capability_id: capability_id.clone(),
            });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("capability invocation projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::CapabilityActivityRequested);
        assert_eq!(
            event.scope.invocation_id,
            InvocationId::from_uuid(activity_id.as_uuid())
        );
        assert_eq!(
            event.parent_invocation_id,
            Some(InvocationId::from_uuid(run_id.as_uuid()))
        );
        assert_eq!(event.capability_id, capability_id);
        assert!(event.provider.is_none());
        assert!(event.runtime.is_none());
    }

    #[test]
    fn capability_completed_milestone_projects_to_dispatch_succeeded() {
        let capability_id = CapabilityId::new("demo.echo").unwrap();
        let provider = ExtensionId::new("demo").unwrap();
        let activity_id = CapabilityActivityId::new();
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::CapabilityCompleted {
                activity_id,
                capability_id: capability_id.clone(),
                provider: provider.clone(),
                runtime: RuntimeKind::Wasm,
                output_bytes: 42,
            });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("capability completion projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::CapabilityActivitySucceeded);
        assert_eq!(
            event.scope.invocation_id,
            InvocationId::from_uuid(activity_id.as_uuid())
        );
        assert_eq!(
            event.parent_invocation_id,
            Some(InvocationId::from_uuid(run_id.as_uuid()))
        );
        assert_eq!(event.capability_id, capability_id);
        assert_eq!(event.provider.as_ref(), Some(&provider));
        assert_eq!(event.runtime, Some(RuntimeKind::Wasm));
        assert_eq!(event.output_bytes, Some(42));
    }

    #[test]
    fn capability_failed_milestone_projects_to_dispatch_failed() {
        let capability_id = CapabilityId::new("demo.echo").unwrap();
        let provider = ExtensionId::new("demo").unwrap();
        let activity_id = CapabilityActivityId::new();
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id: capability_id.clone(),
                provider: Some(provider.clone()),
                runtime: Some(RuntimeKind::Script),
                reason_kind: FailureKind::OperationFailed,
                safe_summary: Some(
                    LoopSafeSummary::new(
                        "read_file failed for path workspace ironclaw_issues.json: file not found",
                    )
                    .expect("safe summary"),
                ),
            });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("capability failure projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::CapabilityActivityFailed);
        assert_eq!(
            event.scope.invocation_id,
            InvocationId::from_uuid(activity_id.as_uuid())
        );
        assert_eq!(
            event.parent_invocation_id,
            Some(InvocationId::from_uuid(run_id.as_uuid()))
        );
        assert_eq!(event.capability_id, capability_id);
        assert_eq!(event.provider.as_ref(), Some(&provider));
        assert_eq!(event.runtime, Some(RuntimeKind::Script));
        assert_eq!(event.error_kind.as_deref(), Some("operation_failed"));
        assert_eq!(
            event.error_summary.as_deref(),
            Some("can't access your workspace file")
        );
    }

    #[tokio::test]
    async fn capability_milestones_with_scope_mismatch_are_not_appended() {
        let capability_id = CapabilityId::new("demo.echo").unwrap();
        let provider = ExtensionId::new("demo").unwrap();
        let activity_id = CapabilityActivityId::new();

        for kind in [
            LoopHostMilestoneKind::CapabilityInvoked {
                activity_id,
                capability_id: capability_id.clone(),
            },
            LoopHostMilestoneKind::CapabilityCompleted {
                activity_id,
                capability_id: capability_id.clone(),
                provider: provider.clone(),
                runtime: RuntimeKind::Wasm,
                output_bytes: 42,
            },
            LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id: capability_id.clone(),
                provider: Some(provider.clone()),
                runtime: Some(RuntimeKind::Script),
                reason_kind: FailureKind::OperationFailed,
                safe_summary: None,
            },
        ] {
            let (mut milestone, thread_id, run_id) = fixture_milestone(kind);
            let (sink, recorded) = projector_with_events(thread_id, run_id);
            milestone.scope.tenant_id = TenantId::new("tenant-foreign").unwrap();

            let error = sink
                .publish_loop_milestone(milestone)
                .await
                .expect_err("scope mismatch must reject capability milestone");
            assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);

            assert!(
                recorded.events().is_empty(),
                "scope-mismatched capability milestone must not append an event"
            );
        }
    }

    #[test]
    fn hook_decision_emitted_milestone_projects_to_runtime_event() {
        let (milestone, thread_id, run_id) =
            fixture_milestone(LoopHostMilestoneKind::HookDecisionEmitted {
                hook_id: HOOK_HEX_ID.to_string(),
                // Reason text must NOT leak into the durable event — only the
                // closed-vocabulary `kind_name()` should be projected.
                decision: HookDecisionSummary::Deny {
                    reason: "policy-denied raw text".to_string(),
                },
                audit_reason: None,
                owning_extension: None,
            });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("hook decision milestone now projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::HookDecisionEmitted);
        assert_eq!(event.hook_decision.as_deref(), Some("deny"));
        assert_eq!(event.hook_id.as_deref(), Some(HOOK_HEX_ID));
        let wire = serde_json::to_string(&event).expect("serialize hook decision event");
        assert!(
            !wire.contains("policy-denied"),
            "raw decision reason leaked into durable event payload: {wire}"
        );
    }

    #[test]
    fn hook_failed_milestone_projects_to_runtime_event() {
        let (milestone, thread_id, run_id) = fixture_milestone(LoopHostMilestoneKind::HookFailed {
            hook_id: HOOK_HEX_ID.to_string(),
            category: "timeout".to_string(),
            disposition: "fail_closed".to_string(),
            owning_extension: None,
        });

        let sink = projector_for(thread_id, run_id);
        let event = sink
            .runtime_event_for_milestone(&milestone)
            .expect("projection succeeds")
            .expect("hook failed milestone now projects to a runtime event");

        assert_eq!(event.kind, RuntimeEventKind::HookFailed);
        assert_eq!(event.hook_failure_category.as_deref(), Some("timeout"));
        assert_eq!(
            event.hook_failure_disposition.as_deref(),
            Some("fail_closed")
        );
        assert_eq!(event.hook_id.as_deref(), Some(HOOK_HEX_ID));
    }

    #[tokio::test]
    async fn model_attempt_persists_only_one_positive_bounded_terminal_event() {
        for (terminal_kind, expected_kind) in [
            (
                LoopHostMilestoneKind::ModelCompleted {
                    effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new(
                        "test-model",
                    )
                    .unwrap(),
                },
                RuntimeEventKind::ModelCompleted,
            ),
            (
                LoopHostMilestoneKind::ModelFailed {
                    reason_kind: AgentLoopHostErrorKind::Unavailable,
                },
                RuntimeEventKind::ModelFailed,
            ),
        ] {
            let (started, thread_id, run_id) =
                fixture_milestone(LoopHostMilestoneKind::ModelStarted {
                    requested_model_profile_id: None,
                });
            let mut terminal = started.clone();
            terminal.kind = terminal_kind;
            let recorded = Arc::new(InMemoryEventSink::new());
            let event_sink: Arc<dyn NonBlockingEventSink> = recorded.clone();
            let milestone_scope = DurableLoopHostMilestoneScope::from_thread_scope_for_run(
                &fixture_thread_scope(),
                thread_id,
                run_id,
            )
            .expect("fixture scope");
            let sink = DurableLoopHostMilestoneSink::new(event_sink, milestone_scope);

            sink.publish_loop_milestone(started)
                .await
                .expect("model start is tracked in memory");
            {
                let mut started = sink
                    .model_started_at
                    .lock()
                    .expect("model-start map is not poisoned");
                let recorded_start = started
                    .get_mut(&run_id)
                    .expect("ModelStarted records the run timestamp");
                *recorded_start = Instant::now() - std::time::Duration::from_millis(1);
            }
            sink.publish_loop_milestone(terminal)
                .await
                .expect("model terminal event is emitted");

            let events = recorded.events();
            assert_eq!(events.len(), 1, "ModelStarted must not be persisted");
            assert_eq!(events[0].kind, expected_kind);
            assert!(
                events[0].duration_ms.is_some_and(
                    |duration| duration > 0 && duration <= MAX_RUNTIME_EVENT_DURATION_MS
                ),
                "terminal model event must carry a positive bounded duration"
            );
        }
    }

    #[tokio::test]
    async fn model_terminal_without_start_preserves_unknown_duration() {
        for (terminal_kind, expected_kind) in [
            (
                LoopHostMilestoneKind::ModelCompleted {
                    effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new(
                        "test-model",
                    )
                    .unwrap(),
                },
                RuntimeEventKind::ModelCompleted,
            ),
            (
                LoopHostMilestoneKind::ModelFailed {
                    reason_kind: AgentLoopHostErrorKind::Unavailable,
                },
                RuntimeEventKind::ModelFailed,
            ),
        ] {
            let (terminal, thread_id, run_id) = fixture_milestone(terminal_kind);
            let (sink, recorded) = projector_with_events(thread_id, run_id);

            sink.publish_loop_milestone(terminal)
                .await
                .expect("terminal milestone is emitted without a matching start");

            let events = recorded.events();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].kind, expected_kind);
            assert_eq!(
                events[0].duration_ms, None,
                "a missing model start must remain distinguishable from a sub-millisecond call"
            );
        }
    }

    #[tokio::test]
    async fn loop_terminal_milestones_release_model_start_tracking() {
        for terminal_kind in [
            LoopHostMilestoneKind::Completed {
                completion_kind: LoopCompletionKind::FinalReply,
                exit_id: LoopExitId::new("exit:model-start-cleanup-completed").unwrap(),
            },
            LoopHostMilestoneKind::Failed {
                reason_kind: LoopFailureKind::InterruptedUnexpectedly,
                exit_id: LoopExitId::new("exit:model-start-cleanup-failed").unwrap(),
            },
            LoopHostMilestoneKind::Blocked {
                gate_ref: LoopGateRef::new("gate:model-start-cleanup").unwrap(),
                checkpoint_id: TurnCheckpointId::new(),
            },
        ] {
            let (started, thread_id, run_id) =
                fixture_milestone(LoopHostMilestoneKind::ModelStarted {
                    requested_model_profile_id: None,
                });
            let mut terminal = started.clone();
            terminal.kind = terminal_kind;
            let sink = projector_for(thread_id, run_id);

            sink.publish_loop_milestone(started)
                .await
                .expect("model start is tracked");
            assert!(
                sink.model_started_at
                    .lock()
                    .expect("model-start map is not poisoned")
                    .contains_key(&run_id)
            );

            sink.publish_loop_milestone(terminal)
                .await
                .expect("loop terminal milestone is handled");
            assert!(
                !sink
                    .model_started_at
                    .lock()
                    .expect("model-start map is not poisoned")
                    .contains_key(&run_id),
                "loop-terminal milestones must release abandoned model-start state"
            );
        }
    }

    #[tokio::test]
    async fn milestone_does_not_wait_for_full_coalescing_channel() {
        let (started, thread_id, run_id) = fixture_milestone(LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None,
        });
        let mut terminal = started.clone();
        terminal.kind = LoopHostMilestoneKind::ModelCompleted {
            effective_model_profile_id: ironclaw_loop_contracts::ModelProfileId::new("test-model")
                .unwrap(),
        };
        let milestone_scope = DurableLoopHostMilestoneScope::from_thread_scope_for_run(
            &fixture_thread_scope(),
            thread_id,
            run_id,
        )
        .expect("fixture scope");
        let filler_scope = milestone_scope
            .resource_scope(&started)
            .expect("fixture milestone matches durable scope");
        let stream = EventStreamKey::from_scope(&filler_scope);
        let log = Arc::new(StalledEventLog::new());
        let coalescing = Arc::new(CoalescingEventSink::new(
            Arc::clone(&log) as Arc<dyn DurableEventLog>,
            EventBatchConfig {
                max_batch: 1,
                flush_interval: std::time::Duration::from_secs(60),
                channel_capacity: 1,
            },
        ));
        let event_sink: Arc<dyn NonBlockingEventSink> = coalescing.clone();
        let sink = DurableLoopHostMilestoneSink::new(event_sink, milestone_scope);

        sink.publish_loop_milestone(started)
            .await
            .expect("model start is tracked in memory");
        let filler = RuntimeEvent::loop_cancelled(
            filler_scope,
            capability_id(LOOP_RUN_CAPABILITY_ID).expect("loop capability id"),
        );
        coalescing
            .emit(filler.clone())
            .await
            .expect("first best-effort event starts the stalled append");
        log.wait_for_append().await;
        coalescing
            .emit(filler)
            .await
            .expect("second best-effort event fills the bounded channel");

        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            sink.publish_loop_milestone(terminal),
        )
        .await
        .expect("runtime event overload must not block the agent loop")
        .expect("runtime event overload must not change the loop outcome");
        assert_eq!(
            coalescing.dropped_count(),
            1,
            "the saturated observability queue must record the dropped milestone"
        );

        log.release_appends(4);
        coalescing
            .flush()
            .await
            .expect("graceful flush persists events accepted before overload");

        let replay = log
            .read_after_cursor(&stream, &ReadScope::default(), None, 10)
            .await
            .expect("replay persisted events");
        assert_eq!(replay.entries.len(), 2);
        assert!(
            replay
                .entries
                .iter()
                .all(|entry| entry.record.kind == RuntimeEventKind::LoopCancelled),
            "the dropped terminal milestone must not appear in durable replay"
        );
    }
}
