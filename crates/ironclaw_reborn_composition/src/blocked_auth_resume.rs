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
//! undispatched and a re-drive (flow reconcile / lifecycle cleanup) retries
//! it — resumed runs leave `BlockedAuth` and are skipped on replay, and the
//! primary dispatcher settles idempotently on an already-settled gate.
//!
//! Scope safety mirrors the deleted read model: the scan is bounded to the
//! completed flow's `tenant_id` + explicit owner `user_id`, so this can never
//! resume another caller's parked run.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthProductError, RebornAuthContinuationDispatcher,
};
use ironclaw_processes::{
    ProcessGateOwnerMatch, ProcessGateQuery, ProcessGateQuerySource, ProcessSuspensionKind,
};
use ironclaw_turns::{
    IdempotencyKey, ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest,
    SourceBindingRef, TurnActor, TurnCoordinator, TurnError, TurnRunId,
};
use uuid::Uuid;

use crate::process_gate_turn_view::{turn_run_id_from_process_id, turn_scope_from_process_gate};

/// Decorates the single-run continuation dispatcher with the caller-wide
/// blocked-run fan-out described in the module docs.
pub(crate) struct BlockedAuthResumeFanout {
    inner: Arc<dyn RebornAuthContinuationDispatcher>,
    gate_source: Arc<dyn ProcessGateQuerySource<Error = TurnError>>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl BlockedAuthResumeFanout {
    pub(crate) fn new(
        inner: Arc<dyn RebornAuthContinuationDispatcher>,
        gate_source: Arc<dyn ProcessGateQuerySource<Error = TurnError>>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            inner,
            gate_source,
            turn_coordinator,
        }
    }

