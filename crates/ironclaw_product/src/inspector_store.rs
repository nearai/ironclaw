//! Bounded, process-local storage for operator inspection diagnostics.
//!
//! The store deliberately has no persistence backend. It keeps raw diagnostic
//! content out of durable events and drops all state at process restart.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use ironclaw_host_api::{
    ids::{TenantId, ThreadId, UserId},
    turn::TurnRunId,
};
use ironclaw_product_contracts::inspector::{
    DEFAULT_MAX_ACTIVITY_ENTRIES, DEFAULT_MAX_MODEL_CALLS_PER_RUN,
    DEFAULT_MAX_RETAINED_RUNS_PER_SESSION, DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
    DEFAULT_MAX_TOOL_EXECUTIONS_PER_RUN, DEFAULT_MAX_TRACKED_SESSIONS, DiagnosticActivityEntry,
    DiagnosticActivityEvent, DiagnosticCursor, DiagnosticScope, DiagnosticSequence,
    DiagnosticSnapshot, DiagnosticStreamId, DiagnosticUpdateBatch, DiagnosticUpdateEnvelope,
    DiagnosticUpdateKind, ModelCallDiagnostic, PromptDiagnostic, SessionDiagnosticStats,
    ToolExecutionDiagnostic,
};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticStoreLimits {
    pub max_sessions: usize,
    pub max_runs_per_session: usize,
    pub max_model_calls_per_run: usize,
    pub max_tool_executions_per_run: usize,
    pub max_activity_entries_per_run: usize,
    pub max_updates_per_run: usize,
    pub live_update_capacity: usize,
}

impl Default for DiagnosticStoreLimits {
    fn default() -> Self {
        Self {
            max_sessions: DEFAULT_MAX_TRACKED_SESSIONS,
            max_runs_per_session: DEFAULT_MAX_RETAINED_RUNS_PER_SESSION,
            max_model_calls_per_run: DEFAULT_MAX_MODEL_CALLS_PER_RUN,
            max_tool_executions_per_run: DEFAULT_MAX_TOOL_EXECUTIONS_PER_RUN,
            max_activity_entries_per_run: DEFAULT_MAX_ACTIVITY_ENTRIES,
            max_updates_per_run: DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
            live_update_capacity: DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
        }
    }
}

