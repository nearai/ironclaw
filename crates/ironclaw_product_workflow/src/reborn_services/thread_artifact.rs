//! Caller-owned, redacted evidence bundle for a complete thread.

use chrono::{DateTime, Utc};
use ironclaw_reborn_traces::contribution::DeterministicTraceRedactor;
use ironclaw_threads::{BoundedThreadMessages, BoundedThreadMessagesRequest, ThreadMessageRecord};
use serde::{Deserialize, Serialize};

use super::{
    RebornServices, RebornServicesError, RebornServicesErrorCode, RebornViewDescriptor,
    RunArtifactLogs, RunArtifactMessage, RunArtifactRedaction, WebUiAuthenticatedCaller,
    map_timeline_probe_error, parse_thread_id_field, run_artifact::ARTIFACT_REDACTION_PIPELINE,
    thread_scope_from_turn_scope,
};

pub const THREAD_ARTIFACT_SCHEMA: &str = "ironclaw.thread_artifact.v1";
const THREAD_ARTIFACT_MAX_MESSAGES: usize = 1_000;
const THREAD_ARTIFACT_MAX_STORED_BYTES: usize = 16 * 1024 * 1024;
const THREAD_ARTIFACT_MAX_SERIALIZED_BYTES: usize = 20 * 1024 * 1024;
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
        let actor = caller.actor();
        let access = self
            .resolve_thread_access_for_caller(caller.clone(), scope, &actor)
            .await?;
        let thread_scope =
            thread_scope_from_turn_scope(&access.scope, Some(access.run_actor.user_id.clone()))?;
        let records: Vec<ThreadMessageRecord> = match self
            .thread_service
            .list_thread_messages_bounded(BoundedThreadMessagesRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                max_messages: THREAD_ARTIFACT_MAX_MESSAGES,
                max_bytes: THREAD_ARTIFACT_MAX_STORED_BYTES,
            })
            .await
            .map_err(map_timeline_probe_error)?
        {
            BoundedThreadMessages::Complete(history) => history.messages,
            BoundedThreadMessages::LimitExceeded => return Err(thread_artifact_too_large()),
        };

        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let (messages, message_redaction_applied) = self
            .artifact_messages_for_records(thread_scope, &thread_id, records, &redactor)
            .await?;
        let (logs, log_redaction_applied) = self
            .artifact_logs(caller, thread_id.to_string(), None, &redactor)
            .await;

        let artifact = RebornThreadArtifact {
            schema: THREAD_ARTIFACT_SCHEMA.to_string(),
            generated_at: Utc::now(),
            thread_id: thread_id.to_string(),
            messages,
            logs,
            redaction: RunArtifactRedaction {
                pipeline: ARTIFACT_REDACTION_PIPELINE.to_string(),
                applied: message_redaction_applied || log_redaction_applied,
            },
        };
        let serialized_bytes = serde_json::to_vec(&artifact)
            .map_err(RebornServicesError::internal_from)?
            .len();
        if serialized_bytes > THREAD_ARTIFACT_MAX_SERIALIZED_BYTES {
            return Err(thread_artifact_too_large());
        }
        Ok(artifact)
    }
}

fn thread_artifact_too_large() -> RebornServicesError {
    RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 413, false)
}
