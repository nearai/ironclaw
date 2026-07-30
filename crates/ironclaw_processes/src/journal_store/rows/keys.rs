use ironclaw_filesystem::{IndexKey, IndexKind, IndexName, IndexSpec};
use ironclaw_host_api::{ResourceScope, ScopedPath};

use super::super::ProcessJournalStoreError;
use crate::{ProcessKind, ProcessLifecycleStatus};

pub(super) fn index_name(name: &str) -> Result<IndexName, ProcessJournalStoreError> {
    IndexName::new(name).map_err(|error| {
        ProcessJournalStoreError::InvalidPath(crate::types::invalid_path(error).to_string())
    })
}

pub(super) fn ordered_index(
    name: &str,
    keys: &[&str],
) -> Result<IndexSpec, ProcessJournalStoreError> {
    Ok(IndexSpec::new(
        index_name(name)?,
        keys.iter()
            .map(|key| index_key(key))
            .collect::<Result<Vec<_>, _>>()?,
        IndexKind::Exact,
    ))
}

pub(super) fn process_kind_key(kind: &ProcessKind) -> Result<String, ProcessJournalStoreError> {
    serde_json::to_value(kind)
        .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))
        .and_then(|value| match value {
            serde_json::Value::String(value) => Ok(value),
            serde_json::Value::Object(value) if value.len() == 1 => {
                let (kind, detail) = value.into_iter().next().ok_or_else(|| {
                    ProcessJournalStoreError::Serialization(
                        "extension-defined process kind had no value".to_string(),
                    )
                })?;
                Ok(format!("{kind}:{}", detail.as_str().unwrap_or_default()))
            }
            _ => Err(ProcessJournalStoreError::Serialization(
                "process kind did not serialize to an indexable value".to_string(),
            )),
        })
}

pub(super) fn process_status_key(
    status: ProcessLifecycleStatus,
) -> Result<String, ProcessJournalStoreError> {
    serde_json::to_value(status)
        .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| {
            ProcessJournalStoreError::Serialization(
                "process status did not serialize to an indexable value".to_string(),
            )
        })
}

pub(super) fn scope_owner_key(scope: &ResourceScope) -> Result<String, ProcessJournalStoreError> {
    serde_json::to_string(&(
        &scope.tenant_id,
        &scope.user_id,
        &scope.agent_id,
        &scope.project_id,
        &scope.mission_id,
        &scope.thread_id,
    ))
    .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))
}

pub(super) fn owner_scope_key(scope: &ResourceScope) -> Result<String, ProcessJournalStoreError> {
    serde_json::to_string(&(&scope.tenant_id, &scope.user_id))
        .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))
}

pub(super) fn lineage_scope_key(scope: &ResourceScope) -> Result<String, ProcessJournalStoreError> {
    serde_json::to_string(&(
        &scope.tenant_id,
        &scope.user_id,
        &scope.agent_id,
        &scope.project_id,
        &scope.mission_id,
    ))
    .map_err(|error| ProcessJournalStoreError::Serialization(error.to_string()))
}

pub(super) fn index_key(value: &str) -> Result<IndexKey, ProcessJournalStoreError> {
    IndexKey::new(value).map_err(|error| {
        ProcessJournalStoreError::InvalidPath(crate::types::invalid_path(error).to_string())
    })
}

pub(super) fn scoped_path(value: &str) -> Result<ScopedPath, ProcessJournalStoreError> {
    ScopedPath::new(value).map_err(|error| {
        ProcessJournalStoreError::InvalidPath(crate::types::invalid_path(error).to_string())
    })
}