impl DiagnosticStoreLimits {
    fn validate(self) -> Result<Self, DiagnosticStoreError> {
        let values = [
            (
                "max_sessions",
                self.max_sessions,
                DEFAULT_MAX_TRACKED_SESSIONS,
            ),
            (
                "max_runs_per_session",
                self.max_runs_per_session,
                DEFAULT_MAX_RETAINED_RUNS_PER_SESSION,
            ),
            (
                "max_model_calls_per_run",
                self.max_model_calls_per_run,
                DEFAULT_MAX_MODEL_CALLS_PER_RUN,
            ),
            (
                "max_tool_executions_per_run",
                self.max_tool_executions_per_run,
                DEFAULT_MAX_TOOL_EXECUTIONS_PER_RUN,
            ),
            (
                "max_activity_entries_per_run",
                self.max_activity_entries_per_run,
                DEFAULT_MAX_ACTIVITY_ENTRIES,
            ),
            (
                "max_updates_per_run",
                self.max_updates_per_run,
                DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
            ),
            (
                "live_update_capacity",
                self.live_update_capacity,
                DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
            ),
        ];
        if let Some((name, _, _)) = values.iter().copied().find(|(_, value, _)| *value == 0) {
            return Err(DiagnosticStoreError::InvalidLimit(name));
        }
        if let Some((name, _, maximum)) = values
            .iter()
            .copied()
            .find(|(_, value, maximum)| value > maximum)
        {
            return Err(DiagnosticStoreError::LimitExceedsMaximum { name, maximum });
        }
        Ok(self)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DiagnosticStoreError {
    #[error("diagnostic store limit `{0}` must be non-zero")]
    InvalidLimit(&'static str),
    #[error("diagnostic store limit `{name}` exceeds its maximum of {maximum}")]
    LimitExceedsMaximum { name: &'static str, maximum: usize },
    #[error("diagnostic store state is unavailable")]
    StateUnavailable,
    #[error("diagnostic sequence space is exhausted")]
    SequenceExhausted,
    #[error("diagnostic store invariant failed")]
    Invariant,
    #[error("diagnostic subscriber lagged by {0} updates")]
    SubscriberLagged(u64),
    #[error("diagnostic subscription is closed")]
    SubscriptionClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DiagnosticSessionKey {
    tenant_id: TenantId,
    user_id: UserId,
    thread_id: ThreadId,
}

impl From<&DiagnosticScope> for DiagnosticSessionKey {
    fn from(scope: &DiagnosticScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            thread_id: scope.thread_id.clone(),
        }
    }
}

#[derive(Debug)]
struct DiagnosticRunState {
    stream_id: DiagnosticStreamId,
    prompt: Option<PromptDiagnostic>,
    model_calls: VecDeque<ModelCallDiagnostic>,
    tool_executions: VecDeque<ToolExecutionDiagnostic>,
    activity: VecDeque<DiagnosticActivityEntry>,
    updates: VecDeque<DiagnosticUpdateEnvelope>,
    latest_sequence: DiagnosticSequence,
}

impl Default for DiagnosticRunState {
    fn default() -> Self {
        Self {
            stream_id: DiagnosticStreamId::new(),
            prompt: None,
            model_calls: VecDeque::new(),
            tool_executions: VecDeque::new(),
            activity: VecDeque::new(),
            updates: VecDeque::new(),
            latest_sequence: DiagnosticSequence::ZERO,
        }
    }
}

#[derive(Debug, Default)]
struct DiagnosticSessionState {
    runs: HashMap<TurnRunId, DiagnosticRunState>,
    run_order: VecDeque<TurnRunId>,
    stats: SessionDiagnosticStats,
}

#[derive(Debug, Default)]
struct DiagnosticStoreState {
    sessions: HashMap<DiagnosticSessionKey, DiagnosticSessionState>,
    session_order: VecDeque<DiagnosticSessionKey>,
}

impl DiagnosticStoreState {
    fn run_mut(
        &mut self,
        scope: &DiagnosticScope,
        limits: DiagnosticStoreLimits,
    ) -> Result<&mut DiagnosticRunState, DiagnosticStoreError> {
        let session_key = DiagnosticSessionKey::from(scope);
        if !self.sessions.contains_key(&session_key) {
            while self.sessions.len() >= limits.max_sessions {
                let Some(evicted) = self.session_order.pop_front() else {
                    return Err(DiagnosticStoreError::Invariant);
                };
                self.sessions.remove(&evicted);
            }
            self.sessions
                .insert(session_key.clone(), DiagnosticSessionState::default());
        }
        touch(&mut self.session_order, session_key.clone());

        let session = self
            .sessions
            .get_mut(&session_key)
            .ok_or(DiagnosticStoreError::Invariant)?;
        if !session.runs.contains_key(&scope.run_id) {
            while session.runs.len() >= limits.max_runs_per_session {
                let Some(evicted) = session.run_order.pop_front() else {
                    return Err(DiagnosticStoreError::Invariant);
                };
                session.runs.remove(&evicted);
            }
            session
                .runs
                .insert(scope.run_id, DiagnosticRunState::default());
        }
        touch(&mut session.run_order, scope.run_id);
        session
            .runs
            .get_mut(&scope.run_id)
            .ok_or(DiagnosticStoreError::Invariant)
    }

    fn run(&self, scope: &DiagnosticScope) -> Option<&DiagnosticRunState> {
        self.sessions
            .get(&DiagnosticSessionKey::from(scope))?
            .runs
            .get(&scope.run_id)
    }

    fn session(&self, scope: &DiagnosticScope) -> Option<&DiagnosticSessionState> {
        self.sessions.get(&DiagnosticSessionKey::from(scope))
    }
}

fn touch<T: PartialEq>(order: &mut VecDeque<T>, value: T) {
    if let Some(index) = order.iter().position(|entry| entry == &value) {
        order.remove(index);
    }
    order.push_back(value);
}

#[derive(Debug)]
pub struct InMemoryDiagnosticStore {
    limits: DiagnosticStoreLimits,
    state: Mutex<DiagnosticStoreState>,
    updates: broadcast::Sender<Arc<DiagnosticUpdateEnvelope>>,
}

impl InMemoryDiagnosticStore {
    pub fn new(limits: DiagnosticStoreLimits) -> Result<Self, DiagnosticStoreError> {
        let limits = limits.validate()?;
        let (updates, _) = broadcast::channel(limits.live_update_capacity);
        Ok(Self {
            limits,
            state: Mutex::new(DiagnosticStoreState::default()),
            updates,
        })
    }

    pub fn record_prompt(
        &self,
        scope: DiagnosticScope,
        prompt: PromptDiagnostic,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let update = DiagnosticUpdateKind::PromptUpdated {
            component_count: prompt.components.len(),
            total_estimated_tokens: prompt.total_estimated_tokens,
            truncated: prompt.any_content_truncated(),
        };
        self.record(scope, update, |run, _| {
            run.prompt = Some(prompt);
        })
    }

    pub fn record_model_call(
        &self,
        scope: DiagnosticScope,
        model_call: ModelCallDiagnostic,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let update = DiagnosticUpdateKind::ModelCall(model_call.clone());
        let cap = self.limits.max_model_calls_per_run;
        self.record(scope, update, move |run, _| {
            replace_or_push(
                &mut run.model_calls,
                model_call,
                cap,
                |existing, incoming| existing.call_id == incoming.call_id,
            );
        })
    }

    pub fn record_tool_execution(
        &self,
        scope: DiagnosticScope,
        tool: ToolExecutionDiagnostic,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let update = DiagnosticUpdateKind::ToolExecutionUpdated {
            activity_id: tool.activity_id,
            model_call_id: tool.model_call_id,
            capability_name: tool.capability_name.clone(),
            status: tool.status,
            duration_ms: tool.duration_ms,
            output_bytes: tool.output_bytes,
            result_truncated: tool.result_truncated(),
        };
        let cap = self.limits.max_tool_executions_per_run;
        self.record(scope, update, move |run, _| {
            replace_or_push(&mut run.tool_executions, tool, cap, |existing, incoming| {
                existing.activity_id == incoming.activity_id
            });
        })
    }

    pub fn record_activity(
        &self,
        scope: DiagnosticScope,
        event: DiagnosticActivityEvent,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let update = DiagnosticUpdateKind::Activity(event.clone());
        let cap = self.limits.max_activity_entries_per_run;
        self.record(scope, update, move |run, sequence| {
            push_bounded(
                &mut run.activity,
                DiagnosticActivityEntry { sequence, event },
                cap,
            );
        })
    }

    pub fn record_stats(
        &self,
        scope: DiagnosticScope,
        stats: SessionDiagnosticStats,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let stats = stats.into_bounded();
        let session_key = DiagnosticSessionKey::from(&scope);
        let mut state = self
            .state
            .lock()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let run = state.run_mut(&scope, self.limits)?;
        let next = run
            .latest_sequence
            .as_u64()
            .checked_add(1)
            .ok_or(DiagnosticStoreError::SequenceExhausted)?;
        let sequence = DiagnosticSequence::new(next);
        let cursor = DiagnosticCursor::new(run.stream_id, sequence);
        let envelope = DiagnosticUpdateEnvelope {
            scope,
            stream_id: run.stream_id,
            sequence,
            emitted_at: Utc::now(),
            update: DiagnosticUpdateKind::Stats(stats.clone()),
        };
        run.latest_sequence = sequence;
        push_bounded(
            &mut run.updates,
            envelope.clone(),
            self.limits.max_updates_per_run,
        );
        let session = state
            .sessions
            .get_mut(&session_key)
            .ok_or(DiagnosticStoreError::Invariant)?;
        session.stats = stats;
        let _ = self.updates.send(Arc::new(envelope));
        Ok(cursor)
    }

    pub fn snapshot(
        &self,
        scope: &DiagnosticScope,
    ) -> Result<Option<DiagnosticSnapshot>, DiagnosticStoreError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let Some(run) = state.run(scope) else {
            return Ok(None);
        };
        let stats = state
            .session(scope)
            .map(|session| session.stats.clone())
            .unwrap_or_default();
        Ok(Some(DiagnosticSnapshot {
            scope: scope.clone(),
            stream_id: run.stream_id,
            prompt: run.prompt.clone(),
            model_calls: run.model_calls.iter().cloned().collect(),
            tool_executions: run.tool_executions.iter().cloned().collect(),
            activity: run.activity.iter().cloned().collect(),
            stats,
            latest_sequence: run.latest_sequence,
        }))
    }

    pub fn updates_after(
        &self,
        scope: &DiagnosticScope,
        after: Option<DiagnosticCursor>,
    ) -> Result<DiagnosticUpdateBatch, DiagnosticStoreError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let Some(run) = state.run(scope) else {
            return Ok(DiagnosticUpdateBatch {
                updates: Vec::new(),
                retention_floor: None,
                latest_cursor: None,
                rebase_required: false,
            });
        };
        let retention_floor = run.updates.front().map(DiagnosticUpdateEnvelope::cursor);
        let rebase_required = match (after, retention_floor) {
            (Some(after), _) if after.stream_id != run.stream_id => true,
            (Some(after), Some(floor)) => {
                after.sequence.as_u64().saturating_add(1) < floor.sequence.as_u64()
            }
            _ => false,
        };
        let updates = run
            .updates
            .iter()
            .filter(|update| {
                after.is_none_or(|cursor| {
                    cursor.stream_id != run.stream_id || update.sequence > cursor.sequence
                })
            })
            .cloned()
            .collect();
        Ok(DiagnosticUpdateBatch {
            updates,
            retention_floor,
            latest_cursor: Some(DiagnosticCursor::new(run.stream_id, run.latest_sequence)),
            rebase_required,
        })
    }

    pub fn subscribe(&self, scope: DiagnosticScope) -> DiagnosticSubscription {
        DiagnosticSubscription {
            scope,
            receiver: self.updates.subscribe(),
        }
    }

    fn record(
        &self,
        scope: DiagnosticScope,
        update: DiagnosticUpdateKind,
        mutate: impl FnOnce(&mut DiagnosticRunState, DiagnosticSequence),
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let run = state.run_mut(&scope, self.limits)?;
        let next = run
            .latest_sequence
            .as_u64()
            .checked_add(1)
            .ok_or(DiagnosticStoreError::SequenceExhausted)?;
        let sequence = DiagnosticSequence::new(next);
        mutate(run, sequence);
        let cursor = DiagnosticCursor::new(run.stream_id, sequence);
        let envelope = DiagnosticUpdateEnvelope {
            scope,
            stream_id: run.stream_id,
            sequence,
            emitted_at: Utc::now(),
            update,
        };
        run.latest_sequence = sequence;
        push_bounded(
            &mut run.updates,
            envelope.clone(),
            self.limits.max_updates_per_run,
        );
        let _ = self.updates.send(Arc::new(envelope));
        Ok(cursor)
    }
}

impl Default for InMemoryDiagnosticStore {
    fn default() -> Self {
        let limits = DiagnosticStoreLimits::default();
        let (updates, _) = broadcast::channel(limits.live_update_capacity);
        Self {
            limits,
            state: Mutex::new(DiagnosticStoreState::default()),
            updates,
        }
    }
}

fn push_bounded<T>(values: &mut VecDeque<T>, value: T, capacity: usize) {
    while values.len() >= capacity {
        values.pop_front();
    }
    values.push_back(value);
}

fn replace_or_push<T>(
    values: &mut VecDeque<T>,
    value: T,
    capacity: usize,
    matches: impl Fn(&T, &T) -> bool,
) {
    if let Some(index) = values.iter().position(|existing| matches(existing, &value)) {
        values.remove(index);
    }
    push_bounded(values, value, capacity);
}

pub struct DiagnosticSubscription {
    scope: DiagnosticScope,
    receiver: broadcast::Receiver<Arc<DiagnosticUpdateEnvelope>>,
}

impl DiagnosticSubscription {
    pub async fn recv(&mut self) -> Result<Arc<DiagnosticUpdateEnvelope>, DiagnosticStoreError> {
        loop {
            match self.receiver.recv().await {
                Ok(update) if update.scope == self.scope => return Ok(update),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    return Err(DiagnosticStoreError::SubscriberLagged(count));
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(DiagnosticStoreError::SubscriptionClosed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use chrono::Utc;
    use ironclaw_host_api::{
        ids::{TenantId, ThreadId, UserId},
        turn::TurnRunId,
    };
    use ironclaw_product_contracts::inspector::{
        DiagnosticActivityEvent, DiagnosticActivityKind, DiagnosticModelCallId, DiagnosticScope,
        InspectorModelCallStatus, ModelCallDiagnostic, PromptDiagnostic, ToolExecutionDiagnostic,
        ToolExecutionStatus,
    };

    use super::*;

    fn scope(tenant: &str, user: &str, thread: &str, run_id: TurnRunId) -> DiagnosticScope {
        DiagnosticScope::new(
            TenantId::new(tenant).expect("tenant"),
            UserId::new(user).expect("user"),
            ThreadId::new(thread).expect("thread"),
            run_id,
        )
    }

    fn activity(summary: &str) -> DiagnosticActivityEvent {
        DiagnosticActivityEvent::new(
            Utc::now(),
            DiagnosticActivityKind::Progress,
            None,
            None,
            None,
            Some(summary.to_string()),
        )
    }

    fn tiny_limits() -> DiagnosticStoreLimits {
        DiagnosticStoreLimits {
            max_sessions: 2,
            max_runs_per_session: 2,
            max_model_calls_per_run: 2,
            max_tool_executions_per_run: 2,
            max_activity_entries_per_run: 2,
            max_updates_per_run: 2,
            live_update_capacity: 8,
        }
    }

    #[test]
    fn rejects_zero_limits() {
        let mut limits = tiny_limits();
        limits.max_sessions = 0;
        assert_eq!(
            InMemoryDiagnosticStore::new(limits).expect_err("zero must fail"),
            DiagnosticStoreError::InvalidLimit("max_sessions")
        );
    }

    #[test]
    fn rejects_limits_above_the_hard_ceiling() {
        let mut limits = tiny_limits();
        limits.max_sessions = DEFAULT_MAX_TRACKED_SESSIONS + 1;
        assert_eq!(
            InMemoryDiagnosticStore::new(limits).expect_err("oversized limit must fail"),
            DiagnosticStoreError::LimitExceedsMaximum {
                name: "max_sessions",
                maximum: DEFAULT_MAX_TRACKED_SESSIONS,
            }
        );
    }

    #[test]
    fn exact_scope_keys_prevent_cross_scope_reads() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let run_id = TurnRunId::new();
        let allowed = scope("tenant-a", "user-a", "thread-a", run_id);
        store
            .record_activity(allowed.clone(), activity("allowed"))
            .expect("record");

        for denied in [
            scope("tenant-b", "user-a", "thread-a", run_id),
            scope("tenant-a", "user-b", "thread-a", run_id),
            scope("tenant-a", "user-a", "thread-b", run_id),
            scope("tenant-a", "user-a", "thread-a", TurnRunId::new()),
        ] {
            assert!(store.snapshot(&denied).expect("snapshot").is_none());
        }
        assert!(store.snapshot(&allowed).expect("snapshot").is_some());
    }

    #[test]
    fn prompt_model_and_tool_records_share_one_scoped_snapshot() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let prompt = PromptDiagnostic::new(
            Utc::now(),
            Vec::new(),
            "system prompt",
            Some(10),
            1,
            0,
            1,
            Vec::new(),
            1,
            Some("requested-model".to_string()),
            Some("effective-model".to_string()),
            Some(100_000),
        );
        let model_call = ModelCallDiagnostic::new(
            DiagnosticModelCallId::new(),
            1,
            "requested-model",
            Some("effective-model".to_string()),
            Utc::now(),
            Some(Utc::now()),
            Some(25),
            InspectorModelCallStatus::Succeeded,
            None,
            None,
        );
        let tool = ToolExecutionDiagnostic::new(
            ironclaw_host_api::turn::CapabilityActivityId::new(),
            Some(model_call.call_id),
            "filesystem.read",
            Some("{}".to_string()),
            Some("result".to_string()),
            ToolExecutionStatus::Succeeded,
            Some(5),
            None,
            None,
            None,
        );
        store.record_prompt(scope.clone(), prompt).expect("prompt");
        store
            .record_model_call(scope.clone(), model_call)
            .expect("model call");
        store
            .record_tool_execution(scope.clone(), tool)
            .expect("tool");

        let snapshot = store.snapshot(&scope).expect("snapshot").expect("present");
        assert!(snapshot.prompt.is_some());
        assert_eq!(snapshot.model_calls.len(), 1);
        assert_eq!(snapshot.tool_executions.len(), 1);
        assert_eq!(snapshot.latest_sequence, DiagnosticSequence::new(3));
    }

    #[test]
    fn session_eviction_is_deterministic_and_write_lru() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let first = scope("tenant", "user-1", "thread", TurnRunId::new());
        let second = scope("tenant", "user-2", "thread", TurnRunId::new());
        let third = scope("tenant", "user-3", "thread", TurnRunId::new());
        store
            .record_activity(first.clone(), activity("first"))
            .expect("record first");
        store
            .record_activity(second.clone(), activity("second"))
            .expect("record second");
        store
            .record_activity(first.clone(), activity("touch first"))
            .expect("touch first");
        store
            .record_activity(third.clone(), activity("third"))
            .expect("record third");

        assert!(store.snapshot(&first).expect("first").is_some());
        assert!(store.snapshot(&second).expect("second").is_none());
        assert!(store.snapshot(&third).expect("third").is_some());
    }

    #[test]
    fn run_eviction_is_deterministic_and_write_lru() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let first = scope("tenant", "user", "thread", TurnRunId::new());
        let second = scope("tenant", "user", "thread", TurnRunId::new());
        let third = scope("tenant", "user", "thread", TurnRunId::new());
        store
            .record_activity(first.clone(), activity("first"))
            .expect("first");
        store
            .record_activity(second.clone(), activity("second"))
            .expect("second");
        store
            .record_activity(first.clone(), activity("touch"))
            .expect("touch");
        store
            .record_activity(third.clone(), activity("third"))
            .expect("third");

        assert!(store.snapshot(&first).expect("first").is_some());
        assert!(store.snapshot(&second).expect("second").is_none());
        assert!(store.snapshot(&third).expect("third").is_some());
    }

    #[test]
    fn activity_and_update_history_are_bounded_and_ordered() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let mut stream_id = None;
        for index in 1..=3 {
            let cursor = store
                .record_activity(scope.clone(), activity(&format!("entry-{index}")))
                .expect("record");
            stream_id = Some(cursor.stream_id);
            assert_eq!(cursor.sequence.as_u64(), index);
        }
        let stream_id = stream_id.expect("stream id");

        let snapshot = store.snapshot(&scope).expect("snapshot").expect("present");
        let sequences: Vec<_> = snapshot
            .activity
            .iter()
            .map(|entry| entry.sequence.as_u64())
            .collect();
        assert_eq!(sequences, vec![2, 3]);

        let batch = store
            .updates_after(
                &scope,
                Some(DiagnosticCursor::new(stream_id, DiagnosticSequence::ZERO)),
            )
            .expect("updates");
        assert!(batch.rebase_required);
        assert_eq!(
            batch
                .updates
                .iter()
                .map(|update| update.sequence.as_u64())
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            batch.retention_floor,
            Some(DiagnosticCursor::new(stream_id, DiagnosticSequence::new(2),))
        );
        assert_eq!(
            batch.latest_cursor,
            Some(DiagnosticCursor::new(stream_id, DiagnosticSequence::new(3),))
        );
    }

    #[test]
    fn recreating_an_evicted_run_changes_the_stream_generation() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let first = scope("tenant", "user", "thread", TurnRunId::new());
        let second = scope("tenant", "user", "thread", TurnRunId::new());
        let third = scope("tenant", "user", "thread", TurnRunId::new());
        let original = store
            .record_activity(first.clone(), activity("first"))
            .expect("first");
        store
            .record_activity(second, activity("second"))
            .expect("second");
        store
            .record_activity(third, activity("third"))
            .expect("third");
        let recreated = store
            .record_activity(first.clone(), activity("recreated"))
            .expect("recreated");
        assert_ne!(original.stream_id, recreated.stream_id);

        let batch = store
            .updates_after(&first, Some(original))
            .expect("rebase batch");
        assert!(batch.rebase_required);
        assert_eq!(batch.updates.len(), 1);
        assert_eq!(batch.latest_cursor, Some(recreated));
    }

