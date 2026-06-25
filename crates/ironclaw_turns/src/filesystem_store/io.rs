use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordKind,
    RecordVersion, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};

use crate::{TurnError, TurnPersistenceSnapshot};

pub(super) const FILESYSTEM_CAS_RETRIES: usize = 32;

const FILESYSTEM_CAS_BACKOFF_BASE: Duration = Duration::from_millis(2);
const FILESYSTEM_CAS_BACKOFF_MAX: Duration = Duration::from_millis(50);

const TURNS_PREFIX: &str = "/turns";
const TURNS_SNAPSHOT_FILE: &str = "state.json";
const TURNS_SNAPSHOT_KIND: &str = "turn_state_snapshot";

pub(super) fn snapshot_path() -> Result<ScopedPath, TurnError> {
    ScopedPath::new(format!("{TURNS_PREFIX}/{TURNS_SNAPSHOT_FILE}")).map_err(|error| {
        TurnError::Unavailable {
            reason: format!("invalid turn-state snapshot path: {error}"),
        }
    })
}

pub(super) fn snapshot_entry(snapshot: &TurnPersistenceSnapshot) -> Result<Entry, TurnError> {
    let body = serde_json::to_vec_pretty(snapshot).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state snapshot serialization failed: {error}"),
    })?;
    let kind = RecordKind::new(TURNS_SNAPSHOT_KIND).map_err(|error| TurnError::Unavailable {
        reason: format!("invalid turn-state snapshot record kind: {error}"),
    })?;
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    entry.kind = Some(kind);
    Ok(entry)
}

pub(super) fn deserialize_snapshot(bytes: &[u8]) -> Result<TurnPersistenceSnapshot, TurnError> {
    serde_json::from_slice(bytes).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state snapshot deserialization failed: {error}"),
    })
}

pub(super) fn fs_error(error: FilesystemError) -> TurnError {
    tracing::debug!(%error, "turn state filesystem operation failed");
    TurnError::Unavailable {
        reason: "turn state persistence temporarily unavailable".to_string(),
    }
}

pub(super) async fn cas_retry_backoff(attempt: usize) {
    let shift = attempt.min(8) as u32;
    let multiplier = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
    let base_delay = FILESYSTEM_CAS_BACKOFF_BASE
        .saturating_mul(multiplier)
        .min(FILESYSTEM_CAS_BACKOFF_MAX);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| {
            let jitter_ceiling = base_delay.as_millis().max(1);
            Duration::from_millis((elapsed.as_nanos() % jitter_ceiling) as u64)
        })
        .unwrap_or_default();
    tokio::time::sleep(base_delay.saturating_add(jitter)).await;
}

/// Local error classification for the CAS-aware put helper.
pub(super) enum PutError {
    /// Backend reported `VersionMismatch` (cross-process raced us). The
    /// caller retries by re-reading the current snapshot.
    VersionMismatch,
    /// Any other backend or serialization failure; surface to caller.
    Other(TurnError),
}

/// Issue a `put` honoring the requested CAS expectation.
///
/// Turn state is a single per-user snapshot, so this store requires a backend
/// with real `Absent` / `Version` CAS. Falling back to `Any` would turn a
/// stale-snapshot race into a blind overwrite.
pub(super) async fn put_with_cas<F>(
    filesystem: &ScopedFilesystem<F>,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<RecordVersion, PutError>
where
    F: RootFilesystem,
{
    let scope = ResourceScope::system();
    match filesystem.put(&scope, path, entry, cas).await {
        Ok(version) => Ok(version),
        Err(FilesystemError::VersionMismatch { .. }) => Err(PutError::VersionMismatch),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => Err(PutError::Other(TurnError::Unavailable {
            reason: "turn state filesystem backend must support versioned CAS".to_string(),
        })),
        Err(error) => Err(PutError::Other(fs_error(error))),
    }
}
