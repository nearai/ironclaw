use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use ironclaw_triggers::{
    TriggerActiveRunLookup, TriggerActiveRunState, TriggerActiveRunStateRequest, TriggerError,
    TriggerRunHistoryStatus,
};
use ironclaw_turns::{TurnPersistenceSnapshot, TurnStatus};

type ActiveRunIndex = HashMap<String, HashMap<ironclaw_turns::TurnRunId, TriggerActiveRunState>>;

pub(crate) struct SnapshotActiveRunLookup {
    snapshot_source: Arc<dyn TriggerTurnSnapshotSource>,
}

impl SnapshotActiveRunLookup {
    pub(crate) fn new(snapshot_source: Arc<dyn TriggerTurnSnapshotSource>) -> Self {
        Self { snapshot_source }
    }
}

#[async_trait]
impl TriggerActiveRunLookup for SnapshotActiveRunLookup {
    async fn active_run_state(
        &self,
        request: TriggerActiveRunStateRequest,
    ) -> Result<TriggerActiveRunState, TriggerError> {
        let snapshot = self.snapshot_source.snapshot().await?;
        let run_index = active_run_index(&snapshot);
        Ok(active_run_state_from_index(&run_index, &request))
    }

    async fn active_run_states(
        &self,
        requests: Vec<TriggerActiveRunStateRequest>,
    ) -> Vec<Result<TriggerActiveRunState, TriggerError>> {
        if requests.is_empty() {
            return Vec::new();
        }
        let snapshot = match self.snapshot_source.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let reason = error.to_string();
                return requests
                    .into_iter()
                    .map(|_| {
                        Err(TriggerError::Backend {
                            reason: reason.clone(),
                        })
                    })
                    .collect();
            }
        };
        let run_index = active_run_index(&snapshot);
        requests
            .iter()
            .map(|request| Ok(active_run_state_from_index(&run_index, request)))
            .collect()
    }
}

fn active_run_index(snapshot: &TurnPersistenceSnapshot) -> ActiveRunIndex {
    let mut index = ActiveRunIndex::new();
    for run in &snapshot.runs {
        let state = if run.status.is_terminal() {
            TriggerActiveRunState::Terminal {
                status: terminal_run_history_status(run.status),
            }
        } else {
            TriggerActiveRunState::Nonterminal
        };
        index
            .entry(run.scope.tenant_id.as_str().to_owned())
            .or_default()
            .insert(run.run_id, state);
    }
    index
}

fn active_run_state_from_index(
    run_index: &ActiveRunIndex,
    request: &TriggerActiveRunStateRequest,
) -> TriggerActiveRunState {
    run_index
        .get(request.tenant_id.as_str())
        .and_then(|tenant_runs| tenant_runs.get(&request.run_id))
        .copied()
        .unwrap_or(TriggerActiveRunState::Missing)
}

fn terminal_run_history_status(status: TurnStatus) -> TriggerRunHistoryStatus {
    debug_assert!(
        status.is_terminal(),
        "only terminal turn statuses should be normalized into run-history status"
    );
    match status {
        TurnStatus::Completed => TriggerRunHistoryStatus::Ok,
        TurnStatus::Cancelled | TurnStatus::Failed | TurnStatus::RecoveryRequired => {
            TriggerRunHistoryStatus::Error
        }
        TurnStatus::Queued
        | TurnStatus::Running
        | TurnStatus::BlockedApproval
        | TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::BlockedExternalTool
        | TurnStatus::CancelRequested => TriggerRunHistoryStatus::Error,
    }
}

#[async_trait]
pub(crate) trait TriggerTurnSnapshotSource: Send + Sync {
    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError>;
}

pub(crate) struct LocalTriggerTurnSnapshotSource<S> {
    store: Arc<S>,
}