    #[tokio::test]
    async fn subscription_filters_scope_and_preserves_sequence() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let allowed = scope("tenant", "user", "thread-a", TurnRunId::new());
        let other = scope("tenant", "user", "thread-b", TurnRunId::new());
        let mut subscription = store.subscribe(allowed.clone());
        store
            .record_activity(other, activity("other"))
            .expect("other");
        store
            .record_activity(allowed, activity("allowed"))
            .expect("allowed");
        let update = subscription.recv().await.expect("matching update");
        assert_eq!(update.sequence, DiagnosticSequence::new(1));
    }

    #[tokio::test]
    async fn stats_are_visible_when_the_update_is_delivered() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let mut subscription = store.subscribe(scope.clone());
        let stats = SessionDiagnosticStats {
            total_model_calls: 7,
            ..SessionDiagnosticStats::default()
        };
        store
            .record_stats(scope.clone(), stats)
            .expect("record stats");
        let update = subscription.recv().await.expect("stats update");
        assert!(matches!(update.update, DiagnosticUpdateKind::Stats(_)));
        let snapshot = store.snapshot(&scope).expect("snapshot").expect("present");
        assert_eq!(snapshot.stats.total_model_calls, 7);
    }

    #[test]
    fn poisoned_state_returns_a_redacted_error() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = store.state.lock().expect("lock before poison");
            panic!("poison store lock");
        }));
        let error = store
            .snapshot(&scope("tenant", "user", "thread", TurnRunId::new()))
            .expect_err("poison must fail closed");
        assert_eq!(error, DiagnosticStoreError::StateUnavailable);
        assert!(!error.to_string().contains("poison"));
    }
}
