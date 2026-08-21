//! Runner projection over process-journal dependencies.

use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_host_api::turn::{LoopMessageRef, TurnRunId, TurnScope};
use ironclaw_host_api::{artifact::CompletedArtifact, ids::ProcessId};
use ironclaw_processes::{
    ClaimProcessDependencySettlementRequest, CloseProcessDependencyRequest,
    CompleteProcessDependencySettlementRequest, ProcessDependencyPort, ProcessDependencyQuery,
    ProcessDependencyRecord, ProcessDependencyState, ProcessJournalStoreError,
    ProcessLifecycleStatus, ProcessTerminalEvidence, TransitionProcessDependencyRequest,
};

use super::{
    AttentionOutcome, AwaitEdge, AwaitEdgeState, AwaitEdgeStoreError, EdgeTerminalKind,
    ReservationReleaseState,
};

pub struct AwaitEdgeStore {
    dependencies: Arc<dyn ProcessDependencyPort<Error = ProcessJournalStoreError>>,
}

pub enum SettlementClaim {
    Acquired {
        edge: AwaitEdge,
        claim_token: String,
    },
    Pending,
    Completed {
        edge: AwaitEdge,
        artifact: CompletedArtifact,
    },
}

pub(super) struct SettlementCompletion {
    pub terminal_kind: EdgeTerminalKind,
    pub terminal_byte_len: Option<u64>,
    pub terminal_reason: Option<String>,
    pub completed_artifact: CompletedArtifact,
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

