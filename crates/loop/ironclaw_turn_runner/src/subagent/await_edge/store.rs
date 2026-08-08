//! Runner projection over process-journal dependencies.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::ids::ProcessId;
use ironclaw_host_api::turn::{TurnRunId, TurnScope};
use ironclaw_processes::{
    CloseProcessDependencyRequest, ProcessDependencyPort, ProcessDependencyQuery,
    ProcessDependencyRecord, ProcessDependencyState, ProcessJournalStoreError,
    ProcessLifecycleStatus, ProcessTerminalEvidence, SettleProcessDependencyRequest,
};

use super::{
    AwaitEdge, AwaitEdgeState, AwaitEdgeStoreError, EdgeTerminalKind, ReservationReleaseState,
};

pub struct AwaitEdgeStore {
    dependencies: Arc<dyn ProcessDependencyPort<Error = ProcessJournalStoreError>>,
}

impl AwaitEdgeStore {
    pub fn new(
        dependencies: Arc<dyn ProcessDependencyPort<Error = ProcessJournalStoreError>>,
    ) -> Self {
        Self { dependencies }
    }

    fn process_id(run_id: TurnRunId) -> ProcessId {
        ProcessId::from_uuid(run_id.as_uuid())
    }

    fn run_id(process_id: ProcessId) -> TurnRunId {
        TurnRunId::from_uuid(process_id.as_uuid())
    }

    fn query(
        scope: &TurnScope,
        parent_run_id: Option<TurnRunId>,
        group_ref: Option<String>,
        include_closed: bool,
    ) -> ProcessDependencyQuery {
        ProcessDependencyQuery {
            scope: scope.to_resource_scope(),
            dependent_process_id: parent_run_id.map(Self::process_id),
            group_ref,
            include_closed,
        }
    }

    fn edge_from_record(record: ProcessDependencyRecord) -> Result<AwaitEdge, AwaitEdgeStoreError> {
        let mut edge = match serde_json::from_value::<AwaitEdge>(record.metadata.clone()) {
            Ok(edge) => edge,
            Err(_) => {
                let submitted: ironclaw_loop_host::AwaitedChildSetRecord =
                    serde_json::from_value(record.metadata).map_err(|error| {
                        AwaitEdgeStoreError::Backend {
                            reason: format!(
                                "process dependency metadata deserialize failed: {error}"
                            ),
                        }
                    })?;
                AwaitEdge {
                    child_scope: submitted.child_scope,
                    child_thread_id: submitted.child_thread_id,
                    parent_thread_id: submitted.parent_run_context.thread_id.clone(),
                    parent_run_context: submitted.parent_run_context,
                    tree_root_run_id: submitted.tree_root_run_id,
                    gate_ref: submitted.gate_ref,
                    source_binding_ref: submitted.source_binding_ref,
                    reply_target_binding_ref: submitted.reply_target_binding_ref,
                    subagent_kind: submitted.subagent_kind,
                    spawn_capability_id: submitted.spawn_capability_id,
                    spawn_provider_call_id: submitted.spawn_provider_call_id,
                    result_ref: submitted.result_ref,
                    mode: submitted.mode,
                    state: AwaitEdgeState::Open,
                    terminal_kind: None,
                    terminal_byte_len: None,
                    terminal_reason: None,
                    reservation_release: ReservationReleaseState::Unclaimed,
                    created_at: record.created_at,
                    settled_at: None,
                }
            }
        };
        edge.state = match record.state {
            ProcessDependencyState::Open => AwaitEdgeState::Open,
            ProcessDependencyState::Settled => AwaitEdgeState::Settled,
            ProcessDependencyState::Consumed => AwaitEdgeState::Drained,
            ProcessDependencyState::Abandoned => AwaitEdgeState::Abandoned,
        };
        edge.reservation_release = if matches!(
            record.state,
            ProcessDependencyState::Consumed | ProcessDependencyState::Abandoned
        ) {
            ReservationReleaseState::Released
        } else {
            ReservationReleaseState::Unclaimed
        };
        edge.settled_at = record.settled_at;
        if let Some(terminal) = record.terminal {
            edge.terminal_kind = EdgeTerminalKind::from_process_status(terminal.status);
            edge.terminal_byte_len = terminal.output_bytes;
            edge.terminal_reason = terminal.sanitized_reason;
        }
        Ok(edge)
    }

