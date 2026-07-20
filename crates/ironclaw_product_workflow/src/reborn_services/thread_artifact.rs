//! Caller-owned, redacted evidence bundle for a complete thread.

use chrono::{DateTime, Utc};
use ironclaw_reborn_traces::contribution::DeterministicTraceRedactor;
use ironclaw_threads::ThreadMessageRecord;
use serde::{Deserialize, Serialize};

use super::{
    RebornServices, RebornServicesError, RebornViewDescriptor, RunArtifactLogs, RunArtifactMessage,
    RunArtifactRedaction, WebUiAuthenticatedCaller, parse_thread_id_field,
    run_artifact::ARTIFACT_REDACTION_PIPELINE,
};

pub const THREAD_ARTIFACT_SCHEMA: &str = "ironclaw.thread_artifact.v1";
pub const THREAD_ARTIFACT_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "thread_artifact",
    paginated: false,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornThreadArtifactRequest {
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornThreadArtifact {
    pub schema: String,
    pub generated_at: DateTime<Utc>,
    pub thread_id: String,
    pub messages: Vec<RunArtifactMessage>,
    pub logs: RunArtifactLogs,
    pub redaction: RunArtifactRedaction,
}

impl RebornServices {
    pub(super) async fn build_thread_artifact(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornThreadArtifactRequest,
    ) -> Result<RebornThreadArtifact, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let scope = caller.turn_scope(thread_id.clone());
        let (thread_scope, history) = self
            .resolve_thread_history_for_caller(caller.clone(), &scope)
            .await?;
        let records: Vec<ThreadMessageRecord> = history.messages;

        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let (messages, message_redaction_applied) = self
            .artifact_messages_for_records(thread_scope, &thread_id, records, &redactor)
            .await?;
        let (logs, log_redaction_applied) = self
            .artifact_logs(caller, thread_id.to_string(), None, &redactor)
            .await;

        Ok(RebornThreadArtifact {
            schema: THREAD_ARTIFACT_SCHEMA.to_string(),
            generated_at: Utc::now(),
            thread_id: thread_id.to_string(),
            messages,
            logs,
            redaction: RunArtifactRedaction {
                pipeline: ARTIFACT_REDACTION_PIPELINE.to_string(),
                applied: message_redaction_applied || log_redaction_applied,
            },
        })
    }
}
