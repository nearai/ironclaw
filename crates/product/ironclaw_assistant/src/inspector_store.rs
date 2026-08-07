//! Bounded, process-local storage for operator inspection diagnostics.
//!
//! The store deliberately has no persistence backend. It keeps raw diagnostic
//! content out of durable events and drops all state at process restart.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, OnceLock, RwLock};

use chrono::Utc;
use ironclaw_host_api::{
    ids::{TenantId, ThreadId, UserId},
    turn::TurnRunId,
};
#[cfg(test)]
use ironclaw_loop_host::HostManagedModelCallDiagnostic;
use ironclaw_loop_host::{
    HostManagedModelCallDiagnosticCapture, HostManagedModelCallDiagnosticOutcome,
    HostManagedModelMessageRole, HostManagedPromptDiagnosticCapture,
    HostManagedPromptDiagnosticMessage, HostManagedPromptDiagnosticSink,
    estimate_tokens_from_chars,
};
use ironclaw_product_contracts::inspector::{
    DEFAULT_MAX_ACTIVITY_ENTRIES, DEFAULT_MAX_LIVE_UPDATE_SCOPES, DEFAULT_MAX_MODEL_CALLS_PER_RUN,
    DEFAULT_MAX_RETAINED_RUNS_PER_SESSION, DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
    DEFAULT_MAX_TOOL_EXECUTIONS_PER_RUN, DEFAULT_MAX_TRACKED_SESSIONS, DiagnosticActivityEntry,
    DiagnosticActivityEvent, DiagnosticActivityKind, DiagnosticCursor, DiagnosticMetricTotal,
    DiagnosticModelCallId, DiagnosticModelCount, DiagnosticScope, DiagnosticSequence,
    DiagnosticSnapshot, DiagnosticStreamId, DiagnosticUpdateBatch, DiagnosticUpdateEnvelope,
    DiagnosticUpdateKind, InspectorModelCallStatus, MAX_MODELS_IN_STATS, ModelCallDiagnostic,
    ModelTokenUsage, PromptComponentDiagnostic, PromptComponentKind, PromptDiagnostic,
    SessionDiagnosticStats, ToolExecutionDiagnostic, ToolExecutionStatus,
};
use ironclaw_safety::LeakDetector;
use thiserror::Error;
use tokio::sync::broadcast;

// Stats replacement identity must outlive the smaller operator-display queues.
// Bound it to the maximum retained diagnostic update horizon instead.
const MAX_STATS_RECORDS_PER_RUN: usize = DEFAULT_MAX_RETAINED_UPDATES_PER_RUN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticStoreLimits {
    pub max_sessions: usize,
    pub max_runs_per_session: usize,
    pub max_model_calls_per_run: usize,
    pub max_tool_executions_per_run: usize,
    pub max_activity_entries_per_run: usize,
    pub max_updates_per_run: usize,
    pub max_live_update_scopes: usize,
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
            max_live_update_scopes: DEFAULT_MAX_LIVE_UPDATE_SCOPES,
            live_update_capacity: DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
        }
    }
}

