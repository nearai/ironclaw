use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_loop_support::{AwaitedChildSetRecord, SubagentGateResolutionStore, SubagentKindId};
use ironclaw_turns::{
    EventCursor, GateRef, TurnEventKind, TurnRunId, TurnStatus,
    run_profile::{AgentLoopHostError, AgentLoopHostErrorKind},
};

const MAX_GATE_RECORDS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitedChildState {
    pub record: AwaitedChildSetRecord,
    pub terminal_status: Option<TurnStatus>,
    pub terminal_event: Option<AwaitedChildTerminalEvent>,
    pub descendant_reservation_release_claimed: bool,
    pub descendant_reservation_released: bool,
    pub delivery_claimed: bool,
    pub delivered_to_parent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitedChildTerminalEvent {
    pub status: TurnStatus,
    pub kind: TurnEventKind,
    pub cursor: EventCursor,
    pub sanitized_reason: Option<String>,
    pub owner_user_id: Option<UserId>,
}

#[derive(Default)]
pub struct BoundedSubagentGateResolutionStore {
    inner: std::sync::Mutex<GateResolutionInner>,
}

#[derive(Default)]
struct GateResolutionInner {
    by_gate: HashMap<GateRef, AwaitedChildState>,
    gates_by_child: HashMap<TurnRunId, Vec<GateRef>>,
    deliverable_by_child: HashMap<TurnRunId, VecDeque<GateRef>>,
}

impl BoundedSubagentGateResolutionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_child_terminal(
        &self,
        child_run_id: TurnRunId,
        terminal_event: AwaitedChildTerminalEvent,
    ) -> Result<(), AgentLoopHostError> {
        let terminal_status = terminal_event.status;
        if !is_subagent_terminal_status(terminal_status) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Invalid,
                "subagent gate result must be terminal",
            ));
        }
        let mut inner = lock(&self.inner)?;
        let gate_refs = inner.gates_by_child.get(&child_run_id).cloned();
        let mut deliverable = Vec::new();
        let Some(gate_refs) = gate_refs else {
            return Ok(());
        };
        for gate_ref in gate_refs {
            if let Some(state) = inner.by_gate.get_mut(&gate_ref) {
                state.terminal_status = Some(terminal_status);
                state.terminal_event = Some(terminal_event.clone());
                if !state.delivery_claimed && !state.delivered_to_parent {
                    deliverable.push(gate_ref);
                }
            }
        }
        inner
            .deliverable_by_child
            .entry(child_run_id)
            .or_default()
            .extend(deliverable);
        Ok(())
    }

    pub fn claim_next_terminal_state_for_child(
        &self,
        child_run_id: TurnRunId,
    ) -> Result<Option<AwaitedChildState>, AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        while let Some(gate_ref) = inner
            .deliverable_by_child
            .get_mut(&child_run_id)
            .and_then(VecDeque::pop_front)
        {
            if let Some(state) = inner.by_gate.get_mut(&gate_ref)
                && state.terminal_status.is_some()
                && !state.delivery_claimed
                && !state.delivered_to_parent
            {
                state.delivery_claimed = true;
                return Ok(Some(state.clone()));
            }
        }
        inner.deliverable_by_child.remove(&child_run_id);
        Ok(None)
    }

    pub fn mark_delivered(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        if let Some(state) = inner.by_gate.get_mut(gate_ref) {
            state.delivery_claimed = false;
            state.delivered_to_parent = true;
        }
        Ok(())
    }

    pub fn release_terminal_claim(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        let child_run_id = if let Some(state) = inner.by_gate.get_mut(gate_ref)
            && !state.delivered_to_parent
        {
            state.delivery_claimed = false;
            state
                .terminal_status
                .is_some()
                .then_some(state.record.child_run_id)
        } else {
            None
        };
        if let Some(child_run_id) = child_run_id {
            inner
                .deliverable_by_child
                .entry(child_run_id)
                .or_default()
                .push_front(gate_ref.clone());
        }
        Ok(())
    }

    pub fn undelivered_terminal_states(
        &self,
    ) -> Result<Vec<AwaitedChildState>, AgentLoopHostError> {
        let inner = lock(&self.inner)?;
        Ok(inner
            .by_gate
            .values()
            .filter(|state| state.terminal_status.is_some() && !state.delivered_to_parent)
            .cloned()
            .collect())
    }

    pub fn claim_descendant_reservation_release(
        &self,
        gate_ref: &GateRef,
    ) -> Result<bool, AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        let Some(state) = inner.by_gate.get_mut(gate_ref) else {
            return Ok(false);
        };
        if state.descendant_reservation_released || state.descendant_reservation_release_claimed {
            return Ok(false);
        }
        state.descendant_reservation_release_claimed = true;
        Ok(true)
    }

    pub fn mark_descendant_reservation_released(
        &self,
        gate_ref: &GateRef,
    ) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        if let Some(state) = inner.by_gate.get_mut(gate_ref) {
            state.descendant_reservation_release_claimed = false;
            state.descendant_reservation_released = true;
        }
        Ok(())
    }

    pub fn release_descendant_reservation_claim(
        &self,
        gate_ref: &GateRef,
    ) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        if let Some(state) = inner.by_gate.get_mut(gate_ref)
            && !state.descendant_reservation_released
        {
            state.descendant_reservation_release_claimed = false;
        }
        Ok(())
    }

    pub fn state_for_gate(
        &self,
        gate_ref: &GateRef,
    ) -> Result<Option<AwaitedChildState>, AgentLoopHostError> {
        Ok(lock(&self.inner)?.by_gate.get(gate_ref).cloned())
    }

    pub fn subagent_kind_for_child(
        &self,
        child_run_id: TurnRunId,
    ) -> Result<Option<SubagentKindId>, AgentLoopHostError> {
        let inner = lock(&self.inner)?;
        let Some(gates) = inner.gates_by_child.get(&child_run_id) else {
            return Ok(None);
        };
        Ok(gates
            .iter()
            .find_map(|gate| inner.by_gate.get(gate))
            .map(|state| state.record.subagent_kind.clone()))
    }

    pub fn len(&self) -> Result<usize, AgentLoopHostError> {
        Ok(lock(&self.inner)?.by_gate.len())
    }

    pub fn is_empty(&self) -> Result<bool, AgentLoopHostError> {
        Ok(lock(&self.inner)?.by_gate.is_empty())
    }
}

