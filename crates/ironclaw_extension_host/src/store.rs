//! Persistence port for installation lifecycle records.
//!
//! `ExtensionHost` is the only writer of installation state; this port is how
//! it persists each transition so a crash mid-transition resumes
//! deterministically at startup. The record carries the resolved contract
//! (so restore never needs the package source), the current lifecycle state,
//! the non-secret config values, and a typed, redacted last error.
//!
//! Production implementations back this on the durable Reborn filesystem
//! (both DB backends). This crate ships the in-memory implementation used by
//! contract tests; the composition-side durable implementation is wired in
//! P2's cutover.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::ResolvedExtensionManifest;
use tokio::sync::Mutex;

use crate::state::InstallationState;

/// One persisted installation record.
#[derive(Clone)]
pub struct InstallationRecord {
    pub extension_id: String,
    pub installation_id: String,
    pub state: InstallationState,
    pub resolved: Arc<ResolvedExtensionManifest>,
    /// Non-secret operator config values keyed by field handle.
    pub config: Vec<(String, String)>,
    /// A typed, redacted reason for the last failure, if any.
    pub last_error: Option<String>,
}

/// Persistence port for installation records.
#[async_trait]
pub trait InstallationRecordStore: Send + Sync {
    async fn list(&self) -> Result<Vec<InstallationRecord>, StoreError>;
    async fn get(&self, extension_id: &str) -> Result<Option<InstallationRecord>, StoreError>;
    async fn upsert(&self, record: InstallationRecord) -> Result<(), StoreError>;
    async fn delete(&self, extension_id: &str) -> Result<(), StoreError>;
}

/// Typed store failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("installation record store is unavailable: {reason}")]
    Unavailable { reason: String },
}

/// In-memory installation record store for contract tests.
#[derive(Default)]
pub struct InMemoryInstallationRecordStore {
    records: Mutex<Vec<InstallationRecord>>,
}

#[async_trait]
impl InstallationRecordStore for InMemoryInstallationRecordStore {
    async fn list(&self) -> Result<Vec<InstallationRecord>, StoreError> {
        Ok(self.records.lock().await.clone())
    }

    async fn get(&self, extension_id: &str) -> Result<Option<InstallationRecord>, StoreError> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .find(|record| record.extension_id == extension_id)
            .cloned())
    }

    async fn upsert(&self, record: InstallationRecord) -> Result<(), StoreError> {
        let mut records = self.records.lock().await;
        if let Some(existing) = records
            .iter_mut()
            .find(|existing| existing.extension_id == record.extension_id)
        {
            *existing = record;
        } else {
            records.push(record);
        }
        Ok(())
    }

    async fn delete(&self, extension_id: &str) -> Result<(), StoreError> {
        self.records
            .lock()
            .await
            .retain(|record| record.extension_id != extension_id);
        Ok(())
    }
}