impl DiagnosticStoreLimits {
    fn validate(self) -> Result<Self, DiagnosticStoreError> {
        // Keep this destructuring exhaustive: adding a limit field must fail
        // compilation until its validation ceiling is defined below.
        let Self {
            max_sessions,
            max_runs_per_session,
            max_model_calls_per_run,
            max_tool_executions_per_run,
            max_activity_entries_per_run,
            max_updates_per_run,
            max_live_update_scopes,
            live_update_capacity,
        } = self;
        let values = [
            ("max_sessions", max_sessions, DEFAULT_MAX_TRACKED_SESSIONS),
            (
                "max_runs_per_session",
                max_runs_per_session,
                DEFAULT_MAX_RETAINED_RUNS_PER_SESSION,
            ),
            (
                "max_model_calls_per_run",
                max_model_calls_per_run,
                DEFAULT_MAX_MODEL_CALLS_PER_RUN,
            ),
            (
                "max_tool_executions_per_run",
                max_tool_executions_per_run,
                DEFAULT_MAX_TOOL_EXECUTIONS_PER_RUN,
            ),
            (
                "max_activity_entries_per_run",
                max_activity_entries_per_run,
                DEFAULT_MAX_ACTIVITY_ENTRIES,
            ),
            (
                "max_updates_per_run",
                max_updates_per_run,
                DEFAULT_MAX_RETAINED_UPDATES_PER_RUN,
            ),
            (
                "max_live_update_scopes",
                max_live_update_scopes,
                DEFAULT_MAX_LIVE_UPDATE_SCOPES,
            ),
            (
                "live_update_capacity",
                live_update_capacity,
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
    model_stats_records: HashMap<DiagnosticModelCallId, ModelStatsContribution>,
    model_stats_order: VecDeque<DiagnosticModelCallId>,
    tool_stats_records:
        HashMap<ironclaw_host_api::turn::CapabilityActivityId, ToolStatsContribution>,
    tool_stats_order: VecDeque<ironclaw_host_api::turn::CapabilityActivityId>,
    activity: VecDeque<DiagnosticActivityEntry>,
    stats: SessionDiagnosticStats,
    updates: VecDeque<DiagnosticUpdateEnvelope>,
    latest_sequence: DiagnosticSequence,
    turn_started_recorded: bool,
}

impl Default for DiagnosticRunState {
    fn default() -> Self {
        Self {
            stream_id: DiagnosticStreamId::new(),
            prompt: None,
            model_calls: VecDeque::new(),
            tool_executions: VecDeque::new(),
            model_stats_records: HashMap::new(),
            model_stats_order: VecDeque::new(),
            tool_stats_records: HashMap::new(),
            tool_stats_order: VecDeque::new(),
            activity: VecDeque::new(),
            stats: SessionDiagnosticStats::default(),
            updates: VecDeque::new(),
            latest_sequence: DiagnosticSequence::ZERO,
            turn_started_recorded: false,
        }
    }
}

#[derive(Debug, Default)]
struct DiagnosticSessionState {
    runs: HashMap<TurnRunId, DiagnosticRunState>,
    run_order: VecDeque<TurnRunId>,
}

#[derive(Debug, Default)]
struct DiagnosticStoreState {
    sessions: HashMap<DiagnosticSessionKey, DiagnosticSessionState>,
    session_order: VecDeque<DiagnosticSessionKey>,
    live_updates: HashMap<DiagnosticScope, broadcast::Sender<Arc<DiagnosticUpdateEnvelope>>>,
    live_update_order: VecDeque<DiagnosticScope>,
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

    fn subscribe(
        &mut self,
        scope: DiagnosticScope,
        capacity: usize,
        max_scopes: usize,
    ) -> Result<broadcast::Receiver<Arc<DiagnosticUpdateEnvelope>>, DiagnosticStoreError> {
        self.prune_inactive_live_updates();
        if !self.live_updates.contains_key(&scope) {
            while self.live_updates.len() >= max_scopes {
                let evicted = self
                    .live_update_order
                    .pop_front()
                    .ok_or(DiagnosticStoreError::Invariant)?;
                self.live_updates
                    .remove(&evicted)
                    .ok_or(DiagnosticStoreError::Invariant)?;
            }
            let (sender, _) = broadcast::channel(capacity);
            self.live_updates.insert(scope.clone(), sender);
        }
        touch(&mut self.live_update_order, scope.clone());
        self.live_updates
            .get(&scope)
            .map(broadcast::Sender::subscribe)
            .ok_or(DiagnosticStoreError::Invariant)
    }

    fn send_live_update(&mut self, envelope: DiagnosticUpdateEnvelope) {
        let scope = envelope.scope.clone();
        let has_no_receivers = self
            .live_updates
            .get(&scope)
            .is_some_and(|sender| sender.send(Arc::new(envelope)).is_err());
        if has_no_receivers {
            self.live_updates.remove(&scope);
            if let Some(index) = self
                .live_update_order
                .iter()
                .position(|entry| entry == &scope)
            {
                self.live_update_order.remove(index);
            }
        }
    }

    fn prune_inactive_live_updates(&mut self) {
        self.live_updates
            .retain(|_, sender| sender.receiver_count() > 0);
        self.live_update_order
            .retain(|scope| self.live_updates.contains_key(scope));
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
    state: RwLock<DiagnosticStoreState>,
}

/// Operator inspection store surface exposed by product composition.
///
/// Capture remains behind [`HostManagedPromptDiagnosticSink`]; consumers of
/// diagnostics do not depend on the in-memory implementation.
pub trait DiagnosticStorePort: Send + Sync {
    fn record_activity(
        &self,
        scope: DiagnosticScope,
        event: DiagnosticActivityEvent,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError>;

    fn snapshot(
        &self,
        scope: &DiagnosticScope,
    ) -> Result<Option<DiagnosticSnapshot>, DiagnosticStoreError>;

    fn prompt(
        &self,
        scope: &DiagnosticScope,
    ) -> Result<Option<PromptDiagnostic>, DiagnosticStoreError>;

    fn tool_execution(
        &self,
        scope: &DiagnosticScope,
        activity_id: ironclaw_host_api::turn::CapabilityActivityId,
    ) -> Result<Option<ToolExecutionDiagnostic>, DiagnosticStoreError>;

    fn updates_after(
        &self,
        scope: &DiagnosticScope,
        after: Option<DiagnosticCursor>,
    ) -> Result<DiagnosticUpdateBatch, DiagnosticStoreError>;
}

impl InMemoryDiagnosticStore {
    pub fn new(limits: DiagnosticStoreLimits) -> Result<Self, DiagnosticStoreError> {
        let limits = limits.validate()?;
        Ok(Self {
            limits,
            state: RwLock::new(DiagnosticStoreState::default()),
        })
    }

    pub fn record_prompt(
        &self,
        scope: DiagnosticScope,
        prompt: PromptDiagnostic,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let prompt = prompt.into_bounded();
        self.record(scope, move |run, _| {
            let update = DiagnosticUpdateKind::PromptUpdated {
                component_count: u32::try_from(prompt.components.len()).unwrap_or(u32::MAX),
                total_estimated_tokens: prompt.total_estimated_tokens,
                truncated: prompt.any_content_truncated(),
            };
            run.prompt = Some(prompt);
            update
        })
    }

    pub fn record_model_call(
        &self,
        scope: DiagnosticScope,
        model_call: ModelCallDiagnostic,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let model_call = model_call.into_bounded();
        let cap = self.limits.max_model_calls_per_run;
        self.record(scope, move |run, _| {
            let contribution = ModelStatsContribution::from(&model_call);
            let existing = replace_bounded_state(
                &mut run.model_stats_records,
                &mut run.model_stats_order,
                model_call.call_id,
                contribution.clone(),
                MAX_STATS_RECORDS_PER_RUN,
            );
            if let Some(existing) = existing.as_ref() {
                update_model_stats(&mut run.stats, existing, false);
            }
            update_model_stats(&mut run.stats, &contribution, true);
            let update = DiagnosticUpdateKind::ModelCall(model_call.clone());
            replace_or_push(
                &mut run.model_calls,
                model_call,
                cap,
                |existing, incoming| existing.call_id == incoming.call_id,
            );
            update
        })
    }

    pub fn record_tool_execution(
        &self,
        scope: DiagnosticScope,
        tool: ToolExecutionDiagnostic,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let tool = tool.into_bounded();
        let cap = self.limits.max_tool_executions_per_run;
        self.record(scope, move |run, _| {
            let contribution = ToolStatsContribution::from(&tool);
            let existing = replace_bounded_state(
                &mut run.tool_stats_records,
                &mut run.tool_stats_order,
                tool.activity_id,
                contribution,
                MAX_STATS_RECORDS_PER_RUN,
            );
            if let Some(existing) = existing.as_ref() {
                update_tool_stats(&mut run.stats, existing, false);
            }
            update_tool_stats(&mut run.stats, &contribution, true);
            let update = DiagnosticUpdateKind::ToolExecutionUpdated {
                activity_id: tool.activity_id,
                model_call_id: tool.model_call_id,
                capability_name: tool.capability_name.clone(),
                status: tool.status,
                duration_ms: tool.duration_ms,
                output_bytes: tool.output_bytes,
                result_truncated: tool.result_truncated(),
            };
            replace_or_push(&mut run.tool_executions, tool, cap, |existing, incoming| {
                existing.activity_id == incoming.activity_id
            });
            update
        })
    }

    pub fn record_activity(
        &self,
        scope: DiagnosticScope,
        event: DiagnosticActivityEvent,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let event = event.into_bounded();
        let cap = self.limits.max_activity_entries_per_run;
        self.record(scope, move |run, sequence| {
            let update = DiagnosticUpdateKind::Activity(event.clone());
            push_bounded(
                &mut run.activity,
                DiagnosticActivityEntry { sequence, event },
                cap,
            );
            update
        })
    }

    fn record_turn_started_once(
        &self,
        scope: DiagnosticScope,
        event: DiagnosticActivityEvent,
    ) -> Result<Option<DiagnosticCursor>, DiagnosticStoreError> {
        let event = event.into_bounded();
        let cap = self.limits.max_activity_entries_per_run;
        self.record_optional(scope, move |run, sequence| {
            if run.turn_started_recorded {
                return None;
            }
            run.turn_started_recorded = true;
            let update = DiagnosticUpdateKind::Activity(event.clone());
            push_bounded(
                &mut run.activity,
                DiagnosticActivityEntry { sequence, event },
                cap,
            );
            Some(update)
        })
    }

    pub fn record_stats(
        &self,
        scope: DiagnosticScope,
        stats: SessionDiagnosticStats,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        let stats = stats.into_bounded();
        self.record(scope, move |run, _| {
            run.stats = stats.clone();
            DiagnosticUpdateKind::Stats(stats)
        })
    }

    pub fn snapshot(
        &self,
        scope: &DiagnosticScope,
    ) -> Result<Option<DiagnosticSnapshot>, DiagnosticStoreError> {
        let state = self
            .state
            .read()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let Some(run) = state.run(scope) else {
            return Ok(None);
        };
        Ok(Some(DiagnosticSnapshot {
            scope: scope.clone(),
            stream_id: run.stream_id,
            prompt: run.prompt.clone(),
            model_calls: run.model_calls.iter().cloned().collect(),
            tool_executions: run.tool_executions.iter().cloned().collect(),
            activity: run.activity.iter().cloned().collect(),
            stats: run.stats.clone(),
            latest_sequence: run.latest_sequence,
        }))
    }

    pub fn prompt(
        &self,
        scope: &DiagnosticScope,
    ) -> Result<Option<PromptDiagnostic>, DiagnosticStoreError> {
        let state = self
            .state
            .read()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        Ok(state.run(scope).and_then(|run| run.prompt.clone()))
    }

    pub fn tool_execution(
        &self,
        scope: &DiagnosticScope,
        activity_id: ironclaw_host_api::turn::CapabilityActivityId,
    ) -> Result<Option<ToolExecutionDiagnostic>, DiagnosticStoreError> {
        let state = self
            .state
            .read()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        Ok(state.run(scope).and_then(|run| {
            run.tool_executions
                .iter()
                .find(|tool| tool.activity_id == activity_id)
                .cloned()
        }))
    }

    pub fn updates_after(
        &self,
        scope: &DiagnosticScope,
        after: Option<DiagnosticCursor>,
    ) -> Result<DiagnosticUpdateBatch, DiagnosticStoreError> {
        let state = self
            .state
            .read()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let Some(run) = state.run(scope) else {
            return Ok(DiagnosticUpdateBatch {
                updates: Vec::new(),
                retention_floor: None,
                latest_cursor: None,
                rebase_required: after.is_some(),
            });
        };
        let retention_floor = run.updates.front().map(DiagnosticUpdateEnvelope::cursor);
        let rebase_required = match (after, retention_floor) {
            (Some(after), _) if after.stream_id != run.stream_id => true,
            (Some(after), _) if after.sequence > run.latest_sequence => true,
            (Some(after), Some(floor)) => {
                after.sequence.as_u64().saturating_add(1) < floor.sequence.as_u64()
            }
            (None, Some(floor)) => floor.sequence.as_u64() > 1,
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

    pub fn subscribe(
        &self,
        scope: DiagnosticScope,
    ) -> Result<DiagnosticSubscription, DiagnosticStoreError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let receiver = state.subscribe(
            scope,
            self.limits.live_update_capacity,
            self.limits.max_live_update_scopes,
        )?;
        Ok(DiagnosticSubscription { receiver })
    }

    fn record(
        &self,
        scope: DiagnosticScope,
        mutate: impl FnOnce(&mut DiagnosticRunState, DiagnosticSequence) -> DiagnosticUpdateKind,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        self.record_optional(scope, |run, sequence| Some(mutate(run, sequence)))?
            .ok_or(DiagnosticStoreError::Invariant)
    }

    fn record_optional(
        &self,
        scope: DiagnosticScope,
        mutate: impl FnOnce(&mut DiagnosticRunState, DiagnosticSequence) -> Option<DiagnosticUpdateKind>,
    ) -> Result<Option<DiagnosticCursor>, DiagnosticStoreError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| DiagnosticStoreError::StateUnavailable)?;
        let run = state.run_mut(&scope, self.limits)?;
        let next = run
            .latest_sequence
            .as_u64()
            .checked_add(1)
            .ok_or(DiagnosticStoreError::SequenceExhausted)?;
        let sequence = DiagnosticSequence::new(next);
        let Some(update) = mutate(run, sequence) else {
            return Ok(None);
        };
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
        state.send_live_update(envelope);
        Ok(Some(cursor))
    }
}

fn update_counter(counter: &mut u64, value: u64, add: bool) {
    *counter = if add {
        counter.saturating_add(value)
    } else {
        counter.saturating_sub(value)
    };
}

fn update_metric(metric: &mut DiagnosticMetricTotal, value: Option<u64>, add: bool) {
    match value {
        Some(value) => update_counter(&mut metric.known_total, value, add),
        None => update_counter(&mut metric.unavailable_samples, 1, add),
    }
}

#[derive(Debug, Clone)]
struct ModelStatsContribution {
    model: String,
    usage: Option<ModelTokenUsage>,
    duration_ms: Option<u64>,
}

impl From<&ModelCallDiagnostic> for ModelStatsContribution {
    fn from(call: &ModelCallDiagnostic) -> Self {
        Self {
            model: call
                .effective_model
                .as_ref()
                .unwrap_or(&call.requested_model)
                .content()
                .to_string(),
            usage: call.usage.clone(),
            duration_ms: call.duration_ms,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ToolStatsContribution {
    status: ToolExecutionStatus,
}

impl From<&ToolExecutionDiagnostic> for ToolStatsContribution {
    fn from(tool: &ToolExecutionDiagnostic) -> Self {
        Self {
            status: tool.status,
        }
    }
}

fn update_model_count(stats: &mut SessionDiagnosticStats, model: &str, add: bool) {
    if let Some(index) = stats
        .calls_per_model
        .iter()
        .position(|entry| entry.model.content() == model)
    {
        update_counter(&mut stats.calls_per_model[index].calls, 1, add);
        if stats.calls_per_model[index].calls == 0 {
            stats.calls_per_model.remove(index);
        }
    } else if add {
        if stats.calls_per_model.len() < MAX_MODELS_IN_STATS {
            stats
                .calls_per_model
                .push(DiagnosticModelCount::new(model, 1));
        } else {
            // This is a monotonic completeness marker. Once a model bucket was
            // omitted, later removals cannot reconstruct its historical count,
            // so clearing the flag would incorrectly claim an exact breakdown.
            stats.calls_per_model_truncated = true;
        }
    }
}

fn update_model_stats(
    stats: &mut SessionDiagnosticStats,
    call: &ModelStatsContribution,
    add: bool,
) {
    update_counter(&mut stats.total_model_calls, 1, add);
    update_model_count(stats, &call.model, add);
    match call.usage.as_ref() {
        Some(usage) => {
            update_metric(&mut stats.input_tokens, usage.input_tokens, add);
            update_metric(&mut stats.output_tokens, usage.output_tokens, add);
            update_metric(
                &mut stats.cache_read_input_tokens,
                usage.cache_read_input_tokens,
                add,
            );
            update_metric(
                &mut stats.cache_creation_input_tokens,
                usage.cache_creation_input_tokens,
                add,
            );
        }
        None => {
            update_metric(&mut stats.input_tokens, None, add);
            update_metric(&mut stats.output_tokens, None, add);
            update_metric(&mut stats.cache_read_input_tokens, None, add);
            update_metric(&mut stats.cache_creation_input_tokens, None, add);
        }
    }
    update_metric(&mut stats.total_latency_ms, call.duration_ms, add);
}

fn update_tool_stats(stats: &mut SessionDiagnosticStats, tool: &ToolStatsContribution, add: bool) {
    update_counter(&mut stats.total_tool_calls, 1, add);
    match tool.status {
        ToolExecutionStatus::Succeeded => {
            update_counter(&mut stats.successful_tool_calls, 1, add);
        }
        ToolExecutionStatus::Failed => {
            update_counter(&mut stats.failed_tool_calls, 1, add);
        }
        ToolExecutionStatus::Started => {}
    }
}

fn replace_bounded_state<K, V>(
    values: &mut HashMap<K, V>,
    order: &mut VecDeque<K>,
    key: K,
    value: V,
    capacity: usize,
) -> Option<V>
where
    K: Clone + Eq + Hash,
{
    let previous = values.insert(key.clone(), value);
    touch(order, key);
    while values.len() > capacity {
        let Some(evicted) = order.pop_front() else {
            break;
        };
        values.remove(&evicted);
    }
    previous
}

impl DiagnosticStorePort for InMemoryDiagnosticStore {
    fn record_activity(
        &self,
        scope: DiagnosticScope,
        event: DiagnosticActivityEvent,
    ) -> Result<DiagnosticCursor, DiagnosticStoreError> {
        InMemoryDiagnosticStore::record_activity(self, scope, event)
    }

    fn snapshot(
        &self,
        scope: &DiagnosticScope,
    ) -> Result<Option<DiagnosticSnapshot>, DiagnosticStoreError> {
        InMemoryDiagnosticStore::snapshot(self, scope)
    }

    fn prompt(
        &self,
        scope: &DiagnosticScope,
    ) -> Result<Option<PromptDiagnostic>, DiagnosticStoreError> {
        InMemoryDiagnosticStore::prompt(self, scope)
    }

    fn tool_execution(
        &self,
        scope: &DiagnosticScope,
        activity_id: ironclaw_host_api::turn::CapabilityActivityId,
    ) -> Result<Option<ToolExecutionDiagnostic>, DiagnosticStoreError> {
        InMemoryDiagnosticStore::tool_execution(self, scope, activity_id)
    }

    fn updates_after(
        &self,
        scope: &DiagnosticScope,
        after: Option<DiagnosticCursor>,
    ) -> Result<DiagnosticUpdateBatch, DiagnosticStoreError> {
        InMemoryDiagnosticStore::updates_after(self, scope, after)
    }
}

fn diagnostic_prompt_text(detector: &LeakDetector, value: &str) -> String {
    let validated = value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .collect::<String>();
    detector.redact_all_secrets(&validated).0
}

fn prompt_leak_detector() -> &'static LeakDetector {
    static DETECTOR: OnceLock<LeakDetector> = OnceLock::new();
    DETECTOR.get_or_init(LeakDetector::new)
}

fn prompt_component_kind(
    index: usize,
    identity_message_count: u32,
    message: &HostManagedPromptDiagnosticMessage,
) -> PromptComponentKind {
    let content_ref = message.content_ref.as_str();
    if index < identity_message_count as usize {
        PromptComponentKind::Identity
    } else if content_ref.starts_with("msg:instruction.") {
        PromptComponentKind::Instruction
    } else if content_ref.starts_with("msg:snippet.") {
        PromptComponentKind::Skill
    } else if content_ref.contains("capability") {
        PromptComponentKind::Capability
    } else {
        match message.role {
            HostManagedModelMessageRole::System => PromptComponentKind::System,
            HostManagedModelMessageRole::User
            | HostManagedModelMessageRole::Assistant
            | HostManagedModelMessageRole::ToolResult => PromptComponentKind::Conversation,
        }
    }
}

fn prompt_component_label(kind: PromptComponentKind, index: usize) -> String {
    let source = match kind {
        PromptComponentKind::System => "System",
        PromptComponentKind::Identity => "Identity",
        PromptComponentKind::Instruction => "Instruction",
        PromptComponentKind::Skill => "Skill",
        PromptComponentKind::Capability => "Capability",
        PromptComponentKind::Conversation => "Conversation",
        PromptComponentKind::Other => "Other",
    };
    format!("{source} {}", index.saturating_add(1))
}

impl HostManagedPromptDiagnosticSink for InMemoryDiagnosticStore {
    fn record_prompt(&self, capture: HostManagedPromptDiagnosticCapture) {
        let user_id = capture
            .context
            .actor
            .as_ref()
            .map(|actor| actor.user_id.clone())
            .or_else(|| capture.context.scope.explicit_owner_user_id().cloned());
        let Some(user_id) = user_id else {
            tracing::debug!(
                run_id = %capture.context.run_id,
                "prompt diagnostics skipped because the run has no user scope"
            );
            return;
        };
        let scope = DiagnosticScope::new(
            capture.context.scope.tenant_id.clone(),
            user_id,
            capture.context.thread_id.clone(),
            capture.context.run_id,
        );
        let detector = prompt_leak_detector();
        let reconstruction_capacity = capture
            .messages
            .iter()
            .map(|message| message.content.len().saturating_add(32))
            .sum();
        let mut reconstructed = String::with_capacity(reconstruction_capacity);
        let mut total_estimated_tokens = 0u64;
        let mut components = Vec::with_capacity(
            capture
                .messages
                .len()
                .saturating_add(capture.capability_ids.len()),
        );
        for (index, message) in capture.messages.iter().enumerate() {
            let kind = prompt_component_kind(index, capture.identity_message_count, message);
            let label = prompt_component_label(kind, index);
            let content = diagnostic_prompt_text(detector, &message.content);
            let estimated_tokens = estimate_tokens_from_chars(&message.content).as_u64();
            total_estimated_tokens = total_estimated_tokens.saturating_add(estimated_tokens);
            if !reconstructed.is_empty() {
                reconstructed.push_str("\n\n");
            }
            reconstructed.push_str(&label);
            reconstructed.push_str(":\n");
            reconstructed.push_str(&content);
            components.push(PromptComponentDiagnostic::new(
                kind,
                label,
                content,
                Some(estimated_tokens),
            ));
        }
        for capability_id in &capture.capability_ids {
            components.push(PromptComponentDiagnostic::new(
                PromptComponentKind::Capability,
                "Capability surface",
                capability_id.as_str(),
                Some(0),
            ));
        }
        let active_skills = capture
            .active_skills
            .iter()
            .map(|skill| diagnostic_prompt_text(detector, skill.as_str()))
            .collect();
        let requested_model = capture
            .requested_model
            .as_ref()
            .map(|model| diagnostic_prompt_text(detector, model.as_str()));
        let effective_model = capture
            .effective_model
            .as_ref()
            .map(|model| diagnostic_prompt_text(detector, model.as_str()));
        let prompt = PromptDiagnostic::new(
            Utc::now(),
            components,
            reconstructed,
            Some(total_estimated_tokens),
            u32::try_from(capture.messages.len()).unwrap_or(u32::MAX),
            capture.identity_message_count,
            capture.instruction_snippet_count,
            active_skills,
            u32::try_from(capture.capability_ids.len()).unwrap_or(u32::MAX),
            requested_model,
            effective_model,
            Some(capture.context_limit),
        );
        if let Err(error) = self.record_turn_started_once(
            scope.clone(),
            DiagnosticActivityEvent::new(
                Utc::now(),
                DiagnosticActivityKind::TurnStarted,
                None,
                None,
                None,
                Some("Turn started".to_string()),
            ),
        ) {
            tracing::debug!(%error, "turn-start diagnostics could not be retained");
        }
        if let Err(error) = InMemoryDiagnosticStore::record_prompt(self, scope.clone(), prompt) {
            tracing::debug!(%error, "prompt diagnostics could not be retained");
        }
        if let Err(error) = self.record_activity(
            scope,
            DiagnosticActivityEvent::new(
                Utc::now(),
                DiagnosticActivityKind::PromptPrepared,
                None,
                None,
                None,
                Some("Prompt prepared".to_string()),
            ),
        ) {
            tracing::debug!(%error, "prompt activity diagnostics could not be retained");
        }
    }

    fn record_model_call(&self, capture: HostManagedModelCallDiagnosticCapture) {
        let (
            diagnostic,
            completed_at,
            duration_ms,
            status,
            usage,
            failure_summary,
            kind,
            default_summary,
        ) = match capture {
            HostManagedModelCallDiagnosticCapture::Started(diagnostic) => (
                diagnostic,
                None,
                None,
                InspectorModelCallStatus::Started,
                None,
                None,
                DiagnosticActivityKind::ModelCallStarted,
                Some("Model call started".to_string()),
            ),
            HostManagedModelCallDiagnosticCapture::Completed {
                diagnostic,
                completed_at,
                duration_ms,
                outcome,
            } => match outcome {
                HostManagedModelCallDiagnosticOutcome::Succeeded { usage } => (
                    diagnostic,
                    Some(completed_at),
                    Some(duration_ms),
                    InspectorModelCallStatus::Succeeded,
                    usage,
                    None,
                    DiagnosticActivityKind::ModelCallCompleted,
                    Some("Model call completed".to_string()),
                ),
                HostManagedModelCallDiagnosticOutcome::Failed {
                    usage,
                    failure_summary,
                } => (
                    diagnostic,
                    Some(completed_at),
                    Some(duration_ms),
                    InspectorModelCallStatus::Failed,
                    usage,
                    Some(failure_summary),
                    DiagnosticActivityKind::ModelCallFailed,
                    None,
                ),
            },
        };
        let user_id = diagnostic
            .context
            .actor
            .as_ref()
            .map(|actor| actor.user_id.clone())
            .or_else(|| diagnostic.context.scope.explicit_owner_user_id().cloned());
        let Some(user_id) = user_id else {
            tracing::debug!(
                run_id = %diagnostic.context.run_id,
                "model-call diagnostics skipped because the run has no user scope"
            );
            return;
        };
        let scope = DiagnosticScope::new(
            diagnostic.context.scope.tenant_id.clone(),
            user_id,
            diagnostic.context.thread_id.clone(),
            diagnostic.context.run_id,
        );
        let usage = usage.map(|usage| ModelTokenUsage {
            input_tokens: Some(u64::from(usage.input_tokens)),
            output_tokens: Some(u64::from(usage.output_tokens)),
            cache_read_input_tokens: Some(u64::from(usage.cache_read_input_tokens)),
            cache_creation_input_tokens: Some(u64::from(usage.cache_creation_input_tokens)),
        });
        let detector = LeakDetector::new();
        let failure_summary =
            failure_summary.map(|summary| diagnostic_prompt_text(&detector, &summary));
        let call_id = DiagnosticModelCallId::from_uuid(diagnostic.call_id);
        let model_call = ModelCallDiagnostic::new(
            call_id,
            diagnostic.iteration,
            diagnostic_prompt_text(&detector, &diagnostic.requested_model),
            diagnostic
                .effective_model
                .map(|model| diagnostic_prompt_text(&detector, &model)),
            diagnostic.started_at,
            completed_at,
            duration_ms,
            status,
            usage,
            failure_summary.clone(),
        );
        if let Err(error) =
            InMemoryDiagnosticStore::record_model_call(self, scope.clone(), model_call)
        {
            tracing::debug!(%error, "model-call diagnostics could not be retained");
        }
        let summary = failure_summary.or(default_summary);
        if let Err(error) = self.record_activity(
            scope,
            DiagnosticActivityEvent::new(
                Utc::now(),
                kind,
                Some(diagnostic.iteration),
                None,
                Some(call_id),
                summary,
            ),
        ) {
            tracing::debug!(%error, "model-call activity diagnostics could not be retained");
        }
    }
}

impl Default for InMemoryDiagnosticStore {
    fn default() -> Self {
        let limits = DiagnosticStoreLimits::default();
        Self {
            limits,
            state: RwLock::new(DiagnosticStoreState::default()),
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
    receiver: broadcast::Receiver<Arc<DiagnosticUpdateEnvelope>>,
}

impl DiagnosticSubscription {
    pub async fn recv(&mut self) -> Result<Arc<DiagnosticUpdateEnvelope>, DiagnosticStoreError> {
        match self.receiver.recv().await {
            Ok(update) => Ok(update),
            Err(broadcast::error::RecvError::Lagged(count)) => {
                Err(DiagnosticStoreError::SubscriberLagged(count))
            }
            Err(broadcast::error::RecvError::Closed) => {
                Err(DiagnosticStoreError::SubscriptionClosed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Barrier;

    use chrono::Utc;
    use ironclaw_host_api::{
        ids::{CapabilityId, TenantId, ThreadId, UserId},
        turn::{RunProfileId, RunProfileVersion, TurnActor, TurnId, TurnRunId, TurnScope},
    };
    use ironclaw_loop_contracts::{
        LoopModelUsage, LoopRunContext, ModelProfileId, ResolvedRunProfile, SkillName,
    };
    use ironclaw_loop_host::ProviderModelId;
    use ironclaw_product_contracts::inspector::{
        BoundedDiagnosticText, DIAGNOSTIC_LABEL_MAX_BYTES, DIAGNOSTIC_SUMMARY_MAX_BYTES,
        DiagnosticActivityEvent, DiagnosticActivityKind, DiagnosticModelCallId,
        DiagnosticModelCount, DiagnosticScope, InspectorModelCallStatus, MAX_ACTIVE_SKILLS,
        MAX_MODELS_IN_STATS, MAX_PROMPT_COMPONENTS, ModelCallDiagnostic,
        PROMPT_COMPONENT_CONTENT_MAX_BYTES, PROMPT_COMPONENT_TOTAL_MAX_BYTES,
        PromptComponentDiagnostic, PromptComponentKind, PromptDiagnostic, TOOL_ARGUMENTS_MAX_BYTES,
        TOOL_RESULT_MAX_BYTES, ToolExecutionDiagnostic, ToolExecutionStatus,
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
            max_live_update_scopes: 2,
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
    fn zero_validation_covers_every_limit_field_with_its_stable_name() {
        let valid = tiny_limits();
        let cases = [
            (
                "max_sessions",
                DiagnosticStoreLimits {
                    max_sessions: 0,
                    ..valid
                },
            ),
            (
                "max_runs_per_session",
                DiagnosticStoreLimits {
                    max_runs_per_session: 0,
                    ..valid
                },
            ),
            (
                "max_model_calls_per_run",
                DiagnosticStoreLimits {
                    max_model_calls_per_run: 0,
                    ..valid
                },
            ),
            (
                "max_tool_executions_per_run",
                DiagnosticStoreLimits {
                    max_tool_executions_per_run: 0,
                    ..valid
                },
            ),
            (
                "max_activity_entries_per_run",
                DiagnosticStoreLimits {
                    max_activity_entries_per_run: 0,
                    ..valid
                },
            ),
            (
                "max_updates_per_run",
                DiagnosticStoreLimits {
                    max_updates_per_run: 0,
                    ..valid
                },
            ),
            (
                "max_live_update_scopes",
                DiagnosticStoreLimits {
                    max_live_update_scopes: 0,
                    ..valid
                },
            ),
            (
                "live_update_capacity",
                DiagnosticStoreLimits {
                    live_update_capacity: 0,
                    ..valid
                },
            ),
        ];

        for (name, limits) in cases {
            assert_eq!(
                limits.validate(),
                Err(DiagnosticStoreError::InvalidLimit(name))
            );
        }
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
    fn default_store_limits_pass_the_validated_constructor_contract() {
        let limits = DiagnosticStoreLimits::default();

        assert_eq!(limits.validate(), Ok(limits));
        assert_eq!(InMemoryDiagnosticStore::default().limits, limits);
    }

    #[test]
    fn inspector_state_allows_concurrent_readers() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let first_reader = store.state.read().expect("first reader");
        let second_reader = store
            .state
            .try_read()
            .expect("read-only inspector queries must not exclude each other");

        drop((first_reader, second_reader));
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
    fn record_prompt_reapplies_limits_to_a_literal_dto() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let oversized_label = "l".repeat(DIAGNOSTIC_LABEL_MAX_BYTES + 1);
        let oversized_content = "x".repeat(PROMPT_COMPONENT_CONTENT_MAX_BYTES + 1);
        let prompt = PromptDiagnostic {
            captured_at: Utc::now(),
            components: (0..=MAX_PROMPT_COMPONENTS)
                .map(|index| PromptComponentDiagnostic {
                    kind: PromptComponentKind::Instruction,
                    label: BoundedDiagnosticText::reconstructed_prompt(oversized_label.clone()),
                    content: BoundedDiagnosticText::reconstructed_prompt(if index < 5 {
                        oversized_content.clone()
                    } else {
                        "x".to_string()
                    }),
                    estimated_tokens: None,
                })
                .collect(),
            components_truncated: false,
            reconstructed_prompt: BoundedDiagnosticText::reconstructed_prompt("prompt"),
            total_estimated_tokens: None,
            message_count: 1,
            identity_message_count: 0,
            instruction_snippet_count: 1,
            active_skills: (0..=MAX_ACTIVE_SKILLS)
                .map(|_| BoundedDiagnosticText::reconstructed_prompt(oversized_label.clone()))
                .collect(),
            active_skills_truncated: false,
            capability_count: 0,
            requested_model: Some(BoundedDiagnosticText::reconstructed_prompt(
                oversized_label.clone(),
            )),
            effective_model: Some(BoundedDiagnosticText::reconstructed_prompt(oversized_label)),
            context_limit: None,
        };

        store.record_prompt(scope.clone(), prompt).expect("prompt");

        let prompt = store
            .snapshot(&scope)
            .expect("snapshot")
            .expect("run")
            .prompt
            .expect("prompt");
        assert!(prompt.components_truncated);
        assert!(prompt.components.len() <= MAX_PROMPT_COMPONENTS);
        assert!(
            prompt
                .components
                .iter()
                .all(|component| component.label.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES)
        );
        assert!(prompt.components.iter().all(
            |component| component.content.content().len() <= PROMPT_COMPONENT_CONTENT_MAX_BYTES
        ));
        assert!(
            prompt
                .components
                .iter()
                .map(|component| component.content.content().len())
                .sum::<usize>()
                <= PROMPT_COMPONENT_TOTAL_MAX_BYTES
        );
        assert!(prompt.active_skills_truncated);
        assert_eq!(prompt.active_skills.len(), MAX_ACTIVE_SKILLS);
        assert!(
            prompt
                .active_skills
                .iter()
                .all(|skill| skill.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES)
        );
        assert!(
            prompt
                .requested_model
                .as_ref()
                .is_some_and(|model| model.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES)
        );
        assert!(
            prompt
                .effective_model
                .as_ref()
                .is_some_and(|model| model.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES)
        );
    }

    #[test]
    fn record_boundaries_reapply_limits_to_literal_dtos() {
        let mut limits = tiny_limits();
        limits.max_updates_per_run = 8;
        let store = InMemoryDiagnosticStore::new(limits).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let model_call_id = DiagnosticModelCallId::new();
        let activity_id = ironclaw_host_api::turn::CapabilityActivityId::new();
        let oversized_label =
            BoundedDiagnosticText::reconstructed_prompt("l".repeat(DIAGNOSTIC_LABEL_MAX_BYTES + 1));
        let oversized_summary = BoundedDiagnosticText::reconstructed_prompt(
            "s".repeat(DIAGNOSTIC_SUMMARY_MAX_BYTES + 1),
        );
        let oversized_tool_text =
            BoundedDiagnosticText::reconstructed_prompt("x".repeat(TOOL_ARGUMENTS_MAX_BYTES + 1));

        store
            .record_model_call(
                scope.clone(),
                ModelCallDiagnostic {
                    call_id: model_call_id,
                    iteration: 1,
                    requested_model: oversized_label.clone(),
                    effective_model: Some(oversized_label.clone()),
                    started_at: Utc::now(),
                    completed_at: None,
                    duration_ms: None,
                    status: InspectorModelCallStatus::Failed,
                    usage: None,
                    failure_summary: Some(oversized_summary.clone()),
                },
            )
            .expect("model call");
        store
            .record_tool_execution(
                scope.clone(),
                ToolExecutionDiagnostic {
                    activity_id,
                    model_call_id: Some(model_call_id),
                    capability_name: oversized_label.clone(),
                    arguments: Some(oversized_tool_text.clone()),
                    result: Some(oversized_tool_text.clone()),
                    status: ToolExecutionStatus::Failed,
                    duration_ms: None,
                    output_bytes: Some(1),
                    failure_category: Some(oversized_label.clone()),
                    failure_summary: Some(oversized_summary.clone()),
                },
            )
            .expect("tool execution");
        store
            .record_activity(
                scope.clone(),
                DiagnosticActivityEvent {
                    occurred_at: Utc::now(),
                    kind: DiagnosticActivityKind::ToolFailed,
                    iteration: Some(1),
                    activity_id: Some(activity_id),
                    model_call_id: Some(model_call_id),
                    summary: Some(oversized_summary),
                },
            )
            .expect("activity");

        store
            .record_stats(
                scope.clone(),
                SessionDiagnosticStats {
                    calls_per_model: (0..=MAX_MODELS_IN_STATS)
                        .map(|_| DiagnosticModelCount {
                            model: oversized_label.clone(),
                            calls: 1,
                        })
                        .collect(),
                    ..SessionDiagnosticStats::default()
                },
            )
            .expect("stats");

        let snapshot = store.snapshot(&scope).expect("snapshot").expect("run");
        let model_call = snapshot.model_calls.first().expect("model call");
        assert!(model_call.requested_model.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES);
        assert!(
            model_call
                .effective_model
                .as_ref()
                .is_some_and(|model| model.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES)
        );
        assert!(
            model_call
                .failure_summary
                .as_ref()
                .is_some_and(|summary| summary.content().len() <= DIAGNOSTIC_SUMMARY_MAX_BYTES)
        );

        let tool = snapshot.tool_executions.first().expect("tool execution");
        assert!(tool.capability_name.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES);
        assert!(
            tool.arguments
                .as_ref()
                .is_some_and(|arguments| arguments.content().len() <= TOOL_ARGUMENTS_MAX_BYTES)
        );
        assert!(
            tool.result
                .as_ref()
                .is_some_and(|result| result.content().len() <= TOOL_RESULT_MAX_BYTES)
        );
        assert_eq!(
            tool.output_bytes,
            tool.result
                .as_ref()
                .map(BoundedDiagnosticText::original_bytes)
        );
        assert!(
            tool.failure_category
                .as_ref()
                .is_some_and(|category| category.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES)
        );
        assert!(
            tool.failure_summary
                .as_ref()
                .is_some_and(|summary| summary.content().len() <= DIAGNOSTIC_SUMMARY_MAX_BYTES)
        );

        let event = &snapshot.activity.first().expect("activity").event;
        assert!(
            event
                .summary
                .as_ref()
                .is_some_and(|summary| summary.content().len() <= DIAGNOSTIC_SUMMARY_MAX_BYTES)
        );
        assert!(snapshot.stats.calls_per_model_truncated);
        assert_eq!(snapshot.stats.calls_per_model.len(), MAX_MODELS_IN_STATS);
        assert!(
            snapshot
                .stats
                .calls_per_model
                .iter()
                .all(|count| count.model.content().len() <= DIAGNOSTIC_LABEL_MAX_BYTES)
        );
    }

    #[test]
    fn metrics_cover_provider_failover_replacements_missing_usage_and_retention() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let first_id = DiagnosticModelCallId::new();
        let first_started = ModelCallDiagnostic::new(
            first_id,
            2,
            "requested-a",
            Some("model-a".to_string()),
            Utc::now(),
            None,
            None,
            InspectorModelCallStatus::Started,
            None,
            None,
        );
        store
            .record_model_call(scope.clone(), first_started)
            .expect("record start");
        let first_completed = ModelCallDiagnostic::new(
            first_id,
            2,
            "requested-a",
            Some("model-a".to_string()),
            Utc::now(),
            Some(Utc::now()),
            Some(100),
            InspectorModelCallStatus::Succeeded,
            Some(ModelTokenUsage {
                input_tokens: Some(15),
                output_tokens: Some(5),
                cache_read_input_tokens: Some(3),
                cache_creation_input_tokens: Some(2),
            }),
            None,
        );
        store
            .record_model_call(scope.clone(), first_completed)
            .expect("complete first");
        store
            .record_model_call(
                scope.clone(),
                ModelCallDiagnostic::new(
                    DiagnosticModelCallId::new(),
                    3,
                    "requested-a",
                    Some("model-b".to_string()),
                    Utc::now(),
                    Some(Utc::now()),
                    Some(300),
                    InspectorModelCallStatus::Failed,
                    Some(ModelTokenUsage {
                        input_tokens: Some(7),
                        output_tokens: Some(1),
                        cache_read_input_tokens: Some(0),
                        cache_creation_input_tokens: Some(0),
                    }),
                    Some("provider failed".to_string()),
                ),
            )
            .expect("record failed call with usage");
        store
            .record_model_call(
                scope.clone(),
                ModelCallDiagnostic::new(
                    DiagnosticModelCallId::new(),
                    4,
                    "requested-a",
                    Some("model-a".to_string()),
                    Utc::now(),
                    Some(Utc::now()),
                    None,
                    InspectorModelCallStatus::Succeeded,
                    None,
                    None,
                ),
            )
            .expect("record call without usage");

        let tool_id = ironclaw_host_api::turn::CapabilityActivityId::new();
        for status in [ToolExecutionStatus::Started, ToolExecutionStatus::Succeeded] {
            store
                .record_tool_execution(
                    scope.clone(),
                    ToolExecutionDiagnostic::new(
                        tool_id,
                        None,
                        "filesystem.read",
                        None,
                        None,
                        status,
                        None,
                        None,
                        None,
                        None,
                    ),
                )
                .expect("record tool transition");
        }
        store
            .record_tool_execution(
                scope.clone(),
                ToolExecutionDiagnostic::new(
                    ironclaw_host_api::turn::CapabilityActivityId::new(),
                    None,
                    "network.fetch",
                    None,
                    None,
                    ToolExecutionStatus::Failed,
                    None,
                    None,
                    None,
                    None,
                ),
            )
            .expect("record failed tool");

        let snapshot = store.snapshot(&scope).expect("snapshot").expect("present");
        assert_eq!(snapshot.model_calls.len(), 2, "run detail stays bounded");
        assert_eq!(snapshot.model_calls[0].iteration, 3);
        assert_eq!(snapshot.model_calls[1].iteration, 4);
        assert_eq!(snapshot.stats.total_model_calls, 3);
        assert_eq!(snapshot.stats.input_tokens.known_total, 22);
        assert_eq!(snapshot.stats.input_tokens.unavailable_samples, 1);
        assert_eq!(snapshot.stats.output_tokens.known_total, 6);
        assert_eq!(snapshot.stats.total_latency_ms.known_total, 400);
        assert_eq!(snapshot.stats.total_latency_ms.unavailable_samples, 1);
        assert_eq!(snapshot.stats.total_tool_calls, 2);
        assert_eq!(snapshot.stats.successful_tool_calls, 1);
        assert_eq!(snapshot.stats.failed_tool_calls, 1);
        assert_eq!(snapshot.stats.calls_per_model.len(), 2);
        assert_eq!(snapshot.stats.calls_per_model[0].calls, 2);
        assert_eq!(snapshot.stats.calls_per_model[1].calls, 1);
    }

    #[test]
    fn terminal_updates_do_not_recount_records_evicted_from_display_retention() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let first_model_id = DiagnosticModelCallId::new();

        for (call_id, iteration, status) in [
            (first_model_id, 1, InspectorModelCallStatus::Started),
            (
                DiagnosticModelCallId::new(),
                2,
                InspectorModelCallStatus::Succeeded,
            ),
            (
                DiagnosticModelCallId::new(),
                3,
                InspectorModelCallStatus::Failed,
            ),
        ] {
            store
                .record_model_call(
                    scope.clone(),
                    ModelCallDiagnostic::new(
                        call_id,
                        iteration,
                        "requested",
                        Some("provider-model".to_string()),
                        Utc::now(),
                        None,
                        None,
                        status,
                        None,
                        None,
                    ),
                )
                .expect("record model call");
        }

        let first_tool_id = ironclaw_host_api::turn::CapabilityActivityId::new();
        for (activity_id, status) in [
            (first_tool_id, ToolExecutionStatus::Started),
            (
                ironclaw_host_api::turn::CapabilityActivityId::new(),
                ToolExecutionStatus::Succeeded,
            ),
            (
                ironclaw_host_api::turn::CapabilityActivityId::new(),
                ToolExecutionStatus::Failed,
            ),
        ] {
            store
                .record_tool_execution(
                    scope.clone(),
                    ToolExecutionDiagnostic::new(
                        activity_id,
                        None,
                        "filesystem.read",
                        None,
                        None,
                        status,
                        None,
                        None,
                        None,
                        None,
                    ),
                )
                .expect("record tool execution");
        }

        let retained = store.snapshot(&scope).expect("snapshot").expect("run");
        assert_eq!(retained.model_calls.len(), 2);
        assert!(
            retained
                .model_calls
                .iter()
                .all(|call| call.call_id != first_model_id)
        );
        assert_eq!(retained.tool_executions.len(), 2);
        assert!(
            retained
                .tool_executions
                .iter()
                .all(|tool| tool.activity_id != first_tool_id)
        );

        store
            .record_model_call(
                scope.clone(),
                ModelCallDiagnostic::new(
                    first_model_id,
                    1,
                    "requested",
                    Some("provider-model".to_string()),
                    Utc::now(),
                    Some(Utc::now()),
                    Some(50),
                    InspectorModelCallStatus::Succeeded,
                    Some(ModelTokenUsage {
                        input_tokens: Some(11),
                        output_tokens: Some(4),
                        cache_read_input_tokens: Some(0),
                        cache_creation_input_tokens: Some(0),
                    }),
                    None,
                ),
            )
            .expect("complete evicted model call");
        store
            .record_tool_execution(
                scope.clone(),
                ToolExecutionDiagnostic::new(
                    first_tool_id,
                    None,
                    "filesystem.read",
                    None,
                    None,
                    ToolExecutionStatus::Succeeded,
                    None,
                    None,
                    None,
                    None,
                ),
            )
            .expect("complete evicted tool execution");

        let snapshot = store.snapshot(&scope).expect("snapshot").expect("run");
        assert_eq!(snapshot.stats.total_model_calls, 3);
        assert_eq!(snapshot.stats.input_tokens.known_total, 11);
        assert_eq!(snapshot.stats.input_tokens.unavailable_samples, 2);
        assert_eq!(snapshot.stats.total_tool_calls, 3);
        assert_eq!(snapshot.stats.successful_tool_calls, 2);
        assert_eq!(snapshot.stats.failed_tool_calls, 1);
    }

    #[test]
    fn model_breakdown_truncation_remains_latched_after_a_bucket_is_removed() {
        let store = InMemoryDiagnosticStore::default();
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let first_call_id = DiagnosticModelCallId::new();
        for index in 0..=MAX_MODELS_IN_STATS {
            let call_id = if index == 0 {
                first_call_id
            } else {
                DiagnosticModelCallId::new()
            };
            store
                .record_model_call(
                    scope.clone(),
                    ModelCallDiagnostic::new(
                        call_id,
                        u32::try_from(index).expect("model index"),
                        "requested",
                        Some(format!("model-{index}")),
                        Utc::now(),
                        Some(Utc::now()),
                        Some(1),
                        InspectorModelCallStatus::Succeeded,
                        None,
                        None,
                    ),
                )
                .expect("record distinct model");
        }
        store
            .record_model_call(
                scope.clone(),
                ModelCallDiagnostic::new(
                    first_call_id,
                    0,
                    "requested",
                    Some("model-1".to_string()),
                    Utc::now(),
                    Some(Utc::now()),
                    Some(1),
                    InspectorModelCallStatus::Succeeded,
                    None,
                    None,
                ),
            )
            .expect("replace first model contribution");

        let snapshot = store.snapshot(&scope).expect("snapshot").expect("run");
        assert!(snapshot.stats.calls_per_model_truncated);
        assert_eq!(
            snapshot.stats.total_model_calls,
            u64::try_from(MAX_MODELS_IN_STATS + 1).expect("model count")
        );
        assert_eq!(
            snapshot.stats.calls_per_model.len(),
            MAX_MODELS_IN_STATS - 1
        );
    }

    #[test]
    fn metric_aggregation_saturates_instead_of_wrapping() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        store
            .record_stats(
                scope.clone(),
                SessionDiagnosticStats {
                    total_model_calls: u64::MAX,
                    input_tokens: DiagnosticMetricTotal {
                        known_total: u64::MAX,
                        unavailable_samples: u64::MAX,
                    },
                    ..SessionDiagnosticStats::default()
                },
            )
            .expect("seed saturated stats");
        store
            .record_model_call(
                scope.clone(),
                ModelCallDiagnostic::new(
                    DiagnosticModelCallId::new(),
                    0,
                    "requested",
                    Some("model".to_string()),
                    Utc::now(),
                    Some(Utc::now()),
                    Some(1),
                    InspectorModelCallStatus::Succeeded,
                    None,
                    None,
                ),
            )
            .expect("aggregate at saturation");

        let stats = store
            .snapshot(&scope)
            .expect("snapshot")
            .expect("present")
            .stats;
        assert_eq!(stats.total_model_calls, u64::MAX);
        assert_eq!(stats.input_tokens.known_total, u64::MAX);
        assert_eq!(stats.input_tokens.unavailable_samples, u64::MAX);
    }

    #[test]
    fn host_prompt_capture_is_scoped_redacted_validated_and_bounded() {
        let store = InMemoryDiagnosticStore::default();
        let tenant_id = TenantId::new("tenant").expect("tenant");
        let user_id = UserId::new("user").expect("user");
        let thread_id = ThreadId::new("thread").expect("thread");
        let run_id = TurnRunId::new();
        let profile = ResolvedRunProfile::legacy_compatibility(
            RunProfileId::interactive_default(),
            RunProfileVersion::new(1),
            true,
        );
        let context = LoopRunContext::new(
            TurnScope::new(tenant_id.clone(), None, None, thread_id.clone()),
            TurnId::new(),
            run_id,
            profile,
        )
        .with_actor(TurnActor::new(user_id.clone()));
        let secret = format!("sk-{}", "diagnostictestvalue0123456789");
        let prompt_capture = HostManagedPromptDiagnosticCapture {
            context: context.clone(),
            messages: vec![
                HostManagedPromptDiagnosticMessage {
                    role: HostManagedModelMessageRole::System,
                    content_ref: ironclaw_host_api::turn::LoopMessageRef::new(
                        "msg:identity.system",
                    )
                    .expect("ref"),
                    content: format!(
                        "identity sk-diagnostic\u{1b}testvalue0123456789 {}",
                        "x".repeat(70 * 1024)
                    ),
                },
                HostManagedPromptDiagnosticMessage {
                    role: HostManagedModelMessageRole::System,
                    content_ref: ironclaw_host_api::turn::LoopMessageRef::new(
                        "msg:instruction.system.0.deadbeef",
                    )
                    .expect("ref"),
                    content: "Follow the workspace instructions.".to_string(),
                },
                HostManagedPromptDiagnosticMessage {
                    role: HostManagedModelMessageRole::System,
                    content_ref: ironclaw_host_api::turn::LoopMessageRef::new(
                        "msg:snippet.skill.workspace-search.0.deadbeef",
                    )
                    .expect("ref"),
                    content: "Use workspace search when needed.".to_string(),
                },
                HostManagedPromptDiagnosticMessage {
                    role: HostManagedModelMessageRole::User,
                    content_ref: ironclaw_host_api::turn::LoopMessageRef::new("msg:thread.user")
                        .expect("ref"),
                    content: "hello".to_string(),
                },
            ],
            identity_message_count: 1,
            instruction_snippet_count: 2,
            active_skills: vec![
                SkillName::new("workspace-search").expect("skill name"),
                SkillName::new(secret.clone()).expect("secret-like skill name"),
            ],
            capability_ids: vec![CapabilityId::new("filesystem.read").expect("capability")],
            requested_model: Some(
                ModelProfileId::new("diagnostic-profile").expect("model profile"),
            ),
            effective_model: Some(
                ProviderModelId::new("provider/diagnostic-model").expect("provider model"),
            ),
            context_limit: 128_000,
        };
        HostManagedPromptDiagnosticSink::record_prompt(&store, prompt_capture.clone());
        HostManagedPromptDiagnosticSink::record_prompt(&store, prompt_capture);
        HostManagedPromptDiagnosticSink::record_model_call(
            &store,
            HostManagedModelCallDiagnosticCapture::Completed {
                diagnostic: HostManagedModelCallDiagnostic {
                    call_id: uuid::Uuid::new_v4(),
                    context,
                    iteration: 4,
                    requested_model: "interactive_model".to_string(),
                    effective_model: None,
                    started_at: Utc::now(),
                },
                completed_at: Utc::now(),
                duration_ms: 42,
                outcome: HostManagedModelCallDiagnosticOutcome::Succeeded {
                    usage: Some(LoopModelUsage {
                        input_tokens: 12,
                        output_tokens: 4,
                        cache_read_input_tokens: 2,
                        cache_creation_input_tokens: 1,
                    }),
                },
            },
        );

        let diagnostic_scope = DiagnosticScope::new(tenant_id, user_id, thread_id, run_id);
        let prompt = store
            .prompt(&diagnostic_scope)
            .expect("prompt read")
            .expect("prompt captured");
        assert_eq!(prompt.message_count, 4);
        assert_eq!(prompt.identity_message_count, 1);
        assert_eq!(prompt.instruction_snippet_count, 2);
        assert_eq!(prompt.capability_count, 1);
        assert_eq!(prompt.active_skills[0].content(), "workspace-search");
        assert!(prompt.active_skills[1].content().contains("[REDACTED]"));
        assert!(!prompt.active_skills[1].content().contains(&secret));
        assert_eq!(
            prompt.requested_model.as_ref().map(|model| model.content()),
            Some("diagnostic-profile")
        );
        assert_eq!(
            prompt.effective_model.as_ref().map(|model| model.content()),
            Some("provider/diagnostic-model")
        );
        assert!(prompt.components[0].content.truncated());
        assert!(!prompt.components[0].content.content().contains(&secret));
        assert!(!prompt.components[0].content.content().contains('\u{1b}'));
        assert!(
            prompt.components[0]
                .content
                .content()
                .contains("[REDACTED]")
        );
        assert_eq!(prompt.components[0].kind, PromptComponentKind::Identity);
        assert_eq!(prompt.components[1].kind, PromptComponentKind::Instruction);
        assert_eq!(prompt.components[2].kind, PromptComponentKind::Skill);
        assert_eq!(prompt.components[3].kind, PromptComponentKind::Conversation);
        assert_eq!(
            prompt.components.last().map(|component| component.kind),
            Some(PromptComponentKind::Capability)
        );
        let snapshot = store
            .snapshot(&diagnostic_scope)
            .expect("snapshot")
            .expect("present");
        assert_eq!(snapshot.model_calls.len(), 1);
        assert_eq!(snapshot.model_calls[0].iteration, 4);
        assert_eq!(snapshot.model_calls[0].effective_model, None);
        assert_eq!(snapshot.stats.total_model_calls, 1);
        assert_eq!(snapshot.stats.input_tokens.known_total, 12);
        assert_eq!(
            snapshot
                .activity
                .iter()
                .filter(|entry| entry.event.kind == DiagnosticActivityKind::TurnStarted)
                .count(),
            1
        );
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
    fn fresh_reader_requires_rebase_after_the_stream_prefix_is_evicted() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        for index in 1..=2 {
            store
                .record_activity(scope.clone(), activity(&format!("entry-{index}")))
                .expect("record");
        }
        let complete = store.updates_after(&scope, None).expect("complete updates");
        assert!(!complete.rebase_required);

        store
            .record_activity(scope.clone(), activity("entry-3"))
            .expect("record");

        let batch = store.updates_after(&scope, None).expect("updates");

        assert!(batch.rebase_required);
        assert_eq!(
            batch
                .updates
                .iter()
                .map(|update| update.sequence.as_u64())
                .collect::<Vec<_>>(),
            vec![2, 3]
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

    #[test]
    fn cursor_for_an_evicted_run_requires_rebase() {
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

        let batch = store
            .updates_after(&first, Some(original))
            .expect("evicted run batch");

        assert!(batch.rebase_required);
        assert!(batch.updates.is_empty());
        assert_eq!(batch.retention_floor, None);
        assert_eq!(batch.latest_cursor, None);
    }

    #[test]
    fn future_cursor_in_the_current_stream_requires_rebase() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let latest = store
            .record_activity(scope.clone(), activity("first"))
            .expect("record");
        let future = DiagnosticCursor::new(
            latest.stream_id,
            DiagnosticSequence::new(latest.sequence.as_u64() + 1),
        );

        let batch = store
            .updates_after(&scope, Some(future))
            .expect("future cursor batch");

        assert!(batch.rebase_required);
        assert!(batch.updates.is_empty());
        assert_eq!(batch.latest_cursor, Some(latest));
    }

    #[tokio::test]
    async fn unrelated_scope_saturation_does_not_lag_scoped_subscription() {
        let mut limits = tiny_limits();
        limits.live_update_capacity = 2;
        let store = InMemoryDiagnosticStore::new(limits).expect("store");
        let allowed = scope("tenant", "user", "thread-a", TurnRunId::new());
        let other = scope("tenant", "user", "thread-b", TurnRunId::new());
        let mut subscription = store
            .subscribe(allowed.clone())
            .expect("scoped subscription");
        let mut other_subscription = store
            .subscribe(other.clone())
            .expect("other scoped subscription");
        for index in 0..=limits.live_update_capacity {
            store
                .record_activity(other.clone(), activity(&format!("other-{index}")))
                .expect("other");
        }
        store
            .record_activity(allowed.clone(), activity("allowed"))
            .expect("allowed");

        assert!(matches!(
            other_subscription.recv().await,
            Err(DiagnosticStoreError::SubscriberLagged(_))
        ));
        let update = subscription.recv().await.expect("matching update");
        assert_eq!(update.scope, allowed);
        assert_eq!(update.sequence, DiagnosticSequence::new(1));
    }

    #[tokio::test]
    async fn live_scope_limit_evicts_the_least_recently_subscribed_scope() {
        let mut limits = tiny_limits();
        limits.max_live_update_scopes = 2;
        let store = InMemoryDiagnosticStore::new(limits).expect("store");
        let first_scope = scope("tenant", "user", "thread-a", TurnRunId::new());
        let second_scope = scope("tenant", "user", "thread-b", TurnRunId::new());
        let third_scope = scope("tenant", "user", "thread-c", TurnRunId::new());
        let mut first = store
            .subscribe(first_scope.clone())
            .expect("first subscription");
        let mut second = store.subscribe(second_scope).expect("second subscription");
        let _refreshed_first = store
            .subscribe(first_scope.clone())
            .expect("refresh first subscription");
        let mut third = store
            .subscribe(third_scope.clone())
            .expect("third subscription");

        assert_eq!(
            second.recv().await,
            Err(DiagnosticStoreError::SubscriptionClosed)
        );
        store
            .record_activity(first_scope.clone(), activity("first"))
            .expect("record first");
        store
            .record_activity(third_scope.clone(), activity("third"))
            .expect("record third");

        assert_eq!(first.recv().await.expect("first update").scope, first_scope);
        assert_eq!(third.recv().await.expect("third update").scope, third_scope);
        assert_eq!(
            store.state.read().expect("state").live_updates.len(),
            limits.max_live_update_scopes
        );
    }

    #[tokio::test]
    async fn concurrent_writers_deliver_live_updates_in_sequence_order() {
        const WRITER_COUNT: usize = 32;

        let mut limits = tiny_limits();
        limits.live_update_capacity = WRITER_COUNT;
        let store = Arc::new(InMemoryDiagnosticStore::new(limits).expect("store"));
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let mut subscription = store.subscribe(scope.clone()).expect("scoped subscription");
        let barrier = Arc::new(Barrier::new(WRITER_COUNT));

        std::thread::scope(|threads| {
            let handles = (0..WRITER_COUNT)
                .map(|index| {
                    let store = Arc::clone(&store);
                    let scope = scope.clone();
                    let barrier = Arc::clone(&barrier);
                    threads.spawn(move || {
                        barrier.wait();
                        store
                            .record_activity(scope, activity(&format!("writer-{index}")))
                            .expect("record")
                    })
                })
                .collect::<Vec<_>>();

            for handle in handles {
                handle.join().expect("writer thread");
            }
        });

        for expected in 1..=WRITER_COUNT as u64 {
            let update = subscription.recv().await.expect("ordered update");
            assert_eq!(update.sequence, DiagnosticSequence::new(expected));
        }
    }

    #[tokio::test]
    async fn subscription_preserves_sequence_for_its_scope() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let allowed = scope("tenant", "user", "thread-a", TurnRunId::new());
        let mut subscription = store
            .subscribe(allowed.clone())
            .expect("scoped subscription");
        store
            .record_activity(allowed.clone(), activity("allowed"))
            .expect("allowed");
        let update = subscription.recv().await.expect("matching update");
        assert_eq!(update.scope, allowed);
        assert_eq!(update.sequence, DiagnosticSequence::new(1));
    }

    #[tokio::test]
    async fn stats_are_visible_when_the_update_is_delivered() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let scope = scope("tenant", "user", "thread", TurnRunId::new());
        let mut subscription = store.subscribe(scope.clone()).expect("scoped subscription");
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
    fn stats_remain_scoped_to_the_run_that_published_them() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let first = scope("tenant", "user", "thread", TurnRunId::new());
        let second = scope("tenant", "user", "thread", TurnRunId::new());
        store
            .record_stats(
                first.clone(),
                SessionDiagnosticStats {
                    total_model_calls: 3,
                    ..SessionDiagnosticStats::default()
                },
            )
            .expect("first stats");
        store
            .record_stats(
                second.clone(),
                SessionDiagnosticStats {
                    total_model_calls: 8,
                    ..SessionDiagnosticStats::default()
                },
            )
            .expect("second stats");

        assert_eq!(
            store
                .snapshot(&first)
                .expect("first snapshot")
                .expect("first run")
                .stats
                .total_model_calls,
            3
        );
        assert_eq!(
            store
                .snapshot(&second)
                .expect("second snapshot")
                .expect("second run")
                .stats
                .total_model_calls,
            8
        );
    }

    #[test]
    fn poisoned_state_returns_a_redacted_error() {
        let store = InMemoryDiagnosticStore::new(tiny_limits()).expect("store");
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = store.state.write().expect("lock before poison");
            panic!("poison store lock");
        }));
        let error = store
            .snapshot(&scope("tenant", "user", "thread", TurnRunId::new()))
            .expect_err("poison must fail closed");
        assert_eq!(error, DiagnosticStoreError::StateUnavailable);
        assert!(!error.to_string().contains("poison"));
    }
}