    /// Sweep the caller's other provider-blocked runs. An incomplete sweep —
    /// unreadable process gate projection, or any retryable resume failure — returns an
    /// error so the caller leaves the durable continuation UNDISPATCHED and a
    /// re-drive (the browser's flow reconcile, or lifecycle cleanup) retries
    /// the whole dispatch. Retries are safe end-to-end: already-resumed runs
    /// have left `BlockedAuth` and are skipped here, and the primary
    /// dispatcher settles idempotently on a no-longer-blocked gate.
    async fn fan_out(&self, event: &AuthContinuationEvent) -> Result<(), AuthProductError> {
        let primary_run_id = primary_run_id(&event.continuation);
        let tenant_id = &event.scope.resource.tenant_id;
        let user_id = &event.scope.resource.user_id;
        let parked = self
            .gate_source
            .query_process_gates(ProcessGateQuery {
                scope: event.scope.resource.without_thread_and_mission(),
                gate_kind: ProcessSuspensionKind::Authorization,
                scope_match: Some(ironclaw_processes::ProcessGateScopeMatch::Owner),
                owner_user_id: Some(user_id.clone()),
                gate_ref: None,
                owner_match: Some(ProcessGateOwnerMatch::Explicit),
                include_historical: false,
            })
            .await
            .map_err(|error| {
                tracing::warn!(
                    flow_id = %event.flow_id,
                    %error,
                    "blocked-auth fan-out could not read process gates; keeping the continuation retryable"
                );
                AuthProductError::BackendUnavailable
            })?;
        let mut resumed = 0usize;
        let mut incomplete = false;
        for run in parked {
            // Strict caller scoping: same tenant and same explicit owner user.
            if run.scope.tenant_id != *tenant_id || run.owner_user_id.as_ref() != Some(user_id) {
                continue;
            }
            let run_id = turn_run_id_from_process_id(run.process_id);
            if primary_run_id == Some(run_id) {
                // The inner dispatcher already resumed (or reported on) the
                // run the completed flow was pinned to.
                continue;
            }
            let Some(gate_ref) = run.suspension.gate_ref.clone() else {
                continue;
            };
            if !run
                .suspension
                .credential_requirements
                .iter()
                .any(|requirement| requirement.provider.as_str() == event.provider.as_str())
            {
                continue;
            }
            let actor = TurnActor::new(user_id.clone());
            let Some(scope) = turn_scope_from_process_gate(&run.scope) else {
                tracing::warn!(%run_id, "blocked-auth fan-out found a process gate without a thread scope");
                continue;
            };
            let Some(source_binding_ref) = run
                .resume_source_ref
                .as_deref()
                .and_then(|value| SourceBindingRef::new(value).ok())
            else {
                tracing::warn!(%run_id, "blocked-auth fan-out found a process gate without a resume source ref");
                continue;
            };
            let Some(reply_target_binding_ref) = run
                .reply_target_ref
                .as_deref()
                .and_then(|value| ReplyTargetBindingRef::new(value).ok())
            else {
                tracing::warn!(%run_id, "blocked-auth fan-out found a process gate without a reply target ref");
                continue;
            };
            let Ok(idempotency_key) =
                IdempotencyKey::new(format!("blocked-auth-fanout-{}-{}", event.flow_id, run_id))
            else {
                tracing::warn!(
                    %run_id,
                    "blocked-auth fan-out could not build a resume idempotency key"
                );
                continue;
            };
            let request = ResumeTurnRequest {
                scope,
                actor,
                run_id,
                gate_resolution_ref: gate_ref,
                source_binding_ref,
                reply_target_binding_ref,
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
                    // Keep sweeping so one failing run does not starve the
                    // rest, but report the sweep incomplete: the continuation
                    // must stay undispatched so a re-drive retries this run.
                    tracing::warn!(
                        %run_id,
                        flow_id = %event.flow_id,
                        %error,
                        "blocked-auth fan-out failed to resume a parked run; keeping the continuation retryable"
                    );
                    incomplete = true;
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
            return Err(AuthProductError::BackendUnavailable);
        }
        Ok(())
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
        // conflict. Either failure keeps the continuation undispatched (the
        // caller skips `mark_continuation_dispatched`), so a re-drive retries
        // both legs; replays are idempotent on both.
        let sweep = self.fan_out(&event).await;
        primary.and(sweep)
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        // A canceled flow settles only its own continuation: no credential was
        // minted, so the caller's other parked runs keep waiting for a
        // successful connect instead of being denied alongside it.
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
        ExtensionId, InvocationId, ProcessId, ResourceScope, RuntimeCredentialAccountSetup,
        RuntimeCredentialAuthRequirement, TenantId, ThreadId, UserId, VendorId,
    };
    use ironclaw_processes::{ProcessGateRecord, ProcessSuspension};
    use ironclaw_turns::{
        CancelRunRequest, CancelRunResponse, EventCursor, GateRef, GetRunStateRequest,
        SubmitTurnRequest, SubmitTurnResponse, TurnError, TurnRunState, TurnScope, TurnStatus,
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

        async fn dispatch_canceled_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    struct StaticGateSource {
        gates: Vec<ProcessGateRecord>,
    }

    #[async_trait]
    impl ProcessGateQuerySource for StaticGateSource {
        type Error = TurnError;

        async fn query_process_gates(
            &self,
            request: ProcessGateQuery,
        ) -> Result<Vec<ProcessGateRecord>, Self::Error> {
            Ok(self
                .gates
                .iter()
                .filter(|gate| {
                    gate.scope.tenant_id == request.scope.tenant_id
                        && request
                            .owner_user_id
                            .as_ref()
                            .is_none_or(|owner| gate.owner_user_id.as_ref() == Some(owner))
                        && request.gate_ref.as_ref().is_none_or(|gate_ref| {
                            gate.suspension.gate_ref.as_ref() == Some(gate_ref)
                        })
                })
                .cloned()
                .collect())
        }
    }

    #[derive(Default)]
    struct RecordingTurnCoordinator {
        resumed: Mutex<Vec<ResumeTurnRequest>>,
        /// Fail this many resume attempts before succeeding — models a
        /// transient coordinator/store outage that a re-driven dispatch
        /// recovers from.
        fail_next_resumes: Mutex<usize>,
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
            {
                let mut fail_next = self.fail_next_resumes.lock().expect("fail counter");
                if *fail_next > 0 {
                    *fail_next -= 1;
                    return Err(TurnError::Unavailable {
                        reason: "resume backend down".to_string(),
                    });
                }
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
            provider: VendorId::new("slack").expect("provider id"),
            setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            requester_extension: ExtensionId::new("slack").expect("extension id"),
            provider_scopes: Vec::new(),
        }
    }

    fn google_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: VendorId::new("google").expect("provider id"),
            setup: RuntimeCredentialAccountSetup::OAuth { scopes: Vec::new() },
            requester_extension: ExtensionId::new("gmail").expect("extension id"),
            provider_scopes: Vec::new(),
        }
    }

    fn blocked_run(
        owner: &str,
        run_id: TurnRunId,
        requirement: RuntimeCredentialAuthRequirement,
    ) -> ProcessGateRecord {
        let owner_user_id = UserId::new(owner).expect("owner");
        ProcessGateRecord {
            process_id: ProcessId::from_uuid(run_id.as_uuid()),
            scope: ResourceScope {
                tenant_id: TenantId::new(TENANT).expect("tenant"),
                user_id: owner_user_id.clone(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: Some(ThreadId::new(format!("thread-{run_id}")).expect("thread id")),
                invocation_id: InvocationId::new(),
            },
            owner_user_id: Some(owner_user_id),
            suspension: ProcessSuspension {
                kind: ProcessSuspensionKind::Authorization,
                gate_ref: Some(GateRef::new(format!("gate-{run_id}")).expect("gate ref")),
                activity_id: None,
                credential_requirements: vec![requirement],
                detail: None,
            },
            resume_source_ref: Some(format!("source:{run_id}")),
            reply_target_ref: Some(format!("reply:{run_id}")),
            historical: false,
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
        gates: Vec<ProcessGateRecord>,
        fail_next_resumes: usize,
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
            fail_next_resumes: Mutex::new(fail_next_resumes),
        });
        let fanout = BlockedAuthResumeFanout::new(
            inner.clone(),
            Arc::new(StaticGateSource { gates }),
            coordinator.clone(),
        );
        (fanout, coordinator, inner)
    }