    /// Rebuild the edge from the one blob production ever writes into
    /// dependency metadata: `AwaitedChildSetRecord` (`subagent_spawn_port.rs`),
    /// which the delivery-chain CAS then merges `appended_message_ref` and
    /// `attention_outcome` into as sibling top-level keys — never replacing the
    /// shape, only widening it. Those two keys are read back off the raw blob
    /// because `AwaitedChildSetRecord` does not carry them.
    fn edge_from_record(record: ProcessDependencyRecord) -> Result<AwaitEdge, AwaitEdgeStoreError> {
        let appended_message_ref = record
            .metadata
            .get("appended_message_ref")
            .cloned()
            .map(serde_json::from_value::<LoopMessageRef>)
            .transpose()
            .map_err(|error| AwaitEdgeStoreError::Backend {
                reason: format!("appended_message_ref deserialize failed: {error}"),
            })?;
        let attention_outcome = record
            .metadata
            .get("attention_outcome")
            .cloned()
            .map(serde_json::from_value::<AttentionOutcome>)
            .transpose()
            .map_err(|error| AwaitEdgeStoreError::Backend {
                reason: format!("attention_outcome deserialize failed: {error}"),
            })?;
        let submitted: ironclaw_loop_host::AwaitedChildSetRecord =
            serde_json::from_value(record.metadata).map_err(|error| {
                AwaitEdgeStoreError::Backend {
                    reason: format!("process dependency metadata deserialize failed: {error}"),
                }
            })?;
        let mut edge = AwaitEdge {
            child_scope: submitted.child_scope,
            child_thread_id: submitted.child_thread_id,
            parent_thread_id: submitted.parent_run_context.thread_id.clone(),
            parent_run_context: submitted.parent_run_context,
            tree_root_run_id: submitted.tree_root_run_id,
            gate_ref: submitted.gate_ref,
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
            appended_message_ref,
            attention_outcome,
            created_at: record.created_at,
            settled_at: None,
        };
        edge.state = match record.state {
            ProcessDependencyState::Open | ProcessDependencyState::Settling => AwaitEdgeState::Open,
            ProcessDependencyState::Settled => AwaitEdgeState::Settled,
            ProcessDependencyState::ResultAppended => AwaitEdgeState::ResultAppended,
            ProcessDependencyState::AttentionScheduled => AwaitEdgeState::AttentionScheduled,
            // The kernel keeps this name domain-neutral; the loop tier is where
            // "which cap deferred it" is knowable, so the spelling differs.
            ProcessDependencyState::AttentionDeferred => AwaitEdgeState::AttentionDeferredStreakCap,
            ProcessDependencyState::Consumed => AwaitEdgeState::Drained,
            ProcessDependencyState::Abandoned => AwaitEdgeState::Abandoned,
        };
        edge.reservation_release = if record.state.is_closed() {
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

    pub async fn claim_settlement(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<SettlementClaim, AwaitEdgeStoreError> {
        let claimed_at = Utc::now();
        let claim_token = uuid::Uuid::new_v4().to_string();
        let record = self
            .dependencies
            .claim_process_dependency_settlement(ClaimProcessDependencySettlementRequest {
                dependent_process_id: Self::process_id(parent_run_id),
                dependency_process_id: Self::process_id(child_run_id),
                scope: scope.to_resource_scope(),
                claim_token: claim_token.clone(),
                claimed_at,
                lease_expires_at: claimed_at + Duration::seconds(30),
            })
            .await
            .map_err(map_process_error)?
            .ok_or_else(|| AwaitEdgeStoreError::Backend {
                reason: "process dependency disappeared during settlement claim".to_string(),
            })?;
        if let Some(artifact) = record.completed_artifact.clone() {
            return Ok(SettlementClaim::Completed {
                edge: Self::edge_from_record(record)?,
                artifact,
            });
        }
        let acquired = record
            .settlement
            .as_ref()
            .is_some_and(|claim| claim.claim_token == claim_token);
        if acquired {
            Ok(SettlementClaim::Acquired {
                edge: Self::edge_from_record(record)?,
                claim_token,
            })
        } else {
            Ok(SettlementClaim::Pending)
        }
    }

    pub(super) async fn complete_settlement(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        claim_token: String,
        completion: SettlementCompletion,
    ) -> Result<SettlementClaim, AwaitEdgeStoreError> {
        let record = self
            .dependencies
            .complete_process_dependency_settlement(CompleteProcessDependencySettlementRequest {
                dependent_process_id: Self::process_id(parent_run_id),
                dependency_process_id: Self::process_id(child_run_id),
                scope: scope.to_resource_scope(),
                claim_token,
                terminal: ProcessTerminalEvidence {
                    status: completion.terminal_kind.to_process_status(),
                    output_bytes: completion.terminal_byte_len,
                    sanitized_reason: completion.terminal_reason,
                },
                completed_artifact: completion.completed_artifact,
                settled_at: Utc::now(),
            })
            .await
            .map_err(map_process_error)?
            .ok_or_else(|| AwaitEdgeStoreError::Backend {
                reason: "process dependency disappeared during settlement completion".to_string(),
            })?;
        let artifact = record.completed_artifact.clone();
        let edge = Self::edge_from_record(record)?;
        match artifact {
            Some(artifact) => Ok(SettlementClaim::Completed { edge, artifact }),
            None => Ok(SettlementClaim::Pending),
        }
    }

    /// `Settled -> ResultAppended`, recording the parent-thread message the
    /// child's result landed as. Replaying an append that already landed
    /// returns the ref already recorded rather than writing a second one.
    pub async fn record_result_appended(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        message_ref: LoopMessageRef,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        self.transition(
            scope,
            parent_run_id,
            child_run_id,
            ProcessDependencyState::ResultAppended,
            Some(serde_json::json!({"appended_message_ref": message_ref.as_str()})),
        )
        .await
    }

    /// `ResultAppended | AttentionDeferred -> AttentionScheduled`, recording how
    /// the parent was made attentive.
    ///
    /// Two predecessors are legal, because draining a parked edge *is*
    /// scheduling attention (design §4.1/§4.2: a streak-capped edge stays
    /// unclosed "until a permitted or human-initiated run start drains it").
    /// Without the second one `AttentionDeferredStreakCap` would be a dead end:
    /// no forward path, and `consume` takes only `Settled | AttentionScheduled`.
    /// The kernel owns that relation, so this caller names only its target.
    pub async fn record_attention(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        outcome: AttentionOutcome,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        let outcome_value =
            serde_json::to_value(outcome).map_err(|error| AwaitEdgeStoreError::Backend {
                reason: format!("attention outcome serialize failed: {error}"),
            })?;
        self.transition(
            scope,
            parent_run_id,
            child_run_id,
            ProcessDependencyState::AttentionScheduled,
            Some(serde_json::json!({"attention_outcome": outcome_value})),
        )
        .await
    }

    /// `ResultAppended -> AttentionDeferred`: the result is durably appended
    /// but the parent hit its consecutive-interruption cap, so attention is
    /// parked. The edge stays unclosed and claimable.
    pub async fn defer_streak_capped(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        self.transition(
            scope,
            parent_run_id,
            child_run_id,
            ProcessDependencyState::AttentionDeferred,
            None,
        )
        .await
    }

    /// One CAS over the kernel's state column: the write lands only if the
    /// stored state is a legal predecessor of `next`. `metadata` is merged into
    /// the stored blob — which *is* the serialized edge — never substituted for
    /// it.
    async fn transition(
        &self,
        scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
        next: ProcessDependencyState,
        metadata: Option<serde_json::Value>,
    ) -> Result<Option<AwaitEdge>, AwaitEdgeStoreError> {
        self.dependencies
            .transition_process_dependency(TransitionProcessDependencyRequest {
                dependent_process_id: Self::process_id(parent_run_id),
                dependency_process_id: Self::process_id(child_run_id),
                scope: scope.to_resource_scope(),
                next,
                metadata,
                transitioned_at: Utc::now(),
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
            // The two states the kernel will consume — closing releases the
            // descendant reservation in the same journal command.
            AwaitEdgeState::Settled | AwaitEdgeState::AttentionScheduled => {
                self.consume(scope, parent_run_id, child_run_id).await
            }
            // Still in flight. `ResultAppended` has no attention recorded yet
            // and a streak-capped edge is parked for a later sweep; the kernel
            // refuses to consume either, because closing would strand the
            // parent with an undelivered result.
            AwaitEdgeState::Open
            | AwaitEdgeState::ResultAppended
            | AwaitEdgeState::AttentionDeferredStreakCap
            | AwaitEdgeState::Drained
            | AwaitEdgeState::Abandoned => Ok(()),
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
mod tests;