    pub async fn abandon(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), AwaitEdgeStoreError> {
        self.dependencies
            .abandon_process_dependency(CloseProcessDependencyRequest {
                dependent_process_id: Self::process_id(parent_run_id),
                dependency_process_id: Self::process_id(child_run_id),
                scope: scope.to_resource_scope(),
                closed_at: Utc::now(),
            })
            .await
            .map(|_| ())
            .map_err(map_process_error)
    }

    pub async fn settle(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        terminal_kind: EdgeTerminalKind,
        terminal_byte_len: Option<u64>,
        terminal_reason: Option<String>,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        self.dependencies
            .settle_process_dependency(SettleProcessDependencyRequest {
                dependent_process_id: Self::process_id(parent_run_id),
                dependency_process_id: Self::process_id(child_run_id),
                scope: scope.to_resource_scope(),
                terminal: ProcessTerminalEvidence {
                    status: terminal_kind.to_process_status(),
                    output_bytes: terminal_byte_len,
                    sanitized_reason: terminal_reason,
                },
                settled_at: Utc::now(),
            })
            .await
            .map_err(map_process_error)?
            .map(Self::edge_from_record)
            .transpose()
    }

    pub async fn list_group(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        gate_ref: &ironclaw_host_api::turn::TurnGateRef,
    ) -> Result<Vec<(TurnRunId, AwaitEdge)>, AwaitEdgeStoreError> {
        self.dependencies
            .query_process_dependencies(Self::query(
                scope,
                Some(parent_run_id),
                Some(gate_ref.as_str().to_string()),
                false,
            ))
            .await
            .map_err(map_process_error)?
            .into_iter()
            .map(|record| {
                let child_run_id = Self::run_id(record.dependency_process_id);
                Self::edge_from_record(record).map(|edge| (child_run_id, edge))
            })
            .collect()
    }

    pub async fn peek(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        let child_process_id = Self::process_id(child_run_id);
        self.dependencies
            .query_process_dependencies(Self::query(scope, Some(parent_run_id), None, false))
            .await
            .map_err(map_process_error)?
            .into_iter()
            .find(|record| record.dependency_process_id == child_process_id)
            .map(Self::edge_from_record)
            .transpose()
    }

    pub async fn list_unclosed_for_scope(
        &self,
        scope: &TurnScope,
    ) -> Result<Vec<(TurnRunId, TurnRunId, AwaitEdge)>, AwaitEdgeStoreError> {
        self.dependencies
            .query_process_dependencies(Self::query(scope, None, None, false))
            .await
            .map_err(map_process_error)?
            .into_iter()
            .map(|record| {
                let parent = Self::run_id(record.dependent_process_id);
                let child = Self::run_id(record.dependency_process_id);
                Self::edge_from_record(record).map(|edge| (parent, child, edge))
            })
            .collect()
    }

    pub async fn consume(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), AwaitEdgeStoreError> {
        self.dependencies
            .consume_process_dependency(CloseProcessDependencyRequest {
                dependent_process_id: Self::process_id(parent_run_id),
                dependency_process_id: Self::process_id(child_run_id),
                scope: scope.to_resource_scope(),
                closed_at: Utc::now(),
            })
            .await
            .map(|_| ())
            .map_err(map_process_error)
    }

