use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use ironclaw_host_api::UserId;
use ironclaw_loop_support::{AwaitedChildSetRecord, SubagentGateResolutionStore, SubagentKindId};
use ironclaw_turns::{
    EventCursor, GateRef, TurnEventKind, TurnRunId, TurnStatus,
    run_profile::{AgentLoopHostError, AgentLoopHostErrorKind},
};
use parking_lot::Mutex;

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
    inner: Mutex<GateResolutionInner>,
}

#[derive(Default)]
struct GateResolutionInner {
    by_gate: HashMap<GateRef, Vec<AwaitedChildState>>,
    gates_by_child: HashMap<TurnRunId, Vec<GateRef>>,
    deliverable_by_child: HashMap<TurnRunId, VecDeque<GateRef>>,
    // Cached total state count across all gate keys. Maintained alongside
    // every push to `by_gate` and prune via `delete_awaited_child`, so the
    // capacity check stays O(1) instead of summing every Vec under the lock.
    total_states: usize,
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
            if let Some(states) = inner.by_gate.get_mut(&gate_ref) {
                let mut any_deliverable = false;
                for state in states
                    .iter_mut()
                    .filter(|state| state.record.child_run_id == child_run_id)
                {
                    // Skip if this child's terminal was already recorded —
                    // event replay or recovery may redeliver the same
                    // terminal; treat the first one as authoritative so the
                    // event payload (cursor, status, sanitized_reason) is
                    // never silently overwritten by a later duplicate.
                    if state.terminal_status.is_some() {
                        continue;
                    }
                    state.terminal_status = Some(terminal_status);
                    state.terminal_event = Some(terminal_event.clone());
                    if !state.delivery_claimed && !state.delivered_to_parent {
                        any_deliverable = true;
                    }
                }
                if any_deliverable {
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
            let claim_result = inner
                .by_gate
                .get_mut(&gate_ref)
                .and_then(|states| try_claim_one_state(states));
            if let Some((claimed, more_unclaimed)) = claim_result {
                if more_unclaimed {
                    inner
                        .deliverable_by_child
                        .entry(child_run_id)
                        .or_default()
                        .push_front(gate_ref);
                }
                return Ok(Some(claimed));
            }
            // Gate exists but not every sibling under it is terminal yet.
            // Re-queue the gate so a future `record_child_terminal` for this
            // child doesn't see an empty queue — without this requeue the
            // gate would orphan from this child's view and rely entirely on
            // sibling-driven requeues, which is fragile under recovery or
            // event replay.
            if inner
                .by_gate
                .get(&gate_ref)
                .is_some_and(|states| !states.iter().all(|s| s.terminal_status.is_some()))
            {
                inner
                    .deliverable_by_child
                    .entry(child_run_id)
                    .or_default()
                    .push_front(gate_ref);
                return Ok(None);
            }
        }
        inner.deliverable_by_child.remove(&child_run_id);
        Ok(None)
    }

    // Drains every deliverable state for `child_run_id` under a single lock
    // acquisition. Each shared batch gate is requeued internally while there
    // are remaining unclaimed states, so callers see one Vec containing every
    // state this child unblocks. Equivalent to looping
    // `claim_next_terminal_state_for_child` until None, but takes the lock
    // exactly once.
    pub fn claim_all_terminal_states_for_child(
        &self,
        child_run_id: TurnRunId,
    ) -> Result<Vec<AwaitedChildState>, AgentLoopHostError> {
        let mut claimed = Vec::new();
        let mut inner = lock(&self.inner)?;
        while let Some(gate_ref) = inner
            .deliverable_by_child
            .get_mut(&child_run_id)
            .and_then(VecDeque::pop_front)
        {
            let claim_result = inner
                .by_gate
                .get_mut(&gate_ref)
                .and_then(|states| try_claim_one_state(states));
            match claim_result {
                Some((state, more_unclaimed)) => {
                    if more_unclaimed {
                        inner
                            .deliverable_by_child
                            .entry(child_run_id)
                            .or_default()
                            .push_front(gate_ref);
                    }
                    claimed.push(state);
                }
                None => {
                    // Either gate missing or not all siblings terminal. If
                    // the gate still exists with pending siblings, requeue so
                    // the next `record_child_terminal` for this child doesn't
                    // see an empty queue. (Same defensive requeue as
                    // `claim_next_terminal_state_for_child`.)
                    if inner
                        .by_gate
                        .get(&gate_ref)
                        .is_some_and(|states| !states.iter().all(|s| s.terminal_status.is_some()))
                    {
                        inner
                            .deliverable_by_child
                            .entry(child_run_id)
                            .or_default()
                            .push_front(gate_ref);
                        break;
                    }
                }
            }
        }
        if claimed.is_empty() {
            inner.deliverable_by_child.remove(&child_run_id);
        }
        Ok(claimed)
    }

