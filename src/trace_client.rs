//! Product-layer Trace Commons client facade.
//!
//! Reborn keeps product/userland surfaces away from raw kernel substrates. This
//! facade gives agent/web/CLI callers a narrow client-side Trace Commons host:
//! local capture, local queueing, remote upload/status sync, and local credit
//! notice state. Hosted corpus storage and reviewer/admin control-plane state
//! live in the TraceDAO server repo.

use std::path::PathBuf;

use crate::history::ConversationMessage;
use crate::llm::recording::TraceFile;
use crate::trace_contribution::{
    self as trace, LocalTraceSubmissionRecord, OutcomeMetadata, RawTraceCaptureTurn,
    TraceContributionEnvelope, TraceFailureMode, TraceQueueFlushReport, TraceQueueWorkerReport,
    TraceRedactor as _,
};
use anyhow::Context;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceClientScope(String);

impl TraceClientScope {
    pub fn user(user_id: impl Into<String>) -> Self {
        Self(user_id.into())
    }

    pub fn raw(scope: impl Into<String>) -> Self {
        Self(scope.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct TraceClientAutonomousCaptureRequest<'a> {
    pub scope: TraceClientScope,
    pub channel: trace::TraceChannel,
    pub messages: &'a [ConversationMessage],
    pub policy: &'a trace::StandingTraceContributionPolicy,
    pub max_turns: usize,
}

#[derive(Debug)]
pub enum TraceClientAutonomousCaptureOutcome {
    Skipped,
    Held {
        submission_id: uuid::Uuid,
        reason: String,
    },
    Submit(Box<TraceContributionEnvelope>),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TraceClientHost;

impl TraceClientHost {
    pub async fn build_envelope_from_recorded_trace(
        &self,
        trace_file: &TraceFile,
        options: trace::RecordedTraceContributionOptions,
    ) -> anyhow::Result<TraceContributionEnvelope> {
        let raw = trace::RawTraceContribution::from_recorded_trace(trace_file, options);
        self.redact_and_score_raw_contribution(raw).await
    }

    pub async fn build_envelope_from_capture_turns(
        &self,
        turns: &[RawTraceCaptureTurn],
        options: trace::RecordedTraceContributionOptions,
        outcome: Option<OutcomeMetadata>,
    ) -> anyhow::Result<TraceContributionEnvelope> {
        let mut raw = trace::RawTraceContribution::from_capture_turns(turns, options);
        if let Some(outcome) = outcome
            && outcome.task_success != trace::TaskSuccess::Unknown
        {
            raw.outcome = outcome;
        }
        self.redact_and_score_raw_contribution(raw).await
    }

    async fn redact_and_score_raw_contribution(
        &self,
        raw: trace::RawTraceContribution,
    ) -> anyhow::Result<TraceContributionEnvelope> {
        let redactor = trace::DeterministicTraceRedactor::default();
        let mut envelope = redactor
            .redact_trace(raw)
            .await
            .context("failed to redact trace contribution")?;
        trace::apply_credit_estimate_to_envelope(&mut envelope);
        Ok(envelope)
    }

    pub async fn build_autonomous_envelope_from_messages(
        &self,
        request: TraceClientAutonomousCaptureRequest<'_>,
    ) -> anyhow::Result<Option<TraceContributionEnvelope>> {
        match self
            .prepare_autonomous_envelope_from_messages(request)
            .await?
        {
            TraceClientAutonomousCaptureOutcome::Submit(envelope) => Ok(Some(*envelope)),
            TraceClientAutonomousCaptureOutcome::Skipped
            | TraceClientAutonomousCaptureOutcome::Held { .. } => Ok(None),
        }
    }

    pub async fn prepare_autonomous_envelope_from_messages(
        &self,
        request: TraceClientAutonomousCaptureRequest<'_>,
    ) -> anyhow::Result<TraceClientAutonomousCaptureOutcome> {
        if trace::preflight_trace_contribution_policy(
            request.policy,
            trace::TraceContributionAcceptance::AutonomousSubmit,
        )
        .is_err()
        {
            return Ok(TraceClientAutonomousCaptureOutcome::Skipped);
        }

        let (mut turns, persisted_outcome) =
            capture_turns_from_conversation_messages_with_outcomes(request.messages);
        if turns.is_empty() {
            return Ok(TraceClientAutonomousCaptureOutcome::Skipped);
        }

        let max_turns = request.max_turns.max(1);
        if turns.len() > max_turns {
            turns = turns.split_off(turns.len() - max_turns);
        }

        let options = trace::RecordedTraceContributionOptions {
            include_message_text: request.policy.include_message_text,
            include_tool_payloads: request.policy.include_tool_payloads,
            consent_scopes: vec![request.policy.default_scope],
            channel: request.channel,
            engine_version: None,
            feature_flags: Default::default(),
            pseudonymous_contributor_id: Some(trace::local_pseudonymous_contributor_id(
                request.scope.as_str(),
            )),
            tenant_scope_ref: Some(trace::local_pseudonymous_tenant_scope_ref(
                request.scope.as_str(),
            )),
            credit_account_ref: None,
        };
        let envelope = self
            .build_envelope_from_capture_turns(&turns, options, Some(persisted_outcome))
            .await
            .context("failed to redact autonomous trace")?;

        match trace::trace_autonomous_eligibility(&envelope, request.policy) {
            trace::TraceQueueEligibility::Submit => Ok(
                TraceClientAutonomousCaptureOutcome::Submit(Box::new(envelope)),
            ),
            trace::TraceQueueEligibility::Hold { reason } => {
                Ok(TraceClientAutonomousCaptureOutcome::Held {
                    submission_id: envelope.submission_id,
                    reason,
                })
            }
        }
    }

    pub fn queue_envelope_for_scope(
        &self,
        scope: &TraceClientScope,
        envelope: &TraceContributionEnvelope,
    ) -> anyhow::Result<PathBuf> {
        trace::queue_trace_envelope_for_scope(Some(scope.as_str()), envelope)
    }

    pub async fn flush_scope_queue(
        &self,
        scope: &TraceClientScope,
        limit: usize,
    ) -> anyhow::Result<TraceQueueFlushReport> {
        trace::flush_trace_contribution_queue_for_scope(Some(scope.as_str()), limit).await
    }

    pub async fn flush_default_queue(&self, limit: usize) -> anyhow::Result<TraceQueueFlushReport> {
        trace::flush_trace_contribution_queue_for_scope(None, limit).await
    }

    pub async fn flush_queue_worker_tick<I, S>(
        &self,
        scopes: I,
        limit_per_scope: usize,
    ) -> anyhow::Result<TraceQueueWorkerReport>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        trace::flush_trace_contribution_queue_worker_tick(scopes, limit_per_scope).await
    }

    pub async fn sync_remote_records_for_scope(
        &self,
        scope: &TraceClientScope,
    ) -> anyhow::Result<usize> {
        trace::sync_remote_trace_submission_records_for_scope(Some(scope.as_str())).await
    }

    pub fn pending_credit_notice_outbox_items(
        &self,
        scope: &TraceClientScope,
    ) -> anyhow::Result<Vec<trace::TraceCreditNoticeOutboxItem>> {
        trace::pending_trace_credit_notice_outbox_items_for_scope(Some(scope.as_str()))
    }

    pub fn record_credit_notice_delivery_success(
        &self,
        scope: &TraceClientScope,
        fingerprint: &str,
        channel: &str,
    ) -> anyhow::Result<()> {
        trace::record_trace_credit_notice_delivery_success_for_scope(
            Some(scope.as_str()),
            fingerprint,
            channel,
        )
        .map(|_| ())
    }

    pub fn record_credit_notice_delivery_failure(
        &self,
        scope: &TraceClientScope,
        fingerprint: &str,
        channel: &str,
        error: &str,
    ) -> anyhow::Result<()> {
        trace::record_trace_credit_notice_delivery_failure_for_scope(
            Some(scope.as_str()),
            fingerprint,
            channel,
            error,
        )
        .map(|_| ())
    }

    pub fn read_local_records_for_scope(
        &self,
        scope: &TraceClientScope,
    ) -> anyhow::Result<Vec<LocalTraceSubmissionRecord>> {
        trace::read_local_trace_records_for_scope(Some(scope.as_str()))
    }

    pub fn read_local_records_for_default(
        &self,
    ) -> anyhow::Result<Vec<LocalTraceSubmissionRecord>> {
        trace::read_local_trace_records_for_scope(None)
    }
}

pub fn trace_channel_from_host_channel(channel: &str) -> trace::TraceChannel {
    match channel {
        "gateway" | "web" => trace::TraceChannel::Web,
        "cli" | "repl" | "tui" => trace::TraceChannel::Cli,
        "telegram" => trace::TraceChannel::Telegram,
        "slack" => trace::TraceChannel::Slack,
        "routine" | "heartbeat" => trace::TraceChannel::Routine,
        _ => trace::TraceChannel::Other,
    }
}

pub fn capture_turns_from_conversation_messages_with_outcomes(
    messages: &[ConversationMessage],
) -> (Vec<RawTraceCaptureTurn>, OutcomeMetadata) {
    let mut turns = trace::capture_turns_from_conversation_messages(messages);
    let mut aggregate = OutcomeMetadata::default();
    let mut turn_index = 0usize;
    let mut iter = messages.iter().peekable();

    while let Some(message) = iter.next() {
        match message.role.as_str() {
            "user" => {
                let current_turn_index = turn_index;

                if let Some(next) = iter.peek()
                    && next.role == "tool_calls"
                    && let Some(tool_message) = iter.next()
                    && let Ok(serde_json::Value::Object(obj)) =
                        serde_json::from_str::<serde_json::Value>(&tool_message.content)
                    && let Some(outcome) = obj.get("outcome")
                {
                    if let Some(state) = outcome.get("state").and_then(|v| v.as_str())
                        && let Some(turn) = turns.get_mut(current_turn_index)
                    {
                        turn.state = Some(state.to_string());
                    }
                    merge_trace_outcome_value(outcome, &mut aggregate);
                }

                if let Some(next) = iter.peek()
                    && next.role == "assistant"
                {
                    let _ = iter.next();
                }

                turn_index += 1;
            }
            "assistant" => {
                turn_index += 1;
            }
            _ => {}
        }
    }

    if aggregate.task_success == trace::TaskSuccess::Unknown {
        for turn in &turns {
            if matches!(turn.state.as_deref(), Some("Failed" | "failed")) {
                aggregate.task_success = trace::TaskSuccess::Failure;
                break;
            }
            if turn.response.is_some() {
                aggregate.task_success = trace::TaskSuccess::Success;
            }
        }
    }

    (turns, aggregate)
}

fn parse_trace_failure_mode(value: &str) -> TraceFailureMode {
    match value {
        "environment_or_auth_failure" => TraceFailureMode::EnvironmentOrAuthFailure,
        "unrecoverable_tool_failure" => TraceFailureMode::UnrecoverableToolFailure,
        "premature_termination" => TraceFailureMode::PrematureTermination,
        other => TraceFailureMode::Other(other.to_string()),
    }
}

fn merge_trace_outcome_value(outcome: &serde_json::Value, aggregate: &mut OutcomeMetadata) {
    match outcome.get("task_success").and_then(|v| v.as_str()) {
        Some("failure") => {
            aggregate.task_success = trace::TaskSuccess::Failure;
        }
        Some("success") if aggregate.task_success == trace::TaskSuccess::Unknown => {
            aggregate.task_success = trace::TaskSuccess::Success;
        }
        Some("partial") if !matches!(aggregate.task_success, trace::TaskSuccess::Failure) => {
            aggregate.task_success = trace::TaskSuccess::Partial;
        }
        _ => {}
    }

    if let Some(values) = outcome.get("error_taxonomy").and_then(|v| v.as_array()) {
        for value in values.iter().filter_map(|v| v.as_str()) {
            if !aggregate
                .error_taxonomy
                .iter()
                .any(|existing| existing == value)
            {
                aggregate.error_taxonomy.push(value.to_string());
            }
        }
    }

    if let Some(values) = outcome.get("failure_modes").and_then(|v| v.as_array()) {
        for value in values.iter().filter_map(|v| v.as_str()) {
            let mode = parse_trace_failure_mode(value);
            if !aggregate.failure_modes.contains(&mode) {
                aggregate.failure_modes.push(mode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use crate::history::ConversationMessage;
    use crate::trace_contribution::{
        ConsentScope, StandingTraceContributionPolicy, TaskSuccess, TraceChannel,
        local_pseudonymous_contributor_id, local_pseudonymous_tenant_scope_ref,
    };

    use super::{
        TraceClientAutonomousCaptureOutcome, TraceClientAutonomousCaptureRequest, TraceClientHost,
        TraceClientScope, capture_turns_from_conversation_messages_with_outcomes,
    };

    fn msg(role: &str, content: impl Into<String>) -> ConversationMessage {
        ConversationMessage {
            id: Uuid::new_v4(),
            role: role.to_string(),
            content: content.into(),
            created_at: Utc::now(),
        }
    }

    fn enabled_policy() -> StandingTraceContributionPolicy {
        StandingTraceContributionPolicy {
            enabled: true,
            ingestion_endpoint: Some("https://trace.example.test/v1/traces".to_string()),
            include_message_text: true,
            include_tool_payloads: true,
            require_manual_approval_when_pii_detected: false,
            min_submission_score: 0.0,
            default_scope: ConsentScope::DebuggingEvaluation,
            ..StandingTraceContributionPolicy::default()
        }
    }

    #[test]
    fn captures_turn_outcome_metadata_from_conversation_messages() {
        let outcome = serde_json::json!({
            "state": "Failed",
            "task_success": "failure",
            "error_taxonomy": ["runtime_error"],
            "failure_modes": ["environment_or_auth_failure"],
        });
        let tool_calls = serde_json::json!({
            "outcome": outcome,
            "calls": []
        });
        let messages = vec![
            msg("user", "run the task"),
            msg("tool_calls", tool_calls.to_string()),
            msg("assistant", "it failed"),
        ];

        let (turns, parsed_outcome) =
            capture_turns_from_conversation_messages_with_outcomes(&messages);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].state.as_deref(), Some("Failed"));
        assert_eq!(parsed_outcome.task_success, TaskSuccess::Failure);
        assert_eq!(parsed_outcome.error_taxonomy, vec!["runtime_error"]);
    }

    #[test]
    fn aligns_turn_outcome_metadata_after_assistant_only_messages() {
        let outcome = serde_json::json!({
            "state": "Failed",
            "task_success": "failure",
            "error_taxonomy": ["runtime_error"],
        });
        let tool_calls = serde_json::json!({
            "outcome": outcome,
            "calls": []
        });
        let messages = vec![
            msg("assistant", "ready when you are"),
            msg("user", "run the task"),
            msg("tool_calls", tool_calls.to_string()),
            msg("assistant", "it failed"),
        ];

        let (turns, parsed_outcome) =
            capture_turns_from_conversation_messages_with_outcomes(&messages);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "");
        assert_eq!(turns[0].state.as_deref(), Some("Completed"));
        assert_eq!(turns[1].user_input, "run the task");
        assert_eq!(turns[1].state.as_deref(), Some("Failed"));
        assert_eq!(parsed_outcome.task_success, TaskSuccess::Failure);
    }

    #[tokio::test]
    async fn autonomous_capture_uses_scoped_client_identity() {
        let policy = enabled_policy();
        let scope = TraceClientScope::user("user-123");
        let messages = vec![msg("user", "hello"), msg("assistant", "hi")];

        let envelope = TraceClientHost
            .build_autonomous_envelope_from_messages(TraceClientAutonomousCaptureRequest {
                scope: scope.clone(),
                channel: TraceChannel::Web,
                messages: &messages,
                policy: &policy,
                max_turns: 5,
            })
            .await
            .expect("capture succeeds")
            .expect("eligible envelope");

        assert_eq!(
            envelope.contributor.pseudonymous_contributor_id.as_deref(),
            Some(local_pseudonymous_contributor_id(scope.as_str()).as_str())
        );
        assert_eq!(
            envelope.contributor.tenant_scope_ref.as_deref(),
            Some(local_pseudonymous_tenant_scope_ref(scope.as_str()).as_str())
        );
        assert_eq!(
            envelope.consent.scopes,
            vec![ConsentScope::DebuggingEvaluation]
        );
    }

    #[tokio::test]
    async fn autonomous_capture_skips_when_policy_is_disabled() {
        let policy = StandingTraceContributionPolicy::default();
        let messages = vec![msg("user", "hello"), msg("assistant", "hi")];

        let envelope = TraceClientHost
            .build_autonomous_envelope_from_messages(TraceClientAutonomousCaptureRequest {
                scope: TraceClientScope::user("user-123"),
                channel: TraceChannel::Web,
                messages: &messages,
                policy: &policy,
                max_turns: 5,
            })
            .await
            .expect("policy skip is not an error");

        assert!(envelope.is_none());
    }

    #[tokio::test]
    async fn autonomous_capture_reports_policy_hold_without_queueing() {
        let policy = StandingTraceContributionPolicy {
            auto_submit_failed_traces: false,
            auto_submit_high_value_traces: false,
            ..enabled_policy()
        };
        let messages = vec![msg("user", "hello"), msg("assistant", "hi")];

        let outcome = TraceClientHost
            .prepare_autonomous_envelope_from_messages(TraceClientAutonomousCaptureRequest {
                scope: TraceClientScope::user("user-123"),
                channel: TraceChannel::Web,
                messages: &messages,
                policy: &policy,
                max_turns: 5,
            })
            .await
            .expect("capture evaluates");

        assert!(matches!(
            outcome,
            TraceClientAutonomousCaptureOutcome::Held { .. }
        ));
    }
}
