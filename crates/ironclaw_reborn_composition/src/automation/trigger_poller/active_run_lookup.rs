use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ProcessId;
use ironclaw_processes::{
    ProcessLifecycleLookupBatchRequest, ProcessLifecycleLookupRequest,
    ProcessLifecycleLookupResult, ProcessLifecycleLookupSource, ProcessLifecycleStatus,
    ProcessSuspensionKind,
};
use ironclaw_triggers::{
    BlockedActiveRunKind, TriggerActiveRunLookup, TriggerActiveRunState,
    TriggerActiveRunStateRequest, TriggerError, TriggerRunHistoryStatus,
};
use ironclaw_turns::{TurnError, TurnRunId};

pub(crate) struct ProcessActiveRunLookup {
    lifecycle_source: Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>,
}

pub(crate) struct RebindableProcessLifecycleLookupSource {
    inner: std::sync::Arc<
        std::sync::RwLock<std::sync::Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>>,
    >,
}

impl RebindableProcessLifecycleLookupSource {
    pub(crate) fn new(
        inner: std::sync::Arc<
            std::sync::RwLock<std::sync::Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>>,
        >,
    ) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ProcessLifecycleLookupSource for RebindableProcessLifecycleLookupSource {
    type Error = TurnError;

    async fn process_lifecycle_states(
        &self,
        request: ProcessLifecycleLookupBatchRequest,
    ) -> Vec<Result<ProcessLifecycleLookupResult, Self::Error>> {
        let source = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        source.process_lifecycle_states(request).await
    }
}

impl ProcessActiveRunLookup {
    pub(crate) fn new(
        lifecycle_source: Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>,
    ) -> Self {
        Self { lifecycle_source }
    }
}

#[async_trait]
impl TriggerActiveRunLookup for ProcessActiveRunLookup {
    async fn active_run_state(
        &self,
        request: TriggerActiveRunStateRequest,
    ) -> Result<TriggerActiveRunState, TriggerError> {
        let mut results = self.active_run_states(vec![request]).await;
        results.pop().unwrap_or(Ok(TriggerActiveRunState::Missing))
    }

    async fn active_run_states(
        &self,
        requests: Vec<TriggerActiveRunStateRequest>,
    ) -> Vec<Result<TriggerActiveRunState, TriggerError>> {
        if requests.is_empty() {
            return Vec::new();
        }
        let lookup_request = ProcessLifecycleLookupBatchRequest {
            processes: requests
                .iter()
                .map(|request| ProcessLifecycleLookupRequest {
                    tenant_id: request.tenant_id.clone(),
                    process_id: process_id_from_turn_run_id(request.run_id),
                })
                .collect(),
        };
        self.lifecycle_source
            .process_lifecycle_states(lookup_request)
            .await
            .into_iter()
            .map(|result| {
                result
                    .map(active_run_state_from_process_lifecycle)
                    .map_err(trigger_backend_error)
            })
            .collect()
    }
}

fn process_id_from_turn_run_id(run_id: TurnRunId) -> ProcessId {
    ProcessId::from_uuid(run_id.as_uuid())
}

fn active_run_state_from_process_lifecycle(
    result: ProcessLifecycleLookupResult,
) -> TriggerActiveRunState {
    match result {
        ProcessLifecycleLookupResult::Missing => TriggerActiveRunState::Missing,
        ProcessLifecycleLookupResult::Found { status, suspension } => {
            if status.is_terminal() {
                TriggerActiveRunState::Terminal {
                    status: terminal_process_history_status(status),
                }
            } else if status == ProcessLifecycleStatus::Suspended {
                TriggerActiveRunState::Blocked {
                    kind: blocked_active_process_kind(
                        suspension.as_ref().map(|suspension| suspension.kind),
                    ),
                }
            } else {
                TriggerActiveRunState::Nonterminal
            }
        }
    }
}

fn terminal_process_history_status(status: ProcessLifecycleStatus) -> TriggerRunHistoryStatus {
    debug_assert!(
        status.is_terminal(),
        "only terminal process statuses should be normalized into run-history status"
    );
    match status {
        ProcessLifecycleStatus::Completed | ProcessLifecycleStatus::Stopped => {
            TriggerRunHistoryStatus::Ok
        }
        ProcessLifecycleStatus::Cancelled
        | ProcessLifecycleStatus::Failed
        | ProcessLifecycleStatus::Killed
        | ProcessLifecycleStatus::RecoveryRequired => TriggerRunHistoryStatus::Error,
        ProcessLifecycleStatus::Queued
        | ProcessLifecycleStatus::Running
        | ProcessLifecycleStatus::Suspended
        | ProcessLifecycleStatus::StopRequested
        | ProcessLifecycleStatus::CancelRequested => TriggerRunHistoryStatus::Error,
    }
}

/// User-facing hold granularity for a gate-parked run (#5886): approval and
/// auth get specific copy; the remaining blocked states share a generic one.
fn blocked_active_process_kind(kind: Option<ProcessSuspensionKind>) -> BlockedActiveRunKind {
    match kind {
        Some(ProcessSuspensionKind::Approval) => BlockedActiveRunKind::Approval,
        Some(ProcessSuspensionKind::Authorization) => BlockedActiveRunKind::Auth,
        _ => BlockedActiveRunKind::Other,
    }
}

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
    use ironclaw_processes::ProcessSuspension;
    use ironclaw_triggers::TriggerId;
    use ironclaw_turns::TurnRunId;

