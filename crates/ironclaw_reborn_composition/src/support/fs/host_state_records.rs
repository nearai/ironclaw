//! Shared channel-host durable-state helpers: JSON record read/write over a
//! `ScopedFilesystem` with stable label-tagged infrastructure errors, and the
//! weak-map of per-key async locks the host states use to serialize
//! multi-record read-modify-write sequences in-process (individual-record
//! version CAS remains the cross-process guard).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use serde::{Serialize, de::DeserializeOwned};

/// Weak-map of per-key `tokio::sync::Mutex` locks: entries evaporate once the
/// last holder drops, so the map stays bounded by live keys.
#[derive(Debug, Default)]
pub(crate) struct KeyedAsyncLocks {
    locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
}

impl KeyedAsyncLocks {
    pub(crate) fn lock_for(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }
}

pub(crate) async fn read_json_record<F, T>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    label: &'static str,
) -> Result<Option<(T, RecordVersion)>, FilesystemError>
where
    F: RootFilesystem + 'static,
    T: DeserializeOwned,
{
    let Some(versioned) = filesystem.get(scope, path).await? else {
        return Ok(None);
    };
    let value = serde_json::from_slice(&versioned.entry.body).map_err(|_| {
        FilesystemError::BackendInfrastructure {
            operation: FilesystemOperation::ReadFile,
            reason: format!("{label} record is invalid JSON"),
        }
    })?;
    Ok(Some((value, versioned.version)))
}

pub(crate) async fn write_json_record<F, T>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    value: &T,
    cas: CasExpectation,
    label: &'static str,
) -> Result<RecordVersion, FilesystemError>
where
    F: RootFilesystem + 'static,
    T: Serialize,
{
    let body = serde_json::to_vec(value).map_err(|_| FilesystemError::BackendInfrastructure {
        operation: FilesystemOperation::WriteFile,
        reason: format!("{label} record could not be serialized"),
    })?;
    filesystem
        .put(
            scope,
            path,
            Entry::bytes(body).with_content_type(ContentType::json()),
            cas,
        )
        .await
}
