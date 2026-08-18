//! Caller-owned, redacted evidence bundle for a complete thread.

use chrono::{DateTime, Utc};
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode,
};
use ironclaw_trace_commons::contribution::DeterministicTraceRedactor;

use ironclaw_threads::{BoundedThreadMessages, BoundedThreadMessagesRequest};
use ironclaw_turns::TurnRunId;
use serde::{Deserialize, Serialize};

use ironclaw_product_contracts::views::{RebornViewDescriptor, RebornViewProvider};

use super::{
    ProductCapabilityInvoker, RebornServices, RunArtifactLogs, RunArtifactMessage,
    RunArtifactRedaction, map_timeline_probe_error, parse_thread_id_field,
    run_artifact::{
        ARTIFACT_REDACTION_PIPELINE, artifact_messages, context_messages_by_id,
        timings::RunArtifactTimings,
    },
    thread_scope_from_turn_scope,
};

pub const THREAD_ARTIFACT_SCHEMA: &str = "ironclaw.thread_artifact.v1";
pub use ironclaw_product_contracts::product_wire::RebornThreadArtifactRequest;

pub const THREAD_ARTIFACT_MAX_MESSAGES: usize = 1_000;
const THREAD_ARTIFACT_MAX_STORED_BYTES: usize = 16 * 1024 * 1024;
const THREAD_ARTIFACT_MAX_SERIALIZED_BYTES: usize = 20 * 1024 * 1024;
pub const THREAD_ARTIFACT_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "thread_artifact",
    paginated: false,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornThreadArtifact {
    pub schema: String,
    pub generated_at: DateTime<Utc>,
    pub thread_id: String,
    pub messages: Vec<RunArtifactMessage>,
    pub logs: RunArtifactLogs,
    /// One timing block per run still resident in the process-local store.
    /// Absent runs are simply not listed.
    #[serde(default)]
    pub timings_by_run: Vec<RunArtifactRunTimings>,
    pub redaction: RunArtifactRedaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactRunTimings {
    pub run_id: String,
    pub timings: RunArtifactTimings,
}

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    pub(super) async fn build_thread_artifact(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornThreadArtifactRequest,
    ) -> Result<RebornThreadArtifact, ProductSurfaceError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let scope = caller.turn_scope(thread_id.clone());
        let actor = caller.actor();
        let access = self
            .resolve_thread_access_for_caller(caller.clone(), scope, &actor)
            .await?;
        let thread_scope =
            thread_scope_from_turn_scope(&access.scope, Some(access.run_actor.user_id.clone()))?;
        let snapshot = match self
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
            BoundedThreadMessages::Complete(snapshot) => snapshot,
            BoundedThreadMessages::LimitExceeded => return Err(thread_artifact_too_large()),
        };

        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let context_by_id = context_messages_by_id(snapshot.context.messages);
        let (messages, message_redaction_applied) =
            artifact_messages(snapshot.history.messages, &context_by_id, &redactor);

        // Compute per-run timings before `artifact_logs` takes `caller` by
        // value. Single pass: bucket messages by run first, so each run's
        // timing computation (`derive_wall_clock_ms`'s `updated_at` scan in
        // particular) sees only its own messages instead of the whole
        // thread's — passing the full `messages` list per run would let one
        // run's wall-clock span reach into a later run's activity, and would
        // also re-scan the whole list once per distinct run.
        let mut runs_in_order: Vec<String> = Vec::new();
        let mut messages_by_run: std::collections::HashMap<String, Vec<RunArtifactMessage>> =
            std::collections::HashMap::new();
        for message in &messages {
            // silent-ok: a message with no run_id (pre-turn history, or a
            // non-run message kind) simply contributes no timings entry.
            let Some(run_id_text) = message.run_id.as_deref() else {
                continue;
            };
            messages_by_run
                .entry(run_id_text.to_string())
                .or_insert_with(|| {
                    runs_in_order.push(run_id_text.to_string());
                    Vec::new()
                })
                .push(message.clone());
        }

        let mut timings_by_run = Vec::new();
        for run_id_text in &runs_in_order {
            let run_messages = &messages_by_run[run_id_text];
            // silent-ok: `run_id_text` came from a durably persisted
            // `turn_run_id` that was validated as a `TurnRunId` at write
            // time; a parse failure here would mean corrupted storage, and
            // skipping just this run's timings keeps the export best-effort
            // (see module docs: never fail the whole artifact for one run).
            let Ok(run_id) = TurnRunId::parse(run_id_text) else {
                continue;
            };
            // No per-run `received_at` here; the earliest message creation in
            // the run is the closest durable origin available.
            // silent-ok: no message in the run carries `created_at` (only
            // possible for pre-timestamp records); wall-clock timing has no
            // origin to measure from, so this run is left out rather than
            // reported with a fabricated span.
            let Some(origin) = run_messages.iter().filter_map(|m| m.created_at).min() else {
                continue;
            };
            let timings = self.artifact_timings(&caller, &thread_id, &run_id, origin, run_messages);
            if timings.available {
                timings_by_run.push(RunArtifactRunTimings {
                    run_id: run_id_text.clone(),
                    timings,
                });
            }
        }

        let (logs, log_redaction_applied) = self
            .artifact_logs(caller, &thread_id, None, &redactor)
            .await;

        let artifact = RebornThreadArtifact {
            schema: THREAD_ARTIFACT_SCHEMA.to_string(),
            generated_at: Utc::now(),
            thread_id: thread_id.to_string(),
            messages,
            logs,
            timings_by_run,
            redaction: RunArtifactRedaction {
                pipeline: ARTIFACT_REDACTION_PIPELINE.to_string(),
                applied: message_redaction_applied || log_redaction_applied,
            },
        };
        let serialized_bytes = serde_json::to_vec(&artifact)
            .map_err(ProductSurfaceError::internal_from)?
            .len();
        if serialized_bytes > THREAD_ARTIFACT_MAX_SERIALIZED_BYTES {
            return Err(thread_artifact_too_large());
        }
        Ok(artifact)
    }
}

fn thread_artifact_too_large() -> ProductSurfaceError {
    ProductSurfaceError::from_status(ProductSurfaceErrorCode::InvalidRequest, 413, false)
}