    #[derive(Default)]
    struct CountingLifecycleSource {
        calls: std::sync::Mutex<usize>,
    }

    impl CountingLifecycleSource {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("lifecycle calls lock")
        }
    }

    #[async_trait]
    impl ProcessLifecycleLookupSource for CountingLifecycleSource {
        type Error = TurnError;

        async fn process_lifecycle_states(
            &self,
            request: ProcessLifecycleLookupBatchRequest,
        ) -> Vec<Result<ProcessLifecycleLookupResult, Self::Error>> {
            *self.calls.lock().expect("lifecycle calls lock") += 1;
            request
                .processes
                .into_iter()
                .map(|_| Ok(ProcessLifecycleLookupResult::Missing))
                .collect()
        }
    }

    struct StaticLifecycleSource {
        processes: Vec<LifecycleFixture>,
    }

    struct LifecycleFixture {
        tenant_id: TenantId,
        process_id: ProcessId,
        status: ProcessLifecycleStatus,
        suspension: Option<ProcessSuspension>,
    }

    #[async_trait]
    impl ProcessLifecycleLookupSource for StaticLifecycleSource {
        type Error = TurnError;

        async fn process_lifecycle_states(
            &self,
            request: ProcessLifecycleLookupBatchRequest,
        ) -> Vec<Result<ProcessLifecycleLookupResult, Self::Error>> {
            request
                .processes
                .into_iter()
                .map(|lookup| {
                    let result = self
                        .processes
                        .iter()
                        .find(|fixture| {
                            fixture.tenant_id == lookup.tenant_id
                                && fixture.process_id == lookup.process_id
                        })
                        .map(|fixture| ProcessLifecycleLookupResult::Found {
                            status: fixture.status,
                            suspension: fixture.suspension.clone(),
                        })
                        .unwrap_or(ProcessLifecycleLookupResult::Missing);
                    Ok(result)
                })
                .collect()
        }
    }

    #[derive(Default)]
    struct FailingLifecycleSource {
        calls: std::sync::Mutex<usize>,
    }

    impl FailingLifecycleSource {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("lifecycle calls lock")
        }
    }

    #[async_trait]
    impl ProcessLifecycleLookupSource for FailingLifecycleSource {
        type Error = TurnError;

        async fn process_lifecycle_states(
            &self,
            request: ProcessLifecycleLookupBatchRequest,
        ) -> Vec<Result<ProcessLifecycleLookupResult, Self::Error>> {
            *self.calls.lock().expect("lifecycle calls lock") += 1;
            request
                .processes
                .into_iter()
                .map(|_| {
                    Err(ironclaw_turns::TurnError::Unavailable {
                        reason: "lifecycle failed".to_string(),
                    })
                })
                .collect()
        }
    }

    #[test]
    fn terminal_process_statuses_map_to_run_history_statuses() {
        let cases = [
            (
                ProcessLifecycleStatus::Completed,
                TriggerRunHistoryStatus::Ok,
            ),
            (ProcessLifecycleStatus::Stopped, TriggerRunHistoryStatus::Ok),
            (
                ProcessLifecycleStatus::Cancelled,
                TriggerRunHistoryStatus::Error,
            ),
            (
                ProcessLifecycleStatus::Failed,
                TriggerRunHistoryStatus::Error,
            ),
            (
                ProcessLifecycleStatus::Killed,
                TriggerRunHistoryStatus::Error,
            ),
            (
                ProcessLifecycleStatus::RecoveryRequired,
                TriggerRunHistoryStatus::Error,
            ),
        ];

        for (process_status, expected) in cases {
            assert_eq!(terminal_process_history_status(process_status), expected);
        }
    }

    #[tokio::test]
    async fn active_run_batch_lookup_uses_one_lifecycle_lookup_for_page() {
        let lifecycle_source = Arc::new(CountingLifecycleSource::default());
        let lookup = ProcessActiveRunLookup::new(lifecycle_source.clone());
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

        assert_eq!(lifecycle_source.calls(), 1);
        assert_eq!(results.len(), 2);
        assert!(
            results
                .into_iter()
                .all(|result| matches!(result, Ok(TriggerActiveRunState::Missing)))
        );
    }

    #[tokio::test]
    async fn active_run_batch_lookup_returns_nonterminal_and_terminal_states_from_lifecycle() {
        let tenant_id = TenantId::new("trigger-active-state-tenant").expect("tenant id");
        let nonterminal_run_id = TurnRunId::new();
        let terminal_run_id = TurnRunId::new();
        let missing_run_id = TurnRunId::new();
        let lifecycle_source = Arc::new(StaticLifecycleSource {
            processes: vec![
                lifecycle_fixture(
                    &tenant_id,
                    nonterminal_run_id,
                    ProcessLifecycleStatus::Running,
                    None,
                ),
                lifecycle_fixture(
                    &tenant_id,
                    terminal_run_id,
                    ProcessLifecycleStatus::Completed,
                    None,
                ),
            ],
        });
        let lookup = ProcessActiveRunLookup::new(lifecycle_source);
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
        let lifecycle_source = Arc::new(StaticLifecycleSource {
            processes: vec![
                lifecycle_fixture(
                    &tenant_id,
                    approval_run,
                    ProcessLifecycleStatus::Suspended,
                    process_suspension(ProcessSuspensionKind::Approval),
                ),
                lifecycle_fixture(
                    &tenant_id,
                    auth_run,
                    ProcessLifecycleStatus::Suspended,
                    process_suspension(ProcessSuspensionKind::Authorization),
                ),
                lifecycle_fixture(
                    &tenant_id,
                    resource_run,
                    ProcessLifecycleStatus::Suspended,
                    process_suspension(ProcessSuspensionKind::Resource),
                ),
                lifecycle_fixture(
                    &tenant_id,
                    dependent_run,
                    ProcessLifecycleStatus::Suspended,
                    process_suspension(ProcessSuspensionKind::AwaitingChildProcess),
                ),
            ],
        });
        let lookup = ProcessActiveRunLookup::new(lifecycle_source);
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

        // Blocked runs keep the active-fire lock (back-pressure) exactly like
        // Nonterminal ones — the kind only feeds read-surface copy (#5886).
        assert!(matches!(
            results[0],
            Ok(TriggerActiveRunState::Blocked {
                kind: BlockedActiveRunKind::Approval
            })
        ));
        assert!(matches!(
            results[1],
            Ok(TriggerActiveRunState::Blocked {
                kind: BlockedActiveRunKind::Auth
            })
        ));
        assert!(matches!(
            results[2],
            Ok(TriggerActiveRunState::Blocked {
                kind: BlockedActiveRunKind::Other
            })
        ));
        assert!(matches!(
            results[3],
            Ok(TriggerActiveRunState::Blocked {
                kind: BlockedActiveRunKind::Other
            })
        ));
    }

    #[tokio::test]
    async fn active_run_batch_lookup_returns_empty_without_lifecycle_read() {
        let lifecycle_source = Arc::new(CountingLifecycleSource::default());
        let lookup = ProcessActiveRunLookup::new(lifecycle_source.clone());

        let results = lookup.active_run_states(Vec::new()).await;

        assert!(results.is_empty());
        assert_eq!(lifecycle_source.calls(), 0);
    }

    #[tokio::test]
    async fn lifecycle_source_error_fans_out_to_all_batch_results() {
        let lifecycle_source = Arc::new(FailingLifecycleSource::default());
        let lookup = ProcessActiveRunLookup::new(lifecycle_source.clone());
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

        assert_eq!(lifecycle_source.calls(), 1);
        assert_eq!(results.len(), 2);
        assert!(results.into_iter().all(|result| matches!(
            result,
            Err(TriggerError::Backend { reason }) if reason.contains("lifecycle failed")
        )));
    }

    fn lifecycle_fixture(
        tenant_id: &TenantId,
        run_id: TurnRunId,
        status: ProcessLifecycleStatus,
        suspension: Option<ProcessSuspension>,
    ) -> LifecycleFixture {
        LifecycleFixture {
            tenant_id: tenant_id.clone(),
            process_id: process_id_from_turn_run_id(run_id),
            status,
            suspension,
        }
    }

    fn process_suspension(kind: ProcessSuspensionKind) -> Option<ProcessSuspension> {
        Some(ProcessSuspension {
            kind,
            gate_ref: None,
            activity_id: None,
            credential_requirements: Vec::new(),
            detail: None,
        })
    }
}