    pub fn mark_delivered(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        if let Some(states) = inner.by_gate.get_mut(gate_ref) {
            for state in states {
                state.delivery_claimed = false;
                state.delivered_to_parent = true;
            }
        }
        Ok(())
    }

    pub fn release_terminal_claim(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        let to_requeue: Vec<TurnRunId> = if let Some(states) = inner.by_gate.get_mut(gate_ref) {
            let mut cids = Vec::new();
            for state in states.iter_mut().filter(|state| !state.delivered_to_parent) {
                if state.delivery_claimed {
                    state.delivery_claimed = false;
                    if state.terminal_status.is_some() {
                        cids.push(state.record.child_run_id);
                    }
                }
            }
            cids
        } else {
            Vec::new()
        };
        for child_run_id in to_requeue {
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
            .filter(|states| states.iter().all(|state| state.terminal_status.is_some()))
            .flat_map(|states| states.iter())
            .filter(|state| !state.delivered_to_parent)
            .cloned()
            .collect())
    }

    pub fn claim_descendant_reservation_release(
        &self,
        gate_ref: &GateRef,
    ) -> Result<bool, AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        let Some(states) = inner.by_gate.get_mut(gate_ref) else {
            return Ok(false);
        };
        let Some(state) = states.iter_mut().find(|state| {
            !state.descendant_reservation_released && !state.descendant_reservation_release_claimed
        }) else {
            return Ok(false);
        };
        state.descendant_reservation_release_claimed = true;
        Ok(true)
    }

    pub fn mark_descendant_reservation_released(
        &self,
        gate_ref: &GateRef,
    ) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        if let Some(states) = inner.by_gate.get_mut(gate_ref)
            && let Some(state) = states
                .iter_mut()
                .find(|s| s.descendant_reservation_release_claimed)
        {
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
        if let Some(states) = inner.by_gate.get_mut(gate_ref)
            && let Some(state) = states
                .iter_mut()
                .find(|s| s.descendant_reservation_release_claimed)
        {
            state.descendant_reservation_release_claimed = false;
        }
        Ok(())
    }

    // Probe for gate presence and gate-level metadata (subagent_kind, mode,
    // parent context) that is uniform across every state under a shared
    // batch gate. Callers that need per-child state must iterate the full
    // Vec via the dedicated APIs.
    pub fn state_for_gate(
        &self,
        gate_ref: &GateRef,
    ) -> Result<Option<AwaitedChildState>, AgentLoopHostError> {
        Ok(lock(&self.inner)?
            .by_gate
            .get(gate_ref)
            .and_then(|states| states.first().cloned()))
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
            .filter_map(|gate| inner.by_gate.get(gate))
            .find_map(|states| states.first())
            .map(|state| state.record.subagent_kind.clone()))
    }

    pub fn len(&self) -> Result<usize, AgentLoopHostError> {
        Ok(lock(&self.inner)?
            .by_gate
            .values()
            .map(|states| states.len())
            .sum())
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
        if inner.total_states >= MAX_GATE_RECORDS {
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
        inner
            .by_gate
            .entry(gate_ref.clone())
            .or_default()
            .push(AwaitedChildState {
                record,
                terminal_status: None,
                terminal_event: None,
                descendant_reservation_release_claimed: false,
                descendant_reservation_released: false,
                delivery_claimed: false,
                delivered_to_parent: false,
            });
        inner.total_states += 1;
        Ok(())
    }

    async fn delete_awaited_child(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError> {
        let mut inner = lock(&self.inner)?;
        if let Some(old) = inner.by_gate.remove(gate_ref) {
            inner.total_states = inner.total_states.saturating_sub(old.len());
            for state in old {
                prune_child_index(
                    &mut inner.gates_by_child,
                    state.record.child_run_id,
                    gate_ref,
                );
                prune_deliverable_child_index(
                    &mut inner.deliverable_by_child,
                    state.record.child_run_id,
                    gate_ref,
                );
            }
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

// Infallible parking_lot lock helper retained for call-site compatibility
// (every public method threads a `?` through this) and to make a future
// switch back to a fallible store easy. `parking_lot::Mutex` cannot poison,
// so this never returns `Err`.
fn lock<T>(mutex: &Mutex<T>) -> Result<parking_lot::MutexGuard<'_, T>, AgentLoopHostError> {
    Ok(mutex.lock())
}

/// Tries to claim one undelivered terminal state from the given gate's state
/// vector. Returns `Some((state, more_unclaimed))` if a state was claimed,
/// where `more_unclaimed` indicates whether further unclaimed terminal states
/// remain in the same gate vector (used by callers to decide whether to
/// requeue the gate for another drain pass).
fn try_claim_one_state(states: &mut [AwaitedChildState]) -> Option<(AwaitedChildState, bool)> {
    if !states.iter().all(|state| state.terminal_status.is_some()) {
        return None;
    }
    let state = states
        .iter_mut()
        .find(|state| !state.delivery_claimed && !state.delivered_to_parent)?;
    state.delivery_claimed = true;
    let claimed = state.clone();
    let more_unclaimed = states
        .iter()
        .any(|s| !s.delivery_claimed && !s.delivered_to_parent);
    Some((claimed, more_unclaimed))
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
        let gate = GateRef::new("gate:subagent-test").unwrap();
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
            .record_awaited_child(record("gate:subagent-first", child_run_id))
            .await
            .unwrap();
        store
            .record_awaited_child(record("gate:subagent-second", child_run_id))
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
        let gate = GateRef::new("gate:subagent-test").unwrap();
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
    async fn terminal_claim_release_resets_only_undelivered_states_for_shared_gate() {
        let store = BoundedSubagentGateResolutionStore::new();
        let delivered_child = TurnRunId::new();
        let retry_child = TurnRunId::new();
        let gate = GateRef::new("gate:subagent-batch-test").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), delivered_child))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate.as_str(), retry_child))
            .await
            .unwrap();
        store
            .record_child_terminal(delivered_child, terminal_event(TurnStatus::Completed))
            .unwrap();
        store
            .record_child_terminal(retry_child, terminal_event(TurnStatus::Completed))
            .unwrap();

        assert!(
            store
                .claim_next_terminal_state_for_child(delivered_child)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .claim_next_terminal_state_for_child(retry_child)
                .unwrap()
                .is_some()
        );
        {
            let mut inner = store.inner.lock();
            let states = inner.by_gate.get_mut(&gate).expect("gate states");
            let delivered = states
                .iter_mut()
                .find(|state| state.record.child_run_id == delivered_child)
                .expect("delivered child state");
            delivered.delivered_to_parent = true;
        }

        store.release_terminal_claim(&gate).unwrap();

        let retried = store
            .claim_next_terminal_state_for_child(retry_child)
            .unwrap()
            .expect("undelivered state should be claimable after release");
        assert_eq!(retried.record.child_run_id, retry_child);
        assert!(
            store
                .claim_next_terminal_state_for_child(delivered_child)
                .unwrap()
                .is_none(),
            "delivered state should not become claimable again"
        );
    }

    #[tokio::test]
    async fn marks_descendant_release_once() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_run_id = TurnRunId::new();
        let gate = GateRef::new("gate:subagent-test").unwrap();
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
        let gate = GateRef::new("gate:subagent-test").unwrap();
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
                .record_awaited_child(record(&format!("gate:subagent-{index}"), TurnRunId::new()))
                .await
                .unwrap();
        }

        let error = store
            .record_awaited_child(record("gate:subagent-overflow", TurnRunId::new()))
            .await
            .unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::BudgetExceeded);
        assert_eq!(store.len().unwrap(), MAX_GATE_RECORDS);
        assert!(
            store
                .state_for_gate(&GateRef::new("gate:subagent-0").unwrap())
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

    #[tokio::test]
    async fn claim_returns_none_when_sibling_on_shared_gate_is_still_pending() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let gate = "gate:subagent-batch.pending";
        store
            .record_awaited_child(record(gate, child_a))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate, child_b))
            .await
            .unwrap();

        store
            .record_child_terminal(child_a, terminal_event(TurnStatus::Completed))
            .unwrap();

        let claim_a = store.claim_next_terminal_state_for_child(child_a).unwrap();
        assert!(
            claim_a.is_none(),
            "shared batch gate must not yield while sibling pending"
        );
    }

    #[tokio::test]
    async fn undelivered_terminal_states_excludes_partially_complete_shared_gates() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let gate = "gate:subagent-batch.partial";
        store
            .record_awaited_child(record(gate, child_a))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate, child_b))
            .await
            .unwrap();
        store
            .record_child_terminal(child_a, terminal_event(TurnStatus::Completed))
            .unwrap();

        let undelivered = store.undelivered_terminal_states().unwrap();
        assert!(
            undelivered.is_empty(),
            "partial shared gate must not surface as undelivered"
        );

        store
            .record_child_terminal(child_b, terminal_event(TurnStatus::Completed))
            .unwrap();
        let undelivered = store.undelivered_terminal_states().unwrap();
        assert_eq!(undelivered.len(), 2);
    }

    #[tokio::test]
    async fn state_for_gate_returns_first_registered_state_for_shared_gate() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let gate = "gate:subagent-batch.order";
        store
            .record_awaited_child(record(gate, child_a))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate, child_b))
            .await
            .unwrap();

        let state = store
            .state_for_gate(&GateRef::new(gate).unwrap())
            .unwrap()
            .expect("gate present");
        assert_eq!(state.record.child_run_id, child_a);
    }

    #[tokio::test]
    async fn descendant_reservation_release_claims_one_state_per_call() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let gate = GateRef::new("gate:subagent-batch.descendant").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_a))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_b))
            .await
            .unwrap();

        assert!(store.claim_descendant_reservation_release(&gate).unwrap());
        store.mark_descendant_reservation_released(&gate).unwrap();
        assert!(
            store.claim_descendant_reservation_release(&gate).unwrap(),
            "second sibling must still be claimable"
        );
        store.mark_descendant_reservation_released(&gate).unwrap();
        assert!(
            !store.claim_descendant_reservation_release(&gate).unwrap(),
            "no further claims once both siblings released"
        );
    }

    #[tokio::test]
    async fn release_terminal_claim_requeues_all_claimed_states_under_shared_gate() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let gate = GateRef::new("gate:subagent-batch.requeue").unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_a))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate.as_str(), child_b))
            .await
            .unwrap();
        store
            .record_child_terminal(child_a, terminal_event(TurnStatus::Completed))
            .unwrap();
        store
            .record_child_terminal(child_b, terminal_event(TurnStatus::Completed))
            .unwrap();

        let claimed = store.claim_all_terminal_states_for_child(child_b).unwrap();
        assert_eq!(claimed.len(), 2);

        // Simulate handle_claimed_terminal_states failure: release every
        // claimed state. Calling release_terminal_claim multiple times for
        // the same shared gate must be idempotent — it resets every claimed
        // state once and re-enqueues the gate for a subsequent attempt.
        for state in &claimed {
            store
                .release_terminal_claim(&state.record.gate_ref)
                .unwrap();
        }
        let reclaimed = store.claim_all_terminal_states_for_child(child_b).unwrap();
        assert_eq!(
            reclaimed.len(),
            2,
            "all states must be reclaimable after release"
        );
    }

    #[tokio::test]
    async fn claim_all_terminal_states_drains_shared_gate_in_single_call() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let child_c = TurnRunId::new();
        let gate = "gate:subagent-batch.drain";
        store
            .record_awaited_child(record(gate, child_a))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate, child_b))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate, child_c))
            .await
            .unwrap();
        store
            .record_child_terminal(child_a, terminal_event(TurnStatus::Completed))
            .unwrap();
        store
            .record_child_terminal(child_b, terminal_event(TurnStatus::Completed))
            .unwrap();
        store
            .record_child_terminal(child_c, terminal_event(TurnStatus::Completed))
            .unwrap();

        let claimed = store.claim_all_terminal_states_for_child(child_c).unwrap();
        assert_eq!(
            claimed.len(),
            3,
            "single drain call must yield every state under shared batch gate"
        );
    }

    #[tokio::test]
    async fn claim_requeues_gate_when_sibling_pending() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child_a = TurnRunId::new();
        let child_b = TurnRunId::new();
        let gate = "gate:subagent-batch.requeue-on-pending";
        store
            .record_awaited_child(record(gate, child_a))
            .await
            .unwrap();
        store
            .record_awaited_child(record(gate, child_b))
            .await
            .unwrap();
        store
            .record_child_terminal(child_a, terminal_event(TurnStatus::Completed))
            .unwrap();

        // child A's claim sees B still pending; must return None AND requeue
        // the gate so a future sibling-driven recorder still finds it.
        let first = store.claim_next_terminal_state_for_child(child_a).unwrap();
        assert!(first.is_none());

        // Sanity check: gate ref still queued for child A.
        {
            let inner = store.inner.lock();
            assert!(
                inner
                    .deliverable_by_child
                    .get(&child_a)
                    .map(|q| q.iter().any(|g| g.as_str() == gate))
                    .unwrap_or(false),
                "gate must be requeued for child A when sibling is pending"
            );
        }

        // Once B terminates, both states drain via either child's claim.
        store
            .record_child_terminal(child_b, terminal_event(TurnStatus::Completed))
            .unwrap();
        let drained = store.claim_all_terminal_states_for_child(child_b).unwrap();
        assert_eq!(drained.len(), 2);
    }

    #[tokio::test]
    async fn record_child_terminal_is_idempotent_on_replay() {
        let store = BoundedSubagentGateResolutionStore::new();
        let child = TurnRunId::new();
        store
            .record_awaited_child(record("gate:subagent.idempotent", child))
            .await
            .unwrap();

        let first_event = AwaitedChildTerminalEvent {
            status: TurnStatus::Completed,
            kind: TurnEventKind::Completed,
            cursor: EventCursor(1),
            sanitized_reason: Some("first".to_string()),
            owner_user_id: Some(ironclaw_host_api::UserId::new("owner").unwrap()),
        };
        let replay_event = AwaitedChildTerminalEvent {
            status: TurnStatus::Failed,
            kind: TurnEventKind::Failed,
            cursor: EventCursor(2),
            sanitized_reason: Some("replay overwrites first".to_string()),
            owner_user_id: Some(ironclaw_host_api::UserId::new("owner").unwrap()),
        };
        store
            .record_child_terminal(child, first_event.clone())
            .unwrap();
        store
            .record_child_terminal(child, replay_event.clone())
            .unwrap();

        let claimed = store
            .claim_next_terminal_state_for_child(child)
            .unwrap()
            .expect("delivery available");
        let recorded = claimed.terminal_event.expect("terminal event present");
        assert_eq!(
            recorded.sanitized_reason.as_deref(),
            Some("first"),
            "first terminal is authoritative; replay must not overwrite"
        );
        assert_eq!(recorded.cursor, EventCursor(1));
    }

    #[tokio::test]
    async fn record_awaited_child_capacity_counts_total_states_not_keys() {
        let store = BoundedSubagentGateResolutionStore::new();
        let shared_gate = "gate:subagent-batch.capacity";
        // Filling the bound through a single shared gate must hit the cap.
        for _ in 0..MAX_GATE_RECORDS {
            store
                .record_awaited_child(record(shared_gate, TurnRunId::new()))
                .await
                .unwrap();
        }
        let error = store
            .record_awaited_child(record(shared_gate, TurnRunId::new()))
            .await
            .unwrap_err();
        assert_eq!(error.kind, AgentLoopHostErrorKind::BudgetExceeded);
    }
}
