//! Filesystem-backed [`CheckpointStateStore`] implementation.
//!
//! Persists host-owned loop checkpoint payloads under the `/checkpoint-state`
//! mount alias. The [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem)
//! resolves that alias through the caller-provided mount view on every
//! operation, so tenant/user isolation is structural. Within-tenant axes
//! (`agent_id`, `project_id`, and `thread_id`) stay in the alias-relative
//! path because they are part of the turn scope.
//!
//! Checkpoint payload bytes are intentionally not part of public turn status,
//! lifecycle events, or checkpoint metadata. This store is the private
//! host-owned payload side of the checkpoint contract.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::ScopedPath;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    CheckpointSchemaId, CheckpointStateRecord, CheckpointStateStore, GetCheckpointStateRequest,
    LoopCheckpointKind, LoopCheckpointStateRef, PutCheckpointStateRequest,
    RedactedCheckpointPayload, RunProfileVersion, TurnError, TurnId, TurnRunId, TurnScope,
    TurnTimestamp,
};

const CHECKPOINT_STATE_PREFIX: &str = "/checkpoint-state";

/// Filesystem-backed checkpoint-state payload store.
///
/// Construct with a [`ScopedFilesystem`] whose mount view exposes
/// `/checkpoint-state`. Payloads are stored as redacted host-owned records; the
/// public checkpoint metadata stores only the returned [`LoopCheckpointStateRef`].
pub struct FilesystemCheckpointStateStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemCheckpointStateStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(record: &StoredCheckpointStateRecord) -> Result<Entry, TurnError> {
        let body = serde_json::to_vec_pretty(record).map_err(|error| TurnError::Unavailable {
            reason: format!("checkpoint state serialization failed: {error}"),
        })?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
    }
}

#[async_trait]
impl<F> CheckpointStateStore for FilesystemCheckpointStateStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    async fn put_checkpoint_state(
        &self,
        request: PutCheckpointStateRequest,
    ) -> Result<CheckpointStateRecord, TurnError> {
        let scope = request.scope.clone();
        let turn_id = request.turn_id;
        let run_id = request.run_id;
        let schema_id = request.schema_id.clone();
        let schema_version = request.schema_version;
        let kind = request.kind;
        let state_ref = new_state_ref()?;
        let record = CheckpointStateRecord {
            state_ref: state_ref.clone(),
            scope,
            turn_id,
            run_id,
            schema_id,
            schema_version,
            kind,
            payload: RedactedCheckpointPayload::new(request.into_payload_bytes())
                .map_err(|reason| TurnError::InvalidRequest { reason })?,
            created_at: Utc::now(),
        };
        let stored = StoredCheckpointStateRecord::from_record(&record);
        let path = state_record_path(&record.scope, &state_ref)?;
        let entry = Self::record_entry(&stored)?;
        put_with_byte_fallback(
            self.filesystem.as_ref(),
            &record.scope,
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await?;
        Ok(record)
    }

    async fn get_checkpoint_state(
        &self,
        request: GetCheckpointStateRequest,
    ) -> Result<Option<CheckpointStateRecord>, TurnError> {
        let path = state_record_path(&request.scope, &request.state_ref)?;
        let Some(versioned) = self
            .filesystem
            .get(&request.scope.to_resource_scope(), &path)
            .await
            .map_err(fs_error)?
        else {
            return Ok(None);
        };
        let stored: StoredCheckpointStateRecord = serde_json::from_slice(&versioned.entry.body)
            .map_err(|error| TurnError::Unavailable {
                reason: format!("checkpoint state deserialization failed: {error}"),
            })?;
        let record = stored.into_record()?;
        if checkpoint_state_record_matches_request(&record, &request) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredCheckpointStateRecord {
    state_ref: LoopCheckpointStateRef,
    scope: TurnScope,
    turn_id: TurnId,
    run_id: TurnRunId,
    schema_id: CheckpointSchemaId,
    schema_version: RunProfileVersion,
    kind: LoopCheckpointKind,
    payload_hex: String,
    created_at: TurnTimestamp,
}

impl StoredCheckpointStateRecord {
    fn from_record(record: &CheckpointStateRecord) -> Self {
        Self {
            state_ref: record.state_ref.clone(),
            scope: record.scope.clone(),
            turn_id: record.turn_id,
            run_id: record.run_id,
            schema_id: record.schema_id.clone(),
            schema_version: record.schema_version,
            kind: record.kind,
            payload_hex: hex::encode(record.payload.as_bytes()),
            created_at: record.created_at,
        }
    }

    fn into_record(self) -> Result<CheckpointStateRecord, TurnError> {
        let payload = hex::decode(self.payload_hex).map_err(|error| TurnError::Unavailable {
            reason: format!("checkpoint state payload decoding failed: {error}"),
        })?;
        let payload =
            RedactedCheckpointPayload::new(payload).map_err(|reason| TurnError::Unavailable {
                reason: format!("checkpoint state payload was invalid: {reason}"),
            })?;
        Ok(CheckpointStateRecord {
            state_ref: self.state_ref,
            scope: self.scope,
            turn_id: self.turn_id,
            run_id: self.run_id,
            schema_id: self.schema_id,
            schema_version: self.schema_version,
            kind: self.kind,
            payload,
            created_at: self.created_at,
        })
    }
}

fn checkpoint_state_record_matches_request(
    record: &CheckpointStateRecord,
    request: &GetCheckpointStateRequest,
) -> bool {
    record.scope == request.scope
        && record.turn_id == request.turn_id
        && record.run_id == request.run_id
        && record.schema_id == request.schema_id
        && record.schema_version == request.schema_version
        && record.kind == request.kind
}

fn new_state_ref() -> Result<LoopCheckpointStateRef, TurnError> {
    LoopCheckpointStateRef::new(format!("checkpoint:{}", Uuid::new_v4())).map_err(|reason| {
        TurnError::Unavailable {
            reason: format!("generated checkpoint state ref was invalid: {reason}"),
        }
    })
}

fn state_record_path(
    scope: &TurnScope,
    state_ref: &LoopCheckpointStateRef,
) -> Result<ScopedPath, TurnError> {
    ScopedPath::new(format!(
        "{}/states/{}.json",
        scope_path_prefix(scope),
        state_ref.as_str().replace(':', "/")
    ))
    .map_err(|error| TurnError::Unavailable {
        reason: format!("invalid checkpoint state path: {error}"),
    })
}

fn scope_path_prefix(scope: &TurnScope) -> String {
    let mut base = String::from(CHECKPOINT_STATE_PREFIX);
    if let Some(agent_id) = &scope.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    base.push_str("/threads/");
    base.push_str(scope.thread_id.as_str());
    base
}

async fn put_with_byte_fallback<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &TurnScope,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    let fallback_entry = entry.clone();
    let resource_scope = scope.to_resource_scope();
    match filesystem.put(&resource_scope, path, entry, cas).await {
        Ok(_) => Ok(()),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => filesystem
            .put(&resource_scope, path, fallback_entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_error),
        Err(error) => Err(fs_error(error)),
    }
}

fn fs_error(error: FilesystemError) -> TurnError {
    tracing::debug!(%error, "checkpoint state filesystem operation failed");
    TurnError::Unavailable {
        reason: "checkpoint state persistence temporarily unavailable".to_string(),
    }
}