impl<S> LocalTriggerTurnSnapshotSource<S> {
    pub(crate) fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

// Durable filesystem store (libSQL/Postgres, without `inmemory-turn-state`):
// async `Result`.
#[cfg(all(
    any(feature = "libsql", feature = "postgres"),
    not(feature = "inmemory-turn-state")
))]
#[async_trait]
impl<F> TriggerTurnSnapshotSource
    for LocalTriggerTurnSnapshotSource<ironclaw_turns::FilesystemTurnStateStore<F>>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
        self.store
            .persistence_snapshot()
            .await
            .map_err(trigger_backend_error)
    }
}

// In-memory authority (no-DB builds, or any build with `inmemory-turn-state`):
// sync infallible snapshot.
#[cfg(any(
    feature = "inmemory-turn-state",
    not(any(feature = "libsql", feature = "postgres"))
))]
#[async_trait]
impl TriggerTurnSnapshotSource
    for LocalTriggerTurnSnapshotSource<ironclaw_turns::InMemoryTurnStateStore>
{
    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
        Ok(self.store.persistence_snapshot())
    }
}

#[cfg(all(
    any(feature = "libsql", feature = "postgres"),
    not(feature = "inmemory-turn-state")
))]
fn trigger_backend_error(error: impl std::fmt::Display) -> TriggerError {
    TriggerError::Backend {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_host_api::TenantId;
    use ironclaw_triggers::TriggerId;
    use ironclaw_turns::{
        AcceptedMessageRef, AgentLoopDriverDescriptor, CancellationPolicy,
        CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId, ConcurrencyClass,
        ContextProfileId, EventCursor, LoopDriverId, ModelProfileId, RedactedRunProfileProvenance,
        ResolvedRunProfile, ResourceBudgetPolicy, ResourceBudgetTier, RunClassId,
        RunProfileFingerprint, RunProfileId, RunProfileVersion, RuntimeProfileConstraints,
        SchedulingClass, SourceBindingRef, SteeringPolicy, TurnId, TurnRunId, TurnRunProfile,
        TurnRunRecord, TurnScope,
    };

    #[derive(Default)]
    struct CountingSnapshotSource {
        calls: std::sync::Mutex<usize>,
    }

    impl CountingSnapshotSource {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("snapshot calls lock")
        }
    }

    #[async_trait]
    impl TriggerTurnSnapshotSource for CountingSnapshotSource {
        async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
            *self.calls.lock().expect("snapshot calls lock") += 1;
            Ok(TurnPersistenceSnapshot::default())
        }
    }

    struct StaticSnapshotSource {
        snapshot: TurnPersistenceSnapshot,
    }

    #[async_trait]
    impl TriggerTurnSnapshotSource for StaticSnapshotSource {
        async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
            Ok(self.snapshot.clone())
        }
    }

    #[derive(Default)]
    struct FailingSnapshotSource {
        calls: std::sync::Mutex<usize>,
    }

    impl FailingSnapshotSource {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("snapshot calls lock")
        }
    }

    #[async_trait]
    impl TriggerTurnSnapshotSource for FailingSnapshotSource {
        async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, TriggerError> {
            *self.calls.lock().expect("snapshot calls lock") += 1;
            Err(TriggerError::Backend {
                reason: "snapshot failed".to_string(),
            })
        }
    }

    #[test]
    fn terminal_turn_statuses_map_to_run_history_statuses() {
        let cases = [
            (TurnStatus::Completed, TriggerRunHistoryStatus::Ok),
            (TurnStatus::Cancelled, TriggerRunHistoryStatus::Error),
            (TurnStatus::Failed, TriggerRunHistoryStatus::Error),
            (TurnStatus::RecoveryRequired, TriggerRunHistoryStatus::Error),
        ];

        for (turn_status, expected) in cases {
            assert_eq!(terminal_run_history_status(turn_status), expected);
        }
    }

    #[tokio::test]
    async fn active_run_batch_lookup_uses_one_snapshot_for_page() {
        let snapshot_source = Arc::new(CountingSnapshotSource::default());
        let lookup = SnapshotActiveRunLookup::new(snapshot_source.clone());
        let tenant_id = TenantId::new("trigger-active-batch-tenant").expect("tenant id");
        let fire_slot = Utc::now();

        let results = lookup
            .active_run_states(vec![
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
                TriggerActiveRunStateRequest {
                    tenant_id,
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
            ])
            .await;

        assert_eq!(snapshot_source.calls(), 1);
        assert_eq!(results.len(), 2);
        assert!(
            results
                .into_iter()
                .all(|result| matches!(result, Ok(TriggerActiveRunState::Missing)))
        );
    }

    #[tokio::test]
    async fn active_run_batch_lookup_returns_nonterminal_and_terminal_states_from_snapshot() {
        let tenant_id = TenantId::new("trigger-active-state-tenant").expect("tenant id");
        let nonterminal_run_id = TurnRunId::new();
        let terminal_run_id = TurnRunId::new();
        let missing_run_id = TurnRunId::new();
        let snapshot_source = Arc::new(StaticSnapshotSource {
            snapshot: TurnPersistenceSnapshot {
                runs: vec![
                    turn_run_record(&tenant_id, nonterminal_run_id, TurnStatus::Running),
                    turn_run_record(&tenant_id, terminal_run_id, TurnStatus::Completed),
                ],
                ..TurnPersistenceSnapshot::default()
            },
        });
        let lookup = SnapshotActiveRunLookup::new(snapshot_source);
        let fire_slot = Utc::now();

        let results = lookup
            .active_run_states(vec![
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: nonterminal_run_id,
                },
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: terminal_run_id,
                },
                TriggerActiveRunStateRequest {
                    tenant_id,
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: missing_run_id,
                },
            ])
            .await;

        assert!(matches!(results[0], Ok(TriggerActiveRunState::Nonterminal)));
        assert!(matches!(
            results[1],
            Ok(TriggerActiveRunState::Terminal {
                status: TriggerRunHistoryStatus::Ok
            })
        ));
        assert!(matches!(results[2], Ok(TriggerActiveRunState::Missing)));
    }

    #[tokio::test]
    async fn human_interaction_gates_keep_active_backpressure() {
        let tenant_id = TenantId::new("trigger-blocked-state-tenant").expect("tenant id");
        let approval_run = TurnRunId::new();
        let auth_run = TurnRunId::new();
        let resource_run = TurnRunId::new();
        let dependent_run = TurnRunId::new();
        let snapshot_source = Arc::new(StaticSnapshotSource {
            snapshot: TurnPersistenceSnapshot {
                runs: vec![
                    turn_run_record(&tenant_id, approval_run, TurnStatus::BlockedApproval),
                    turn_run_record(&tenant_id, auth_run, TurnStatus::BlockedAuth),
                    turn_run_record(&tenant_id, resource_run, TurnStatus::BlockedResource),
                    turn_run_record(&tenant_id, dependent_run, TurnStatus::BlockedDependentRun),
                ],
                ..TurnPersistenceSnapshot::default()
            },
        });
        let lookup = SnapshotActiveRunLookup::new(snapshot_source);
        let fire_slot = Utc::now();
        let request = |run_id| TriggerActiveRunStateRequest {
            tenant_id: tenant_id.clone(),
            trigger_id: TriggerId::new(),
            fire_slot,
            run_id,
        };

        let results = lookup
            .active_run_states(vec![
                request(approval_run),
                request(auth_run),
                request(resource_run),
                request(dependent_run),
            ])
            .await;

        assert!(matches!(results[0], Ok(TriggerActiveRunState::Nonterminal)));
        assert!(matches!(results[1], Ok(TriggerActiveRunState::Nonterminal)));
        assert!(matches!(results[2], Ok(TriggerActiveRunState::Nonterminal)));
        assert!(matches!(results[3], Ok(TriggerActiveRunState::Nonterminal)));
    }

    #[tokio::test]
    async fn active_run_batch_lookup_returns_empty_without_snapshot() {
        let snapshot_source = Arc::new(CountingSnapshotSource::default());
        let lookup = SnapshotActiveRunLookup::new(snapshot_source.clone());

        let results = lookup.active_run_states(Vec::new()).await;

        assert!(results.is_empty());
        assert_eq!(snapshot_source.calls(), 0);
    }

    #[tokio::test]
    async fn snapshot_source_error_fans_out_to_all_batch_results() {
        let snapshot_source = Arc::new(FailingSnapshotSource::default());
        let lookup = SnapshotActiveRunLookup::new(snapshot_source.clone());
        let tenant_id = TenantId::new("trigger-active-error-tenant").expect("tenant id");
        let fire_slot = Utc::now();

        let results = lookup
            .active_run_states(vec![
                TriggerActiveRunStateRequest {
                    tenant_id: tenant_id.clone(),
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
                TriggerActiveRunStateRequest {
                    tenant_id,
                    trigger_id: TriggerId::new(),
                    fire_slot,
                    run_id: TurnRunId::new(),
                },
            ])
            .await;

        assert_eq!(snapshot_source.calls(), 1);
        assert_eq!(results.len(), 2);
        assert!(results.into_iter().all(|result| matches!(
            result,
            Err(TriggerError::Backend { reason }) if reason.contains("snapshot failed")
        )));
    }

    fn turn_run_record(
        tenant_id: &TenantId,
        run_id: TurnRunId,
        status: TurnStatus,
    ) -> TurnRunRecord {
        let scope = TurnScope::new(
            tenant_id.clone(),
            None,
            None,
            ironclaw_host_api::ThreadId::new(format!("thread-{run_id}")).expect("thread id"),
        );
        TurnRunRecord {
            run_id,
            turn_id: TurnId::new(),
            scope,
            accepted_message_ref: AcceptedMessageRef::new(format!("message:{run_id}"))
                .expect("message ref"),
            source_binding_ref: SourceBindingRef::new(format!("source:{run_id}"))
                .expect("source binding ref"),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(format!(
                "reply:{run_id}"
            ))
            .expect("reply target binding ref"),
            status,
            profile: TurnRunProfile::from_resolved(resolved_run_profile()),
            resolved_model_route: None,
            checkpoint_id: None,
            gate_ref: None,
            blocked_activity_id: None,
            credential_requirements: Vec::new(),
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
            CheckpointSchemaId::new("trigger_active_checkpoint").expect("checkpoint schema");
        ResolvedRunProfile {
            run_class_id: RunClassId::new("trigger_active").expect("run class"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: AgentLoopDriverDescriptor {
                id: LoopDriverId::new("trigger_active_loop").expect("loop driver"),
                version: RunProfileVersion::new(1),
                checkpoint_schema_id: Some(checkpoint_schema_id.clone()),
                checkpoint_schema_version: Some(RunProfileVersion::new(1)),
            },
            checkpoint_schema_id,
            checkpoint_schema_version: RunProfileVersion::new(1),
            model_profile_id: ModelProfileId::new("trigger_active_model").expect("model profile"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new("trigger_active_caps")
                .expect("capability surface profile"),
            context_profile_id: ContextProfileId::new("trigger_active_context")
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
                tier: ResourceBudgetTier::new("trigger_active_budget").expect("budget tier"),
                max_model_calls: 1,
                max_capability_invocations: 1,
            },
            personal_context_policy: Default::default(),
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("trigger_active").expect("scheduling class"),
            concurrency_class: ConcurrencyClass::new("trigger_active").expect("concurrency class"),
            resolution_fingerprint: RunProfileFingerprint::new("trigger-active-profile-v1")
                .expect("run profile fingerprint"),
            provenance: RedactedRunProfileProvenance {
                sources: Vec::new(),
                effective_privileges: Vec::new(),
            },
        }
    }
}