    pub async fn close(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), AwaitEdgeStoreError> {
        let Some(edge) = self.peek(scope, parent_run_id, child_run_id).await? else {
            return Ok(());
        };
        match edge.state {
            AwaitEdgeState::Settled => self.consume(scope, parent_run_id, child_run_id).await,
            AwaitEdgeState::Open | AwaitEdgeState::Drained | AwaitEdgeState::Abandoned => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl ironclaw_loop_host::AwaitEdgeWriter for AwaitEdgeStore {
    async fn abandon_awaited_child(
        &self,
        child_scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), ironclaw_loop_contracts::AgentLoopHostError> {
        self.abandon(child_scope, parent_run_id, child_run_id)
            .await
            .map_err(super::map_await_edge_error)
    }
}

#[async_trait::async_trait]
impl crate::loop_exit_applier::AwaitDependentRunEvidenceStore for AwaitEdgeStore {
    async fn has_awaited_child_gate(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &ironclaw_host_api::turn::LoopGateRef,
    ) -> Result<bool, ironclaw_turns::TurnError> {
        let gate_ref =
            ironclaw_host_api::turn::TurnGateRef::new(gate_ref.as_str()).map_err(|reason| {
                ironclaw_turns::TurnError::InvalidRequest {
                    reason: format!("awaited child gate evidence has invalid gate ref: {reason}"),
                }
            })?;
        let group = self
            .list_group(scope, run_id, &gate_ref)
            .await
            .map_err(|error| ironclaw_turns::TurnError::Unavailable {
                reason: error.to_string(),
            })?;
        Ok(group
            .iter()
            .any(|(_, edge)| edge.mode == ironclaw_loop_host::SpawnSubagentMode::Blocking))
    }
}

impl EdgeTerminalKind {
    fn to_process_status(self) -> ProcessLifecycleStatus {
        match self {
            Self::Completed => ProcessLifecycleStatus::Completed,
            Self::Failed => ProcessLifecycleStatus::Failed,
            Self::Cancelled => ProcessLifecycleStatus::Cancelled,
            Self::RecoveryRequired => ProcessLifecycleStatus::RecoveryRequired,
        }
    }

    fn from_process_status(status: ProcessLifecycleStatus) -> Option<Self> {
        match status {
            ProcessLifecycleStatus::Completed => Some(Self::Completed),
            ProcessLifecycleStatus::Failed => Some(Self::Failed),
            ProcessLifecycleStatus::Cancelled => Some(Self::Cancelled),
            ProcessLifecycleStatus::RecoveryRequired => Some(Self::RecoveryRequired),
            _ => None,
        }
    }
}

fn map_process_error(error: ironclaw_processes::ProcessJournalStoreError) -> AwaitEdgeStoreError {
    AwaitEdgeStoreError::Backend {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_host_api::ids::{AgentId, CapabilityId, ProcessId, TenantId, ThreadId, UserId};
    use ironclaw_host_api::turn::{
        LoopResultRef, ReplyTargetBindingRef, SourceBindingRef, TurnActor, TurnGateRef, TurnRunId,
        TurnScope,
    };
    use ironclaw_loop_host::{AwaitedChildSetRecord, SpawnSubagentMode, SubagentKindId};
    use ironclaw_processes::{ProcessDependencyRecord, ProcessDependencyState};

    use super::*;

    fn edge_fixture() -> (TurnRunId, TurnRunId, AwaitEdge) {
        let tenant_id = TenantId::new("store-transition-tenant").expect("tenant");
        let user_id = UserId::new("store-transition-user").expect("user");
        let agent_id = AgentId::new("store-transition-agent").expect("agent");
        let parent_thread_id =
            ThreadId::new("store-transition-parent-thread").expect("parent thread");
        let child_thread_id = ThreadId::new("store-transition-child-thread").expect("child thread");
        let parent_run_id = TurnRunId::new();
        let child_run_id = TurnRunId::new();
        let parent_scope = TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(agent_id.clone()),
            None,
            parent_thread_id.clone(),
            Some(user_id.clone()),
        );
        let child_scope = TurnScope::new_with_owner(
            tenant_id,
            Some(agent_id),
            None,
            child_thread_id.clone(),
            Some(user_id.clone()),
        );
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("await-store-transition");
        parent_run_context.scope = parent_scope;
        parent_run_context.thread_id = parent_thread_id.clone();
        parent_run_context.run_id = parent_run_id;
        parent_run_context.actor = Some(TurnActor::new(user_id));
        (
            parent_run_id,
            child_run_id,
            AwaitEdge {
                child_scope,
                child_thread_id,
                parent_thread_id,
                parent_run_context,
                tree_root_run_id: parent_run_id,
                gate_ref: TurnGateRef::new("gate:store-transition").expect("gate"),
                source_binding_ref: SourceBindingRef::new("source:store-transition")
                    .expect("source"),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:store-transition")
                    .expect("reply"),
                subagent_kind: SubagentKindId::new("general").expect("kind"),
                spawn_capability_id: CapabilityId::new(
                    ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
                )
                .expect("capability"),
                spawn_provider_call_id: Some("spawn-call-store-transition".to_string()),
                result_ref: LoopResultRef::new("result:store-transition").expect("result"),
                mode: SpawnSubagentMode::Blocking,
                state: AwaitEdgeState::Open,
                terminal_kind: None,
                terminal_byte_len: None,
                terminal_reason: None,
                reservation_release: ReservationReleaseState::Unclaimed,
                created_at: Utc::now(),
                settled_at: None,
            },
        )
    }

    fn record(
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        edge: &AwaitEdge,
        state: ProcessDependencyState,
        status: ProcessLifecycleStatus,
    ) -> ProcessDependencyRecord {
        ProcessDependencyRecord {
            dependent_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
            dependency_process_id: ProcessId::from_uuid(child_run_id.as_uuid()),
            root_process_id: ProcessId::from_uuid(parent_run_id.as_uuid()),
            scope: edge.child_scope.to_resource_scope(),
            group_ref: Some(edge.gate_ref.as_str().to_string()),
            state,
            terminal: Some(ProcessTerminalEvidence {
                status,
                output_bytes: Some(42),
                sanitized_reason: Some("terminal-reason".to_string()),
            }),
            created_at: edge.created_at,
            settled_at: Some(Utc::now()),
            consumed_at: None,
            metadata: serde_json::to_value(edge).expect("serialize edge"),
        }
    }

    #[test]
    fn edge_projection_matrix_covers_every_dependency_and_terminal_state() {
        let (parent_run_id, child_run_id, edge) = edge_fixture();
        let cases = [
            (
                ProcessDependencyState::Open,
                AwaitEdgeState::Open,
                ReservationReleaseState::Unclaimed,
                ProcessLifecycleStatus::Completed,
                Some(EdgeTerminalKind::Completed),
            ),
            (
                ProcessDependencyState::Settled,
                AwaitEdgeState::Settled,
                ReservationReleaseState::Unclaimed,
                ProcessLifecycleStatus::Failed,
                Some(EdgeTerminalKind::Failed),
            ),
            (
                ProcessDependencyState::Consumed,
                AwaitEdgeState::Drained,
                ReservationReleaseState::Released,
                ProcessLifecycleStatus::Cancelled,
                Some(EdgeTerminalKind::Cancelled),
            ),
            (
                ProcessDependencyState::Abandoned,
                AwaitEdgeState::Abandoned,
                ReservationReleaseState::Released,
                ProcessLifecycleStatus::RecoveryRequired,
                Some(EdgeTerminalKind::RecoveryRequired),
            ),
            (
                ProcessDependencyState::Open,
                AwaitEdgeState::Open,
                ReservationReleaseState::Unclaimed,
                ProcessLifecycleStatus::Running,
                None,
            ),
        ];

        for (state, expected_state, expected_release, terminal, expected_terminal) in cases {
            let projected = AwaitEdgeStore::edge_from_record(record(
                parent_run_id,
                child_run_id,
                &edge,
                state,
                terminal,
            ))
            .expect("project edge");
            assert_eq!(projected.state, expected_state);
            assert_eq!(projected.reservation_release, expected_release);
            assert_eq!(projected.terminal_kind, expected_terminal);
            assert_eq!(projected.terminal_byte_len, Some(42));
            assert_eq!(
                projected.terminal_reason.as_deref(),
                Some("terminal-reason")
            );
        }
    }

    #[test]
    fn legacy_edge_metadata_fallback_and_malformed_metadata_fail_closed() {
        let (parent_run_id, child_run_id, edge) = edge_fixture();
        let submitted = AwaitedChildSetRecord {
            gate_ref: edge.gate_ref.clone(),
            parent_run_context: edge.parent_run_context.clone(),
            tree_root_run_id: edge.tree_root_run_id,
            child_scope: edge.child_scope.clone(),
            child_run_id,
            child_thread_id: edge.child_thread_id.clone(),
            source_binding_ref: edge.source_binding_ref.clone(),
            reply_target_binding_ref: edge.reply_target_binding_ref.clone(),
            subagent_kind: edge.subagent_kind.clone(),
            spawn_capability_id: edge.spawn_capability_id.clone(),
            spawn_provider_call_id: edge.spawn_provider_call_id.clone(),
            result_ref: edge.result_ref.clone(),
            mode: edge.mode,
        };
        let mut legacy = record(
            parent_run_id,
            child_run_id,
            &edge,
            ProcessDependencyState::Open,
            ProcessLifecycleStatus::Completed,
        );
        legacy.metadata = serde_json::to_value(submitted).expect("serialize legacy metadata");
        let projected = AwaitEdgeStore::edge_from_record(legacy).expect("project legacy edge");
        assert_eq!(projected.state, AwaitEdgeState::Open);
        assert_eq!(projected.gate_ref, edge.gate_ref);

        let mut malformed = record(
            parent_run_id,
            child_run_id,
            &edge,
            ProcessDependencyState::Open,
            ProcessLifecycleStatus::Completed,
        );
        malformed.metadata = serde_json::json!({"unexpected": true});
        assert!(AwaitEdgeStore::edge_from_record(malformed).is_err());
    }
}
