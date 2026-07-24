//! Persistence port for installation lifecycle records.
//!
//! `ExtensionHost` is the only writer of installation state; this port is the
//! host's working set, rehydrated from durable installation membership records
//! at every boot. The record carries the
//! resolved contract (so a restore never needs the package source), the
//! current working state (`Installed` / `Active` / `Failed`), the non-secret
//! config values, and a typed, redacted last error.
//!
//! The in-memory implementation is the only implementation today: the host
//! record is a derived execution view, not the durable source of truth.

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

/// The derived execution view of installation records: rehydrated at boot
/// from the durable `ExtensionInstallationStore` and kept current by the
/// generic host's lifecycle writes (lifecycle.md: an in-memory registry is a
/// derived execution view, never the source of truth — there is no durable
/// twin of this port to keep in lock-step). Contract tests use it standalone.
#[derive(Default)]
pub struct RehydratedInstallationRecordStore {
    records: Mutex<Vec<InstallationRecord>>,
}

#[async_trait]
impl InstallationRecordStore for RehydratedInstallationRecordStore {
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