    #[tokio::test]
    async fn turn_gate_completion_fans_out_to_other_provider_blocked_runs_only() {
        let primary = blocked_run(OWNER, TurnRunId::new(), slack_requirement());
        let waiting = blocked_run(OWNER, TurnRunId::new(), slack_requirement());
        let other_provider = blocked_run(OWNER, TurnRunId::new(), google_requirement());
        let foreign_user = blocked_run("user-mallory", TurnRunId::new(), slack_requirement());
        let primary_run_id = turn_run_id_from_process_id(primary.process_id);
        let waiting_run_id = turn_run_id_from_process_id(waiting.process_id);
        let (fanout, coordinator, inner) =
            fanout_with(vec![primary, waiting, other_provider, foreign_user], 0);

        fanout
            .dispatch_auth_continuation(event(
                "slack",
                AuthContinuationRef::TurnGateResume {
                    turn_run_ref: TurnRunRef::new(primary_run_id.to_string())
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
        assert_eq!(resumed[0].run_id, waiting_run_id);
        assert_eq!(
            resumed[0].precondition,
            ResumeTurnPrecondition::BlockedAuthGate
        );
    }

    #[tokio::test]
    async fn setup_only_completion_resumes_every_provider_blocked_run() {
        let first = blocked_run(OWNER, TurnRunId::new(), slack_requirement());
        let second = blocked_run(OWNER, TurnRunId::new(), slack_requirement());
        let first_run_id = turn_run_id_from_process_id(first.process_id);
        let second_run_id = turn_run_id_from_process_id(second.process_id);
        let (fanout, coordinator, _inner) = fanout_with(vec![first, second], 0);

        fanout
            .dispatch_auth_continuation(event("slack", AuthContinuationRef::SetupOnly))
            .await
            .expect("dispatch succeeds");

        let resumed = coordinator.resumed.lock().expect("resumed");
        let mut run_ids: Vec<_> = resumed.iter().map(|request| request.run_id).collect();
        run_ids.sort_by_key(|id| id.as_uuid());
        let mut expected = vec![first_run_id, second_run_id];
        expected.sort_by_key(|id| id.as_uuid());
        assert_eq!(
            run_ids, expected,
            "a Settings-page connect resumes every blocked chat"
        );
    }

    /// An incomplete sweep keeps the continuation retryable: the first
    /// dispatch hits a transient resume failure and must error (so the caller
    /// never stamps `continuation_emitted_at`); the re-driven dispatch retries
    /// the sweep and resumes the still-parked run. Regression for the
    /// best-effort weakening where a transient coordinator error permanently
    /// stranded every other parked run (mega-PR review finding).
    #[tokio::test]
    async fn incomplete_fan_out_keeps_the_continuation_retryable() {
        let waiting = blocked_run(OWNER, TurnRunId::new(), slack_requirement());
        let (fanout, coordinator, inner) = fanout_with(vec![waiting], 1);

        let error = fanout
            .dispatch_auth_continuation(event("slack", AuthContinuationRef::SetupOnly))
            .await
            .expect_err("an incomplete sweep must keep the continuation retryable");
        assert!(
            matches!(error, AuthProductError::BackendUnavailable),
            "incomplete sweep surfaces as retryable backend unavailability"
        );
        assert!(
            coordinator.resumed.lock().expect("resumed").is_empty(),
            "the failed attempt resumed nothing"
        );

        // The continuation was never marked dispatched, so a re-drive (flow
        // reconcile / lifecycle cleanup) retries the whole dispatch; the
        // parked run is still BlockedAuth in the snapshot and now resumes.
        fanout
            .dispatch_auth_continuation(event("slack", AuthContinuationRef::SetupOnly))
            .await
            .expect("the retried sweep completes");
        assert_eq!(
            inner.events.lock().expect("events").len(),
            2,
            "the primary dispatch replayed alongside the retried sweep"
        );
        assert_eq!(
            coordinator.resumed.lock().expect("resumed").len(),
            1,
            "the parked run resumed exactly once"
        );
    }
}