#[async_trait]
impl SubagentGateResolutionStore for BoundedSubagentGateResolutionStore {
    async fn record_awaited_child(
        &self,
        record: AwaitedChildSetRecord,
    ) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        let gate_ref = record.gate_ref.clone();
        if inner.by_gate.contains_key(&gate_ref) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "subagent awaited-child gate already exists",
            ));
        }
        if inner.by_gate.len() >= MAX_GATE_RECORDS {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "subagent awaited-child gate store is at capacity",
            ));
        }
        inner
            .gates_by_child
            .entry(record.child_run_id)
            .or_default()
            .push(gate_ref.clone());
        inner.by_gate.insert(
            gate_ref.clone(),
            AwaitedChildState {
                record,
                terminal_status: None,
                terminal_event: None,
                descendant_reservation_release_claimed: false,
                descendant_reservation_released: false,
                delivery_claimed: false,
                delivered_to_parent: false,
            },
        );
        Ok(())
    }

    async fn delete_awaited_child(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        if let Some(old) = inner.by_gate.remove(gate_ref) {
            prune_child_index(&mut inner.gates_by_child, old.record.child_run_id, gate_ref);
            prune_deliverable_child_index(
                &mut inner.deliverable_by_child,
                old.record.child_run_id,
                gate_ref,
            );
        }
        Ok(())
    }
}

