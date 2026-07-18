//! Fan a completed auth flow out to every other run the caller has parked on
//! the same provider's credentials.
//!
//! An OAuth flow's continuation references at most one run
//! (`TurnGateResume`), or none at all (`SetupOnly`, when the connect started
//! from the Settings/extensions surface). But the durable outcome of a
//! completed flow — the credential account plus, for Slack, the identity
//! binding — satisfies *every* run of that caller currently `BlockedAuth` on
//! a requirement for the same provider. The retired pairing-code redeem path
//! had exactly this fan-out (the deleted `channel_connection_resume`
//! machinery: pair once, all waiting chats continue); this decorator restores
//! that behavior for OAuth completions, provider-keyed so Google and Slack
//! personal both benefit.
//!
//! Ordering matters: the decorator runs at continuation-dispatch time, which
//! is strictly after `complete_oauth_callback` committed the credential
//! account, so resumed runs re-running `extension_activate` find their
//! requirements satisfied. Fan-out is idempotent per (flow, run), and an
//! incomplete sweep returns an error so the durable continuation remains
//! undispatched and can be retried.
//!
//! Scope safety mirrors the deleted read model: the scan is bounded to the
//! completed flow's `tenant_id` + explicit owner `user_id`, so this can never
//! resume another caller's parked run.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthProductError};
use ironclaw_turns::{
    IdempotencyKey, ResumeTurnPrecondition, ResumeTurnRequest, TurnCoordinator,
    TurnPersistenceSnapshot, TurnRunId, TurnStatus,
};
use uuid::Uuid;

use crate::turn_run_snapshot::TurnRunSnapshotSource;
use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;

/// Source of the durable turn-state snapshot the fan-out scans. Split out so
/// tests can hand-build snapshots without a filesystem-backed store.
#[async_trait]
pub(crate) trait BlockedAuthSnapshotSource: Send + Sync {
    async fn snapshot(&self) -> Option<TurnPersistenceSnapshot>;
}

#[async_trait]
impl<T> BlockedAuthSnapshotSource for T
where
    T: TurnRunSnapshotSource + ?Sized,
{
    async fn snapshot(&self) -> Option<TurnPersistenceSnapshot> {
        match self.turn_run_snapshot().await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::debug!(
                    %error,
                    "blocked-auth fan-out could not read the turn persistence snapshot"
                );
                None
            }
        }
    }
}

/// Decorates the single-run continuation dispatcher with the caller-wide
/// blocked-run fan-out described in the module docs.
pub(crate) struct BlockedAuthResumeFanout {
    inner: Arc<dyn RebornAuthContinuationDispatcher>,
    snapshot_source: Arc<dyn BlockedAuthSnapshotSource>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl BlockedAuthResumeFanout {
    pub(crate) fn new(
        inner: Arc<dyn RebornAuthContinuationDispatcher>,
        snapshot_source: Arc<dyn BlockedAuthSnapshotSource>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            inner,
            snapshot_source,
            turn_coordinator,
        }
    }

