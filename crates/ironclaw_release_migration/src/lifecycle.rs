use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{Duration, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, RecordKind, RecordVersion, RootFilesystem,
};
use ironclaw_host_api::path::VirtualPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ReleasePairMigrationError;

const LEASE_MINUTES: i64 = 10;
const HEARTBEAT_SECONDS: u64 = 60;
const ACQUIRE_RETRIES: usize = 8;

/// Stable identity and rollback contract for one supported direct upgrade.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReleasePair {
    pub(crate) schema: &'static str,
    pub(crate) source_release: &'static str,
    pub(crate) target_release: &'static str,
    pub(crate) migration_path: &'static str,
    pub(crate) domain_migration_root: &'static str,
    pub(crate) old_authorities_retained: bool,
    pub(crate) in_place_rows_backward_readable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MigrationStatus {
    InProgress,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MigrationRecord {
    pub(crate) schema: String,
    pub(crate) source_release: String,
    pub(crate) target_release: String,
    pub(crate) status: MigrationStatus,
    pub(crate) attempt_id: String,
    pub(crate) started_at: chrono::DateTime<Utc>,
    pub(crate) lease_expires_at: chrono::DateTime<Utc>,
    pub(crate) finished_at: Option<chrono::DateTime<Utc>>,
    pub(crate) source_fingerprint: Value,
    /// Counts/status only: no paths, actor ids, payloads, or secret handles.
    pub(crate) report: Option<Value>,
    pub(crate) rollback: RollbackDisposition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RollbackDisposition {
    pub(crate) old_authorities_retained: bool,
    pub(crate) in_place_rows_backward_readable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DomainCompletionRecord {
    pub(crate) schema: String,
    pub(crate) source_release: String,
    pub(crate) target_release: String,
    pub(crate) domain: String,
    pub(crate) status: MigrationStatus,
    pub(crate) completed_at: chrono::DateTime<Utc>,
    /// Counts/status only; the domain report contains no raw paths, actor ids,
    /// payloads, credentials, or secret handles.
    pub(crate) report: Value,
}

pub(crate) struct ReleasePairMigrationLease<F: ?Sized>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<F>,
    pair: ReleasePair,
    path: VirtualPath,
    version: Arc<tokio::sync::Mutex<RecordVersion>>,
    lost_lease: Arc<AtomicBool>,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
    record: MigrationRecord,
    finished_locally: bool,
}

impl<F> ReleasePairMigrationLease<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    pub(crate) async fn acquire<FingerprintFuture>(
        filesystem: Arc<F>,
        pair: ReleasePair,
        fingerprint: FingerprintFuture,
    ) -> Result<Self, ReleasePairMigrationError>
    where
        FingerprintFuture: Future<Output = Result<Value, ReleasePairMigrationError>>,
    {
        let path = migration_path(pair)?;
        let mut source_fingerprint = None;
        let mut fingerprint = Some(fingerprint);
        for _ in 0..ACQUIRE_RETRIES {
            let existing = filesystem.get(&path).await?;
            let now = Utc::now();
            let (cas, was_complete, retained_fingerprint) = match existing {
                Some(versioned) => {
                    let prior: MigrationRecord = serde_json::from_slice(&versioned.entry.body)
                        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
                    validate_pair(&prior, pair)?;
                    if prior.status == MigrationStatus::InProgress && prior.lease_expires_at > now {
                        return Err(ReleasePairMigrationError::ConcurrentStartup);
                    }
                    (
                        CasExpectation::Version(versioned.version),
                        prior.status == MigrationStatus::Complete,
                        (prior.status == MigrationStatus::Complete)
                            .then_some(prior.source_fingerprint),
                    )
                }
                None => (CasExpectation::Absent, false, None),
            };
            let fingerprint = match retained_fingerprint.or_else(|| source_fingerprint.clone()) {
                Some(fingerprint) => fingerprint,
                None => {
                    let fingerprint = fingerprint
                        .take()
                        .ok_or_else(|| {
                            ReleasePairMigrationError::Malformed(
                                "source fingerprint future was consumed before lease acquisition"
                                    .to_string(),
                            )
                        })?
                        .await?;
                    source_fingerprint = Some(fingerprint.clone());
                    fingerprint
                }
            };
            let record = MigrationRecord {
                schema: pair.schema.to_string(),
                source_release: pair.source_release.to_string(),
                target_release: pair.target_release.to_string(),
                status: MigrationStatus::InProgress,
                attempt_id: uuid::Uuid::new_v4().to_string(),
                started_at: now,
                lease_expires_at: now + Duration::minutes(LEASE_MINUTES),
                finished_at: None,
                source_fingerprint: fingerprint,
                report: None,
                // Every transform in this release pair is copy-forward or a
                // backward-readable projection refresh. The retained rc1
                // authorities are the recoverable rollback snapshot.
                rollback: RollbackDisposition {
                    old_authorities_retained: pair.old_authorities_retained,
                    in_place_rows_backward_readable: pair.in_place_rows_backward_readable,
                },
            };
            match filesystem.put(&path, migration_entry(&record)?, cas).await {
                Ok(version) => {
                    if was_complete {
                        tracing::debug!(
                            source_release = pair.source_release,
                            target_release = pair.target_release,
                            "re-verifying completed release-pair migration"
                        );
                    }
                    let version = Arc::new(tokio::sync::Mutex::new(version));
                    let lost_lease = Arc::new(AtomicBool::new(false));
                    let heartbeat = spawn_lease_heartbeat(
                        Arc::clone(&filesystem),
                        path.clone(),
                        Arc::clone(&version),
                        Arc::clone(&lost_lease),
                        record.clone(),
                    );
                    return Ok(Self {
                        filesystem,
                        pair,
                        path,
                        version,
                        lost_lease,
                        heartbeat: Some(heartbeat),
                        record,
                        finished_locally: false,
                    });
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(ReleasePairMigrationError::ConcurrentStartup)
    }

    pub(crate) async fn complete(mut self, report: Value) -> Result<(), ReleasePairMigrationError> {
        self.stop_heartbeat().await?;
        self.record.status = MigrationStatus::Complete;
        self.record.finished_at = Some(Utc::now());
        self.record.report = Some(report.clone());
        self.replace().await?;
        self.finished_locally = true;
        self.write_domain_completion_records(&report).await?;
        Ok(())
    }

    pub(crate) async fn fail(mut self) -> Result<(), ReleasePairMigrationError> {
        self.stop_heartbeat().await?;
        self.record.status = MigrationStatus::Failed;
        self.record.finished_at = Some(Utc::now());
        self.record.report = None;
        self.replace().await?;
        self.finished_locally = true;
        Ok(())
    }

    /// Best-effort cleanup for a startup path that is already returning its
    /// primary error. A failed cleanup remains observable without replacing
    /// the error that caused startup to abort.
    pub(crate) async fn fail_and_log(self) {
        if let Err(error) = self.fail().await {
            tracing::error!(%error, "release-pair migration lease could not be marked failed");
        }
    }

    async fn replace(&mut self) -> Result<(), ReleasePairMigrationError> {
        if self.lost_lease.load(Ordering::Acquire) {
            return Err(ReleasePairMigrationError::LostLease);
        }
        let mut version = self.version.lock().await;
        match self
            .filesystem
            .put(
                &self.path,
                migration_entry(&self.record)?,
                CasExpectation::Version(*version),
            )
            .await
        {
            Ok(next) => {
                *version = next;
                Ok(())
            }
            Err(FilesystemError::VersionMismatch { .. }) => {
                Err(ReleasePairMigrationError::LostLease)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn stop_heartbeat(&mut self) -> Result<(), ReleasePairMigrationError> {
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
            let _ = heartbeat.await;
        }
        let observed = self
            .filesystem
            .get(&self.path)
            .await?
            .ok_or(ReleasePairMigrationError::LostLease)?;
        let stored: MigrationRecord = serde_json::from_slice(&observed.entry.body)
            .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
        if stored.attempt_id != self.record.attempt_id
            || stored.status != MigrationStatus::InProgress
        {
            return Err(ReleasePairMigrationError::LostLease);
        }
        *self.version.lock().await = observed.version;
        Ok(())
    }

    async fn write_domain_completion_records(
        &self,
        report: &Value,
    ) -> Result<(), ReleasePairMigrationError> {
        let domains = report.as_object().ok_or_else(|| {
            ReleasePairMigrationError::Malformed(
                "redacted migration report is not an object".to_string(),
            )
        })?;
        for (domain, counts) in domains {
            if domain.is_empty()
                || !domain
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            {
                return Err(ReleasePairMigrationError::Malformed(
                    "migration report contains an invalid domain".to_string(),
                ));
            }
            let path = virtual_path(&format!(
                "{}/{domain}-v1.complete.json",
                self.pair.domain_migration_root
            ))?;
            let record = DomainCompletionRecord {
                schema: "release-pair-domain-completion-v1".to_string(),
                source_release: self.pair.source_release.to_string(),
                target_release: self.pair.target_release.to_string(),
                domain: domain.clone(),
                status: MigrationStatus::Complete,
                completed_at: Utc::now(),
                report: counts.clone(),
            };
            let kind = RecordKind::new("release_pair_domain_completion")
                .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
            let value = serde_json::to_value(&record)
                .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
            let entry = Entry::record(kind, &value)
                .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Absent)
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => {
                    let existing = self.filesystem.get(&path).await?.ok_or_else(|| {
                        ReleasePairMigrationError::Malformed(
                            "domain completion disappeared during verification".to_string(),
                        )
                    })?;
                    let existing_record: DomainCompletionRecord =
                        serde_json::from_slice(&existing.entry.body).map_err(|error| {
                            ReleasePairMigrationError::Malformed(error.to_string())
                        })?;
                    if existing_record.schema != record.schema
                        || existing_record.source_release != self.pair.source_release
                        || existing_record.target_release != self.pair.target_release
                        || existing_record.domain != *domain
                        || existing_record.status != MigrationStatus::Complete
                    {
                        return Err(ReleasePairMigrationError::UnsupportedReleasePair);
                    }
                    if existing_record.report != record.report {
                        let entry = Entry::record(
                            RecordKind::new("release_pair_domain_completion").map_err(|error| {
                                ReleasePairMigrationError::Malformed(error.to_string())
                            })?,
                            &serde_json::to_value(&record).map_err(|error| {
                                ReleasePairMigrationError::Malformed(error.to_string())
                            })?,
                        )
                        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
                        self.filesystem
                            .put(&path, entry, CasExpectation::Version(existing.version))
                            .await?;
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

fn spawn_lease_heartbeat<F>(
    filesystem: Arc<F>,
    path: VirtualPath,
    version: Arc<tokio::sync::Mutex<RecordVersion>>,
    lost_lease: Arc<AtomicBool>,
    record: MigrationRecord,
) -> tokio::task::JoinHandle<()>
where
    F: RootFilesystem + ?Sized + 'static,
{
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(HEARTBEAT_SECONDS)).await;
            let mut version = version.lock().await;
            let mut heartbeat_record = record.clone();
            heartbeat_record.lease_expires_at = Utc::now() + Duration::minutes(LEASE_MINUTES);
            let entry = match migration_entry(&heartbeat_record) {
                Ok(entry) => entry,
                Err(_) => {
                    lost_lease.store(true, Ordering::Release);
                    return;
                }
            };
            match filesystem
                .put(&path, entry, CasExpectation::Version(*version))
                .await
            {
                Ok(next) => *version = next,
                Err(_) => {
                    lost_lease.store(true, Ordering::Release);
                    return;
                }
            }
        }
    })
}

impl<F> Drop for ReleasePairMigrationLease<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    fn drop(&mut self) {
        if self.finished_locally || self.record.status != MigrationStatus::InProgress {
            return;
        }
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let filesystem = Arc::clone(&self.filesystem);
        let path = self.path.clone();
        let version = Arc::clone(&self.version);
        let mut record = self.record.clone();
        record.status = MigrationStatus::Failed;
        record.finished_at = Some(Utc::now());
        record.report = None;
        runtime.spawn(async move {
            let Ok(entry) = migration_entry(&record) else {
                return;
            };
            let version = *version.lock().await;
            let _ = filesystem
                .put(&path, entry, CasExpectation::Version(version))
                .await;
        });
    }
}

pub(crate) fn migration_path(pair: ReleasePair) -> Result<VirtualPath, ReleasePairMigrationError> {
    virtual_path(pair.migration_path)
}

fn virtual_path(raw: &str) -> Result<VirtualPath, ReleasePairMigrationError> {
    VirtualPath::new(raw).map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))
}

pub(crate) fn migration_entry(
    record: &MigrationRecord,
) -> Result<Entry, ReleasePairMigrationError> {
    let kind = RecordKind::new("release_pair_migration")
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
    let value = serde_json::to_value(record)
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
    Entry::record(kind, &value)
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))
}

fn validate_pair(
    record: &MigrationRecord,
    pair: ReleasePair,
) -> Result<(), ReleasePairMigrationError> {
    if record.schema != pair.schema
        || record.source_release != pair.source_release
        || record.target_release != pair.target_release
    {
        return Err(ReleasePairMigrationError::UnsupportedReleasePair);
    }
    Ok(())
}