fn prune_child_index(
    gates_by_child: &mut HashMap<TurnRunId, Vec<GateRef>>,
    child_run_id: TurnRunId,
    gate_ref: &GateRef,
) {
    if let Some(gates) = gates_by_child.get_mut(&child_run_id) {
        gates.retain(|gate| gate != gate_ref);
        if gates.is_empty() {
            gates_by_child.remove(&child_run_id);
        }
    }
}

fn prune_deliverable_child_index(
    gates_by_child: &mut HashMap<TurnRunId, VecDeque<GateRef>>,
    child_run_id: TurnRunId,
    gate_ref: &GateRef,
) {
    if let Some(gates) = gates_by_child.get_mut(&child_run_id) {
        gates.retain(|gate| gate != gate_ref);
        if gates.is_empty() {
            gates_by_child.remove(&child_run_id);
        }
    }
}

fn is_subagent_terminal_status(status: TurnStatus) -> bool {
    status.is_terminal() || status == TurnStatus::RecoveryRequired
}

fn lock<T>(
    mutex: &std::sync::Mutex<T>,
) -> Result<std::sync::MutexGuard<'_, T>, AgentLoopHostError> {
    mutex.lock().map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "subagent gate store mutex poisoned",
        )
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, CapabilityId, TenantId, ThreadId};
    use ironclaw_loop_support::SpawnSubagentMode;
    use ironclaw_turns::{LoopResultRef, ReplyTargetBindingRef, SourceBindingRef, TurnScope};

    use super::*;

    fn record(gate_ref: &str, child_run_id: TurnRunId) -> AwaitedChildSetRecord {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        let parent_scope = TurnScope::new(
            tenant.clone(),
            Some(agent.clone()),
            None,
            ThreadId::new("parent-thread").unwrap(),
        );
        let child_scope = TurnScope::new(
            tenant,
            Some(agent),
            None,
            ThreadId::new("child-thread").unwrap(),
        );
        let parent_run_id = TurnRunId::new();
        let mut parent_run_context =
            ironclaw_agent_loop::test_support::test_run_context("subagent-gate");
        parent_run_context.scope = parent_scope;
        parent_run_context.thread_id = ThreadId::new("parent-thread").unwrap();
        parent_run_context.run_id = parent_run_id;
        AwaitedChildSetRecord {
            gate_ref: GateRef::new(gate_ref).unwrap(),
            parent_run_context,
            tree_root_run_id: TurnRunId::new(),
            child_scope,
            child_run_id,
            child_thread_id: ThreadId::new("child-thread").unwrap(),
            source_binding_ref: SourceBindingRef::new("subagent-source:test").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:test").unwrap(),
            subagent_kind: SubagentKindId::new("general").unwrap(),
            spawn_capability_id: CapabilityId::new(
                ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
            )
            .unwrap(),
            result_ref: LoopResultRef::new("result:subagent.test").unwrap(),
            mode: SpawnSubagentMode::Blocking,
        }
    }

    #[tokio::test]
    async fn records_terminal_child_once_until_marked_delivered() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        let gate = GateRef::new("gate:subagent:test").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_run_id))
            .await
            .unwrap();

        store
            .record_child_terminal(child_run_id, terminal_event(TurnStatus::Completed))
            .unwrap();
        let ready = store
            .claim_next_terminal_state_for_child(child_run_id)
            .unwrap();
        assert!(ready.is_some());
        store.mark_delivered(&gate).unwrap();
        store
            .record_child_terminal(child_run_id, terminal_event(TurnStatus::Completed))
            .unwrap();
        let ready = store
            .claim_next_terminal_state_for_child(child_run_id)
            .unwrap();
        assert!(ready.is_none());
    }

    #[tokio::test]
    async fn record_child_terminal_rejects_non_terminal_statuses() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        let gate = GateRef::new("gate:subagent:test").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_run_id))
            .await
            .unwrap();

        let error = store
            .record_child_terminal(child_run_id, terminal_event(TurnStatus::Running))
            .unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::Invalid);
        assert!(error.safe_summary.contains("must be terminal"));
    }

    #[tokio::test]
    async fn terminal_child_claims_one_state_at_a_time() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        store
            .record_awaited_child(record("gate:subagent:first", child_run_id))
            .await
            .unwrap();
        store
            .record_awaited_child(record("gate:subagent:second", child_run_id))
            .await
            .unwrap();
        store
            .record_child_terminal(child_run_id, terminal_event(TurnStatus::Completed))
            .unwrap();

        let first = store
            .claim_next_terminal_state_for_child(child_run_id)
            .unwrap()
            .expect("first gate should be claimed");
        let second = store
            .claim_next_terminal_state_for_child(child_run_id)
            .unwrap()
            .expect("second gate should be claimed independently");
        let third = store
            .claim_next_terminal_state_for_child(child_run_id)
            .unwrap();

        assert_ne!(first.record.gate_ref, second.record.gate_ref);
        assert!(third.is_none());
    }

    #[tokio::test]
    async fn terminal_claim_release_allows_retry() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        let gate = GateRef::new("gate:subagent:test").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_run_id))
            .await
            .unwrap();
        store
            .record_child_terminal(child_run_id, terminal_event(TurnStatus::Completed))
            .unwrap();

        let first = store
            .claim_next_terminal_state_for_child(child_run_id)
            .unwrap();
        store.release_terminal_claim(&gate).unwrap();
        let retried = store
            .claim_next_terminal_state_for_child(child_run_id)
            .unwrap();

        assert!(first.is_some());
        assert!(retried.is_some());
    }

    #[tokio::test]
    async fn marks_descendant_release_once() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        let gate = GateRef::new("gate:subagent:test").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_run_id))
            .await
            .unwrap();

        assert!(store.claim_descendant_reservation_release(&gate).unwrap());
        assert!(!store.claim_descendant_reservation_release(&gate).unwrap());
        store.mark_descendant_reservation_released(&gate).unwrap();
        assert!(!store.claim_descendant_reservation_release(&gate).unwrap());
    }

    #[tokio::test]
    async fn descendant_release_claim_can_be_retried_before_marked_released() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        let gate = GateRef::new("gate:subagent:test").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_run_id))
            .await
            .unwrap();

        assert!(store.claim_descendant_reservation_release(&gate).unwrap());
        store.release_descendant_reservation_claim(&gate).unwrap();
        assert!(store.claim_descendant_reservation_release(&gate).unwrap());
    }

    #[tokio::test]
    async fn delete_removes_child_index() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        let gate = GateRef::new("gate:subagent:test").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_run_id))
            .await
            .unwrap();
        store.delete_awaited_child(&gate).await.unwrap();

        assert!(
            store
                .record_child_terminal(child_run_id, terminal_event(TurnStatus::Completed))
                .is_ok()
        );
        assert!(
            store
                .claim_next_terminal_state_for_child(child_run_id)
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn capacity_fails_closed_without_evicting_live_gates() {
        let store = BoundedSubagentGateResolutionStore::new();
        for index in 0..MAX_GATE_RECORDS {
            store
                .record_awaited_child(record(&format!("gate:subagent:{index}"), TurnRunId::new()))
                .await
                .unwrap();
        }

        let error = store
            .record_awaited_child(record("gate:subagent:overflow", TurnRunId::new()))
            .await
            .unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::BudgetExceeded);
        assert_eq!(store.len().unwrap(), MAX_GATE_RECORDS);
        assert!(
            store
                .state_for_gate(&GateRef::new("gate:subagent:0").unwrap())
                .unwrap()
                .is_some()
        );
    }

    fn terminal_event(status: TurnStatus) -> AwaitedChildTerminalEvent {
        AwaitedChildTerminalEvent {
            status,
            kind: TurnEventKind::Completed,
            cursor: EventCursor(1),
            sanitized_reason: None,
            owner_user_id: Some(ironclaw_host_api::UserId::new("owner").unwrap()),
        }
    }
}