    async fn fan_out(&self, event: &AuthContinuationEvent) -> Result<(), AuthProductError> {
        let primary_run_id = primary_run_id(&event.continuation);
        let Some(snapshot) = self.snapshot_source.snapshot().await else {
            tracing::warn!("blocked-auth fan-out could not read the durable turn snapshot");
            return Err(AuthProductError::BackendUnavailable);
        };
        let tenant_id = &event.scope.resource.tenant_id;
        let user_id = &event.scope.resource.user_id;
        let mut resumed = 0usize;
        let mut incomplete = false;
        for run in &snapshot.runs {
            if run.status != TurnStatus::BlockedAuth {
                continue;
            }
            // Strict caller scoping: same tenant and same explicit owner user.
            if run.scope.tenant_id != *tenant_id
                || run.scope.explicit_owner_user_id() != Some(user_id)
            {
                continue;
            }
            if primary_run_id == Some(run.run_id) {
                // The inner dispatcher already resumed (or reported on) the
                // run the completed flow was pinned to.
                continue;
            }
            let Some(gate_ref) = run.gate_ref.clone() else {
                continue;
            };
            if !run
                .credential_requirements
                .iter()
                .any(|requirement| requirement.provider.as_str() == event.provider.as_str())
            {
                continue;
            }
            // The run record does not carry the actor; join it from the
            // parent turn record. A malformed snapshot must keep the
            // continuation retryable rather than permanently stranding this
            // run after the flow is marked dispatched.
            let Some(actor) = snapshot
                .turns
                .iter()
                .find(|turn| turn.turn_id == run.turn_id)
                .map(|turn| turn.actor.clone())
            else {
                tracing::warn!(
                    run_id = %run.run_id,
                    "blocked-auth fan-out found a blocked run with no parent turn record"
                );
                incomplete = true;
                continue;
            };
            let Ok(idempotency_key) = IdempotencyKey::new(format!(
                "blocked-auth-fanout-{}-{}",
                event.flow_id, run.run_id
            )) else {
                tracing::warn!(
                    run_id = %run.run_id,
                    "blocked-auth fan-out could not build a resume idempotency key"
                );
                incomplete = true;
                continue;
            };
            let request = ResumeTurnRequest {
                scope: run.scope.clone(),
                actor,
                run_id: run.run_id,
                gate_resolution_ref: gate_ref,
                source_binding_ref: run.source_binding_ref.clone(),
                reply_target_binding_ref: run.reply_target_binding_ref.clone(),
                idempotency_key,
                // No credential_ref: the resumed run re-runs its capability
                // (extension_activate), which re-checks requirement
                // satisfaction against the now-existing credential account —
                // the same self-correcting shape the pairing redeem relied on.
                precondition: ResumeTurnPrecondition::BlockedAuthGate,
                resume_disposition: None,
            };
            match self.turn_coordinator.resume_turn(request).await {
                Ok(_) => resumed += 1,
                Err(error) => {
                    incomplete = true;
                    tracing::warn!(
                        run_id = %run.run_id,
                        flow_id = %event.flow_id,
                        %error,
                        "blocked-auth fan-out failed to resume a parked run"
                    );
                }
            }
        }
        if resumed > 0 {
            tracing::debug!(
                flow_id = %event.flow_id,
                provider = %event.provider,
                resumed,
                "blocked-auth fan-out resumed additional parked runs"
            );
        }
        if incomplete {
            Err(AuthProductError::BackendUnavailable)
        } else {
            Ok(())
        }
    }
}

