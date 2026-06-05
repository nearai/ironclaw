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
//!
//! Records are write-once and intentionally append under `states/`. Retention
//! is owned by the loop/checkpoint consumer that knows which refs remain
//! reachable from durable checkpoint metadata.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::ScopedPath;
use serde::{Deserialize, Serialize};

use ironclaw_turns::{
    CheckpointSchemaId, CheckpointStateMatchMetadata, CheckpointStateRecord, CheckpointStateStore,
    GetCheckpointStateRequest, LoopCheckpointKind, LoopCheckpointStateRef,
    PutCheckpointStateRequest, RedactedCheckpointPayload, RunProfileVersion, TurnError, TurnId,
    TurnRunId, TurnScope, TurnTimestamp, checkpoint_state_metadata_matches_request,
    new_checkpoint_state_ref,
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
        let body = serde_json::to_vec(record).map_err(|error| TurnError::Unavailable {
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
        let state_ref = new_checkpoint_state_ref()?;
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
        put_with_byte_fallback(
            self.filesystem.as_ref(),
            &record.scope,
            &path,
            CasExpectation::Absent,
            || Self::record_entry(&stored),
        )
        .await?;
        Ok(record)
    }

    async fn get_checkpoint_state(
        &self,
        request: GetCheckpointStateRequest,
    ) -> Result<Option<CheckpointStateRecord>, TurnError> {
        let path = state_record_path(&request.scope, &request.state_ref)?;
        // The resolver needs the turn scope for tenant/user mount rewriting;
        // checkpoint paths only carry agent/project/thread below the alias.
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
        if stored.matches_request(&request) {
            Ok(Some(stored.into_record()?))
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

    fn matches_request(&self, request: &GetCheckpointStateRequest) -> bool {
        checkpoint_state_metadata_matches_request(
            CheckpointStateMatchMetadata {
                state_ref: &self.state_ref,
                scope: &self.scope,
                turn_id: self.turn_id,
                run_id: self.run_id,
                schema_id: &self.schema_id,
                schema_version: self.schema_version,
                kind: self.kind,
            },
            request,
        )
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
    cas: CasExpectation,
    build_entry: impl Fn() -> Result<Entry, TurnError>,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    // Unlike the singleton turn snapshot, checkpoint-state paths do not encode
    // tenant/user. The scoped filesystem resolver needs the turn scope so the
    // `/checkpoint-state` alias resolves under the correct tenant/user mount.
    let resource_scope = scope.to_resource_scope();
    let entry = build_entry()?;
    match filesystem.put(&resource_scope, path, entry, cas).await {
        Ok(_) => Ok(()),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => {
            let fallback_entry = build_entry()?;
            put_bytes_compat(filesystem, &resource_scope, path, fallback_entry.body, cas).await
        }
        Err(FilesystemError::VersionMismatch { .. }) => Err(checkpoint_state_already_exists()),
        Err(error) => Err(fs_error(error)),
    }
}

async fn put_bytes_compat<F>(
    filesystem: &ScopedFilesystem<F>,
    resource_scope: &ironclaw_host_api::ResourceScope,
    path: &ScopedPath,
    body: Vec<u8>,
    cas: CasExpectation,
) -> Result<(), TurnError>
where
    F: RootFilesystem,
{
    match cas {
        CasExpectation::Any => filesystem
            .write_file(resource_scope, path, &body)
            .await
            .map_err(fs_error),
        CasExpectation::Absent => {
            if filesystem
                .get(resource_scope, path)
                .await
                .map_err(fs_error)?
                .is_some()
            {
                return Err(checkpoint_state_already_exists());
            }
            filesystem
                .write_file(resource_scope, path, &body)
                .await
                .map_err(fs_error)
        }
        CasExpectation::Version(_) => Err(TurnError::Unavailable {
            reason: "checkpoint state backend cannot honor versioned CAS".to_string(),
        }),
    }
}

fn checkpoint_state_already_exists() -> TurnError {
    TurnError::Conflict {
        reason: "checkpoint state ref already exists".to_string(),
    }
}

fn fs_error(error: FilesystemError) -> TurnError {
    tracing::debug!(%error, "checkpoint state filesystem operation failed");
    TurnError::Unavailable {
        reason: "checkpoint state persistence temporarily unavailable".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use ironclaw_filesystem::{DirEntry, FileStat, FileType, RecordVersion, VersionedEntry};
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
        ThreadId, VirtualPath,
    };

    use super::*;

    #[derive(Default)]
    struct LegacyBytesFilesystem {
        files: Mutex<HashMap<String, Vec<u8>>>,
    }

    #[async_trait]
    impl RootFilesystem for LegacyBytesFilesystem {
        async fn put(
            &self,
            path: &VirtualPath,
            _entry: Entry,
            _cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            })
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            Ok(self
                .files
                .lock()
                .unwrap()
                .get(path.as_str())
                .map(|body| VersionedEntry {
                    path: path.clone(),
                    entry: Entry::bytes(body.clone()),
                    version: RecordVersion::from_backend(0),
                }))
        }

        async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            self.files
                .lock()
                .unwrap()
                .get(path.as_str())
                .cloned()
                .ok_or_else(|| FilesystemError::NotFound {
                    path: path.clone(),
                    operation: FilesystemOperation::ReadFile,
                })
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            self.files
                .lock()
                .unwrap()
                .insert(path.as_str().to_string(), bytes.to_vec());
            Ok(())
        }

        async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Ok(Vec::new())
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            let len = self
                .files
                .lock()
                .unwrap()
                .get(path.as_str())
                .map(|body| body.len() as u64)
                .ok_or_else(|| FilesystemError::NotFound {
                    path: path.clone(),
                    operation: FilesystemOperation::Stat,
                })?;
            Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len,
                modified: None,
                sensitive: false,
            })
        }
    }

    fn scoped_legacy_fs() -> ScopedFilesystem<LegacyBytesFilesystem> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/checkpoint-state").unwrap(),
            VirtualPath::new("/engine/checkpoint-state").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        ScopedFilesystem::with_fixed_view(Arc::new(LegacyBytesFilesystem::default()), mounts)
    }

    fn test_scope() -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant1").unwrap(),
            Some(AgentId::new("agent1").unwrap()),
            Some(ProjectId::new("project1").unwrap()),
            ThreadId::new("thread1").unwrap(),
        )
    }

    #[tokio::test]
    async fn byte_fallback_preserves_absent_write_once() {
        let filesystem = scoped_legacy_fs();
        let scope = test_scope();
        let state_ref = LoopCheckpointStateRef::new("checkpoint:fallback").unwrap();
        let path = state_record_path(&scope, &state_ref).unwrap();

        put_with_byte_fallback(&filesystem, &scope, &path, CasExpectation::Absent, || {
            Ok(Entry::bytes(b"first".to_vec()))
        })
        .await
        .unwrap();
        let error =
            put_with_byte_fallback(&filesystem, &scope, &path, CasExpectation::Absent, || {
                Ok(Entry::bytes(b"second".to_vec()))
            })
            .await
            .unwrap_err();

        assert!(matches!(error, TurnError::Conflict { .. }));
        let stored = filesystem
            .read_file(&scope.to_resource_scope(), &path)
            .await
            .unwrap();
        assert_eq!(stored, b"first");
    }
}