fn primary_run_id(continuation: &AuthContinuationRef) -> Option<TurnRunId> {
    match continuation {
        AuthContinuationRef::TurnGateResume { turn_run_ref, .. } => {
            Uuid::parse_str(turn_run_ref.as_str())
                .ok()
                .map(TurnRunId::from_uuid)
        }
        _ => None,
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for BlockedAuthResumeFanout {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        let primary = self.inner.dispatch_auth_continuation(event.clone()).await;
        // Fan out regardless of the primary outcome: the credential account
        // exists once this event is emitted, and the caller's other parked
        // runs deserve the resume even if the primary run's own resume hit a
        // conflict.
        let fan_out = self.fan_out(&event).await;
        match (primary, fan_out) {
            (Err(error), _) | (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        self.inner.dispatch_canceled_auth_continuation(event).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use chrono::Utc;
    use ironclaw_auth::{
        AuthFlowId, AuthGateRef, AuthProductScope, AuthProviderId, AuthSurface, TurnRunRef,
    };
    use ironclaw_host_api::{
        ExtensionId, InvocationId, ResourceScope, RuntimeCredentialAccountProviderId,
        RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, TenantId, ThreadId,
        UserId,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, AgentLoopDriverDescriptor, CancelRunRequest, CancelRunResponse,
        CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
        ConcurrencyClass, ContextProfileId, EventCursor, GateRef, GetRunStateRequest, LoopDriverId,
        ModelProfileId, RedactedRunProfileProvenance, ReplyTargetBindingRef, ResolvedRunProfile,
        ResourceBudgetPolicy, ResourceBudgetTier, RunClassId, RunProfileFingerprint, RunProfileId,
        RunProfileVersion, RuntimeProfileConstraints, SchedulingClass, SourceBindingRef,
        SteeringPolicy, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnError, TurnId,
        TurnRecord, TurnRunProfile, TurnRunState, TurnScope,
    };

    struct RecordingInnerDispatcher {
        events: Mutex<Vec<AuthContinuationEvent>>,
    }

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RecordingInnerDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            self.events.lock().expect("inner events lock").push(event);
            Ok(())
        }
    }

    struct StaticSnapshotSource {
        snapshot: TurnPersistenceSnapshot,
    }

    #[async_trait]
    impl BlockedAuthSnapshotSource for StaticSnapshotSource {
        async fn snapshot(&self) -> Option<TurnPersistenceSnapshot> {
            Some(self.snapshot.clone())
        }
    }

    #[derive(Default)]
    struct RecordingTurnCoordinator {
        resumed: Mutex<Vec<ResumeTurnRequest>>,
        fail_resumes: bool,
    }

    #[async_trait]
    impl TurnCoordinator for RecordingTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            unreachable!("fan-out tests do not prepare turns")
        }

        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            unreachable!("fan-out tests do not submit turns")
        }

        async fn resume_turn(
            &self,
            request: ResumeTurnRequest,
        ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
            if self.fail_resumes {
                return Err(TurnError::Unavailable {
                    reason: "resume backend down".to_string(),
                });
            }
            let run_id = request.run_id;
            self.resumed.lock().expect("resume lock").push(request);
            Ok(ironclaw_turns::ResumeTurnResponse {
                run_id,
                status: TurnStatus::Queued,
                event_cursor: EventCursor(1),
            })
        }

        async fn retry_turn(
            &self,
            _request: ironclaw_turns::RetryTurnRequest,
        ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
            unreachable!("fan-out tests do not retry turns")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unreachable!("fan-out tests do not cancel runs")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unreachable!("fan-out tests do not read run state")
        }
    }

    const TENANT: &str = "tenant-fanout";
    const OWNER: &str = "user-alice";

    fn slack_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("slack_personal")
                .expect("provider id"),
            setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            requester_extension: ExtensionId::new("slack").expect("extension id"),
            provider_scopes: Vec::new(),
        }
    }

    fn google_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("google").expect("provider id"),
            setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            requester_extension: ExtensionId::new("gmail").expect("extension id"),
            provider_scopes: Vec::new(),
        }
    }

    fn blocked_run(
        owner: &str,
        run_id: TurnRunId,
        turn_id: TurnId,
        requirement: RuntimeCredentialAuthRequirement,
    ) -> ironclaw_turns::TurnRunRecord {
        let scope = TurnScope::new_with_owner(
            TenantId::new(TENANT).expect("tenant"),
            None,
            None,
            ThreadId::new(format!("thread-{run_id}")).expect("thread id"),
            Some(UserId::new(owner).expect("owner")),
        );
        ironclaw_turns::TurnRunRecord {
            run_id,
            turn_id,
            scope,
            accepted_message_ref: AcceptedMessageRef::new(format!("message:{run_id}"))
                .expect("message ref"),
            source_binding_ref: SourceBindingRef::new(format!("source:{run_id}"))
                .expect("source binding ref"),
            reply_target_binding_ref: ReplyTargetBindingRef::new(format!("reply:{run_id}"))
                .expect("reply target binding ref"),
            status: TurnStatus::BlockedAuth,
            profile: TurnRunProfile::from_resolved(resolved_run_profile()),
            resolved_model_route: None,
            model_usage: None,
            checkpoint_id: None,
            gate_ref: Some(GateRef::new(format!("gate-{run_id}")).expect("gate ref")),
            blocked_activity_id: None,
            credential_requirements: vec![requirement],
            failure: None,
            event_cursor: EventCursor(1),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: Utc::now(),
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
            resume_disposition: None,
        }
    }

    fn resolved_run_profile() -> ResolvedRunProfile {
        let checkpoint_schema_id =
            CheckpointSchemaId::new("blocked_auth_checkpoint").expect("checkpoint schema");
        ResolvedRunProfile {
            run_class_id: RunClassId::new("blocked_auth").expect("run class"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: AgentLoopDriverDescriptor {
                id: LoopDriverId::new("blocked_auth_loop").expect("loop driver"),
                version: RunProfileVersion::new(1),
                checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
                checkpoint_schema_version: Some(RunProfileVersion::new(1)),
            },
            checkpoint_schema_id,
            checkpoint_schema_version: RunProfileVersion::new(1),
            model_profile_id: ModelProfileId::new("blocked_auth_model").expect("model profile"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new("blocked_auth_caps")
                .expect("capability surface profile"),
            context_profile_id: ContextProfileId::new("blocked_auth_context")
                .expect("context profile"),
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
                require_before_side_effect: true,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("blocked_auth_budget").expect("budget tier"),
                max_model_calls: 1,
                max_capability_invocations: 1,
            },
            personal_context_policy: Default::default(),
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("blocked_auth").expect("scheduling class"),
            concurrency_class: ConcurrencyClass::new("blocked_auth").expect("concurrency class"),
            resolution_fingerprint: RunProfileFingerprint::new("blocked-auth-profile-v1")
                .expect("run profile fingerprint"),
            provenance: RedactedRunProfileProvenance {
                sources: Vec::new(),
                effective_privileges: Vec::new(),
            },
        }
    }

    fn parent_turn(owner: &str, run: &ironclaw_turns::TurnRunRecord) -> TurnRecord {
        TurnRecord {
            turn_id: run.turn_id,
            scope: run.scope.clone(),
            actor: TurnActor::new(UserId::new(owner).expect("actor")),
            accepted_message_ref: run.accepted_message_ref.clone(),
            source_binding_ref: run.source_binding_ref.clone(),
            reply_target_binding_ref: run.reply_target_binding_ref.clone(),
            created_at: Utc::now(),
        }
    }

    fn event(provider: &str, continuation: AuthContinuationRef) -> AuthContinuationEvent {
        let resource = ResourceScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            user_id: UserId::new(OWNER).expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        AuthContinuationEvent {
            flow_id: AuthFlowId::new(),
            scope: AuthProductScope::new(resource, AuthSurface::Callback),
            continuation,
            provider: AuthProviderId::new(provider).expect("provider"),
            credential_account_id: None,
            emitted_at: Utc::now(),
        }
    }

    fn fanout_with(
        snapshot: TurnPersistenceSnapshot,
        fail_resumes: bool,
    ) -> (
        BlockedAuthResumeFanout,
        Arc<RecordingTurnCoordinator>,
        Arc<RecordingInnerDispatcher>,
    ) {
        let inner = Arc::new(RecordingInnerDispatcher {
            events: Mutex::new(Vec::new()),
        });
        let coordinator = Arc::new(RecordingTurnCoordinator {
            resumed: Mutex::new(Vec::new()),
            fail_resumes,
        });
        let fanout = BlockedAuthResumeFanout::new(
            inner.clone(),
            Arc::new(StaticSnapshotSource { snapshot }),
            coordinator.clone(),
        );
        (fanout, coordinator, inner)
    }

    #[tokio::test]
    async fn turn_gate_completion_fans_out_to_other_provider_blocked_runs_only() {
        let primary = blocked_run(OWNER, TurnRunId::new(), TurnId::new(), slack_requirement());
        let waiting = blocked_run(OWNER, TurnRunId::new(), TurnId::new(), slack_requirement());
        let other_provider =
            blocked_run(OWNER, TurnRunId::new(), TurnId::new(), google_requirement());
        let foreign_user = blocked_run(
            "user-mallory",
            TurnRunId::new(),
            TurnId::new(),
            slack_requirement(),
        );
        let snapshot = TurnPersistenceSnapshot {
            turns: vec![
                parent_turn(OWNER, &primary),
                parent_turn(OWNER, &waiting),
                parent_turn(OWNER, &other_provider),
                parent_turn("user-mallory", &foreign_user),
            ],
            runs: vec![
                primary.clone(),
                waiting.clone(),
                other_provider.clone(),
                foreign_user.clone(),
            ],
            ..Default::default()
        };
        let (fanout, coordinator, inner) = fanout_with(snapshot, false);

        fanout
            .dispatch_auth_continuation(event(
                "slack_personal",
                AuthContinuationRef::TurnGateResume {
                    turn_run_ref: TurnRunRef::new(primary.run_id.to_string())
                        .expect("turn run ref"),
                    gate_ref: AuthGateRef::new("gate-primary").expect("auth gate ref"),
                },
            ))
            .await
            .expect("dispatch succeeds");

        assert_eq!(inner.events.lock().expect("events").len(), 1);
        let resumed = coordinator.resumed.lock().expect("resumed");
        assert_eq!(
            resumed.len(),
            1,
            "exactly the caller's other slack-blocked run resumes"
        );
        assert_eq!(resumed[0].run_id, waiting.run_id);
        assert_eq!(
            resumed[0].precondition,
            ResumeTurnPrecondition::BlockedAuthGate
        );
    }

    #[tokio::test]
    async fn setup_only_completion_resumes_every_provider_blocked_run() {
        let first = blocked_run(OWNER, TurnRunId::new(), TurnId::new(), slack_requirement());
        let second = blocked_run(OWNER, TurnRunId::new(), TurnId::new(), slack_requirement());
        let snapshot = TurnPersistenceSnapshot {
            turns: vec![parent_turn(OWNER, &first), parent_turn(OWNER, &second)],
            runs: vec![first.clone(), second.clone()],
            ..Default::default()
        };
        let (fanout, coordinator, _inner) = fanout_with(snapshot, false);

        fanout
            .dispatch_auth_continuation(event("slack_personal", AuthContinuationRef::SetupOnly))
            .await
            .expect("dispatch succeeds");

        let resumed = coordinator.resumed.lock().expect("resumed");
        let mut run_ids: Vec<_> = resumed.iter().map(|request| request.run_id).collect();
        run_ids.sort_by_key(|id| id.as_uuid());
        let mut expected = vec![first.run_id, second.run_id];
        expected.sort_by_key(|id| id.as_uuid());
        assert_eq!(
            run_ids, expected,
            "a Settings-page connect resumes every blocked chat"
        );
    }

    #[tokio::test]
    async fn fan_out_failures_keep_the_continuation_retryable() {
        let waiting = blocked_run(OWNER, TurnRunId::new(), TurnId::new(), slack_requirement());
        let snapshot = TurnPersistenceSnapshot {
            turns: vec![parent_turn(OWNER, &waiting)],
            runs: vec![waiting],
            ..Default::default()
        };
        let (fanout, coordinator, inner) = fanout_with(snapshot, true);

        let error = fanout
            .dispatch_auth_continuation(event("slack_personal", AuthContinuationRef::SetupOnly))
            .await
            .expect_err("resume failures must prevent dispatched acknowledgement");

        assert_eq!(error, AuthProductError::BackendUnavailable);
        assert_eq!(inner.events.lock().expect("events").len(), 1);
        assert!(coordinator.resumed.lock().expect("resumed").is_empty());
    }
}
