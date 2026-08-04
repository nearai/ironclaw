//! Startup coordination for the `1.0.0-rc.1` -> `1.1.0-rc.1` release pair.
//!
//! Domain crates own persisted wire transforms. This module serializes them,
//! fingerprints the source layout, and publishes a redacted completion record
//! only after domain-level read-back verification succeeds.

use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{Duration, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, Page, RecordKind, RecordVersion, RootFilesystem,
};
use ironclaw_host_api::path::VirtualPath;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

const SOURCE_RELEASE: &str = "1.0.0-rc.1";
const TARGET_RELEASE: &str = "1.1.0-rc.1";
const MIGRATION_SCHEMA: &str = "release-pair-migration-v1";
const MIGRATION_PATH: &str =
    "/tenants/__system__/shared/startup-migrations/1.0.0-rc.1-to-1.1.0-rc.1.json";
const DOMAIN_MIGRATION_ROOT: &str =
    "/tenants/__system__/shared/startup-migrations/1.0.0-rc.1-to-1.1.0-rc.1/domains";
const LEASE_MINUTES: i64 = 10;
const HEARTBEAT_SECONDS: u64 = 60;
const ACQUIRE_RETRIES: usize = 8;

#[derive(Debug, Error)]
pub(crate) enum ReleasePairMigrationError {
    #[error("filesystem error: {0}")]
    Filesystem(#[from] FilesystemError),
    #[error("release-pair migration record is malformed: {0}")]
    Malformed(String),
    #[error("release-pair migration is already running in another process")]
    ConcurrentStartup,
    #[error("release-pair migration source/target does not match this binary")]
    UnsupportedReleasePair,
    #[error("release-pair migration lost its database-wide lease")]
    LostLease,
    #[error("{domain} startup migration failed: {reason}")]
    Domain {
        domain: &'static str,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MigrationStatus {
    InProgress,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MigrationRecord {
    schema: String,
    source_release: String,
    target_release: String,
    status: MigrationStatus,
    attempt_id: String,
    started_at: chrono::DateTime<Utc>,
    lease_expires_at: chrono::DateTime<Utc>,
    finished_at: Option<chrono::DateTime<Utc>>,
    source_fingerprint: SourceFingerprint,
    /// Counts/status only: no paths, actor ids, payloads, or secret handles.
    report: Option<Value>,
    rollback: RollbackDisposition,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct SourceFingerprint {
    examined_rows: usize,
    rc1_thread_rows: usize,
    rc1_channel_rows: usize,
    rc1_process_rows: usize,
    current_migration_markers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RollbackDisposition {
    old_authorities_retained: bool,
    in_place_rows_backward_readable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DomainCompletionRecord {
    schema: String,
    source_release: String,
    target_release: String,
    domain: String,
    status: MigrationStatus,
    completed_at: chrono::DateTime<Utc>,
    /// Counts/status only; the domain report contains no raw paths, actor ids,
    /// payloads, credentials, or secret handles.
    report: Value,
}

pub(crate) struct ReleasePairMigrationLease<F: ?Sized>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<F>,
    path: VirtualPath,
    version: Arc<tokio::sync::Mutex<RecordVersion>>,
    lost_lease: Arc<AtomicBool>,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
    record: MigrationRecord,
    finished_locally: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ChannelRootMigrationReport {
    pub tenants: usize,
    pub conversation_source_items: usize,
    pub conversation_items_migrated: usize,
    pub conversation_items_unchanged: usize,
    pub idempotency_actions_scanned: usize,
    pub idempotency_actions_migrated: usize,
    pub idempotency_actions_unchanged: usize,
    pub idempotency_transient_leases_expired: usize,
    /// Typed cross-domain evidence used before the startup barrier opens. Raw
    /// identifiers are never copied into the persisted redacted report.
    pub referenced_threads: Vec<ironclaw_conversations::ConversationThreadReference>,
    pub scopes: Vec<ChannelScopeMigrationReport>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChannelScopeMigrationReport {
    pub migrated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflicting: usize,
    pub failed: usize,
}

impl<F> ReleasePairMigrationLease<F>
where
    F: RootFilesystem + ?Sized + 'static,
{
    pub(crate) async fn acquire(filesystem: Arc<F>) -> Result<Self, ReleasePairMigrationError> {
        let path = migration_path()?;
        for _ in 0..ACQUIRE_RETRIES {
            let existing = filesystem.get(&path).await?;
            let now = Utc::now();
            let fingerprint = fingerprint_source(filesystem.as_ref()).await?;
            let (cas, was_complete) = match existing {
                Some(versioned) => {
                    let prior: MigrationRecord = serde_json::from_slice(&versioned.entry.body)
                        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
                    validate_pair(&prior)?;
                    if prior.status == MigrationStatus::InProgress && prior.lease_expires_at > now {
                        return Err(ReleasePairMigrationError::ConcurrentStartup);
                    }
                    (
                        CasExpectation::Version(versioned.version),
                        prior.status == MigrationStatus::Complete,
                    )
                }
                None => (CasExpectation::Absent, false),
            };
            let record = MigrationRecord {
                schema: MIGRATION_SCHEMA.to_string(),
                source_release: SOURCE_RELEASE.to_string(),
                target_release: TARGET_RELEASE.to_string(),
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
                    old_authorities_retained: true,
                    in_place_rows_backward_readable: true,
                },
            };
            match filesystem.put(&path, migration_entry(&record)?, cas).await {
                Ok(version) => {
                    if was_complete {
                        tracing::debug!(
                            source_release = SOURCE_RELEASE,
                            target_release = TARGET_RELEASE,
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
        self.write_domain_completion_records(&report).await?;
        self.stop_heartbeat();
        self.record.status = MigrationStatus::Complete;
        self.record.finished_at = Some(Utc::now());
        self.record.report = Some(report);
        self.replace().await?;
        self.finished_locally = true;
        Ok(())
    }

    pub(crate) async fn fail(mut self) -> Result<(), ReleasePairMigrationError> {
        self.stop_heartbeat();
        self.record.status = MigrationStatus::Failed;
        self.record.finished_at = Some(Utc::now());
        self.record.report = None;
        self.replace().await?;
        self.finished_locally = true;
        Ok(())
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

    fn stop_heartbeat(&mut self) {
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
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
                "{DOMAIN_MIGRATION_ROOT}/{domain}-v1.complete.json"
            ))?;
            let record = DomainCompletionRecord {
                schema: "release-pair-domain-completion-v1".to_string(),
                source_release: SOURCE_RELEASE.to_string(),
                target_release: TARGET_RELEASE.to_string(),
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
                    let existing: DomainCompletionRecord =
                        serde_json::from_slice(&existing.entry.body).map_err(|error| {
                            ReleasePairMigrationError::Malformed(error.to_string())
                        })?;
                    if existing.schema != record.schema
                        || existing.source_release != SOURCE_RELEASE
                        || existing.target_release != TARGET_RELEASE
                        || existing.domain != *domain
                        || existing.status != MigrationStatus::Complete
                    {
                        return Err(ReleasePairMigrationError::UnsupportedReleasePair);
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
        self.stop_heartbeat();
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

fn migration_path() -> Result<VirtualPath, ReleasePairMigrationError> {
    VirtualPath::new(MIGRATION_PATH)
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))
}

fn migration_entry(record: &MigrationRecord) -> Result<Entry, ReleasePairMigrationError> {
    let kind = RecordKind::new("release_pair_migration")
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
    let value = serde_json::to_value(record)
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
    Entry::record(kind, &value)
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))
}

fn validate_pair(record: &MigrationRecord) -> Result<(), ReleasePairMigrationError> {
    if record.schema != MIGRATION_SCHEMA
        || record.source_release != SOURCE_RELEASE
        || record.target_release != TARGET_RELEASE
    {
        return Err(ReleasePairMigrationError::UnsupportedReleasePair);
    }
    Ok(())
}

async fn fingerprint_source<F>(
    filesystem: &F,
) -> Result<SourceFingerprint, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let tenants = VirtualPath::new("/tenants")
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
    let mut fingerprint = SourceFingerprint::default();
    let mut offset = 0u64;
    loop {
        let rows = filesystem
            .query(&tenants, &Filter::All, Page::new(offset, Page::MAX_LIMIT))
            .await?;
        if rows.is_empty() {
            break;
        }
        let received = rows.len();
        fingerprint.examined_rows = fingerprint.examined_rows.saturating_add(received);
        for row in rows {
            let path = row.path.as_str();
            if path.ends_with("/thread.json") && path.contains("/threads/agents/") {
                fingerprint.rc1_thread_rows = fingerprint.rc1_thread_rows.saturating_add(1);
            }
            if ironclaw_extension_host::is_rc1_channel_state_path(path) {
                fingerprint.rc1_channel_rows = fingerprint.rc1_channel_rows.saturating_add(1);
            }
            if path.contains("/turns/")
                || path.contains("/run-state/")
                || path.contains("/checkpoint-state/")
            {
                fingerprint.rc1_process_rows = fingerprint.rc1_process_rows.saturating_add(1);
            }
            if path.contains("/index-migrations/") || path.contains("/startup-migrations/") {
                fingerprint.current_migration_markers =
                    fingerprint.current_migration_markers.saturating_add(1);
            }
        }
        if received < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(received as u64);
    }
    Ok(fingerprint)
}

pub(crate) async fn migrate_channel_roots<F>(
    filesystem: &F,
) -> Result<ChannelRootMigrationReport, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let tenants_root = VirtualPath::new("/tenants")
        .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))?;
    let mut tenant_segments = BTreeSet::new();
    let mut offset = 0u64;
    loop {
        let rows = filesystem
            .query(
                &tenants_root,
                &Filter::All,
                Page::new(offset, Page::MAX_LIMIT),
            )
            .await?;
        if rows.is_empty() {
            break;
        }
        let received = rows.len();
        for row in rows {
            let path = row.path.as_str();
            if !ironclaw_extension_host::is_rc1_channel_state_path(path) {
                continue;
            }
            if let Some(segment) = path
                .strip_prefix("/tenants/")
                .and_then(|rest| rest.split('/').next())
                && !segment.is_empty()
            {
                tenant_segments.insert(segment.to_string());
            }
        }
        if received < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(received as u64);
    }

    let mut aggregate = ChannelRootMigrationReport {
        tenants: tenant_segments.len(),
        ..ChannelRootMigrationReport::default()
    };
    for tenant in tenant_segments {
        for spec in ironclaw_extension_host::rc1_channel_root_migration_specs() {
            let extension = spec.provider_key;
            let source_conversations = virtual_path(&format!(
                "/tenants/{tenant}/shared/{extension}-conversations"
            ))?;
            let target_conversations = virtual_path(&format!(
                "/tenants/{tenant}/shared/channel-extensions/{extension}/conversations"
            ))?;
            let conversation = ironclaw_conversations::migrate_conversation_state_root(
                filesystem,
                &source_conversations,
                &target_conversations,
            )
            .await
            .map_err(|error| ReleasePairMigrationError::Domain {
                domain: "channel conversations",
                reason: error.to_string(),
            })?;
            aggregate.conversation_source_items = aggregate
                .conversation_source_items
                .saturating_add(conversation.source_items);
            aggregate.conversation_items_migrated = aggregate
                .conversation_items_migrated
                .saturating_add(conversation.inserted_items);
            aggregate.conversation_items_unchanged = aggregate
                .conversation_items_unchanged
                .saturating_add(conversation.unchanged_items);
            aggregate
                .referenced_threads
                .extend(conversation.referenced_threads);

            let source_idempotency = virtual_path(&format!(
                "/tenants/{tenant}/shared/{extension}-product-workflow/idempotency"
            ))?;
            let target_idempotency = virtual_path(&format!(
                "/tenants/{tenant}/shared/channel-extensions/{extension}/product-workflow/idempotency"
            ))?;
            let idempotency = ironclaw_product::migrate_idempotency_ledger_root(
                filesystem,
                &source_idempotency,
                &target_idempotency,
            )
            .await
            .map_err(|error| ReleasePairMigrationError::Domain {
                domain: "channel idempotency",
                reason: error.to_string(),
            })?;
            aggregate.idempotency_actions_scanned = aggregate
                .idempotency_actions_scanned
                .saturating_add(idempotency.scanned_actions);
            aggregate.idempotency_actions_migrated = aggregate
                .idempotency_actions_migrated
                .saturating_add(idempotency.migrated_actions);
            aggregate.idempotency_actions_unchanged = aggregate
                .idempotency_actions_unchanged
                .saturating_add(idempotency.unchanged_actions);
            aggregate.idempotency_transient_leases_expired = aggregate
                .idempotency_transient_leases_expired
                .saturating_add(idempotency.skipped_transient_leases);
            aggregate.scopes.push(ChannelScopeMigrationReport {
                migrated: conversation
                    .inserted_items
                    .saturating_add(idempotency.migrated_actions),
                unchanged: conversation
                    .unchanged_items
                    .saturating_add(idempotency.unchanged_actions),
                skipped: idempotency.skipped_transient_leases,
                conflicting: 0,
                failed: 0,
            });
        }
    }
    aggregate.referenced_threads.sort_by(|left, right| {
        (
            left.tenant_id.as_str(),
            left.thread_id.as_str(),
            left.agent_id.as_ref().map(|value| value.as_str()),
            left.project_id.as_ref().map(|value| value.as_str()),
        )
            .cmp(&(
                right.tenant_id.as_str(),
                right.thread_id.as_str(),
                right.agent_id.as_ref().map(|value| value.as_str()),
                right.project_id.as_ref().map(|value| value.as_str()),
            ))
    });
    aggregate.referenced_threads.dedup();
    Ok(aggregate)
}

/// Discover every hosted rc1 installation snapshot. The released hosted
/// profile selected a tenant-specific authority while 1.1 selects the global
/// normalized installation root, so relying on the configured owner would
/// silently strand other tenants.
pub(crate) async fn discover_rc1_hosted_extension_snapshots<F>(
    filesystem: &F,
) -> Result<Vec<VirtualPath>, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let tenants_root = virtual_path("/tenants")?;
    let mut snapshots = BTreeSet::new();
    let mut offset = 0u64;
    loop {
        let rows = filesystem
            .query(
                &tenants_root,
                &Filter::All,
                Page::new(offset, Page::MAX_LIMIT),
            )
            .await?;
        if rows.is_empty() {
            break;
        }
        let received = rows.len();
        for row in rows {
            let path = row.path.as_str();
            if path.starts_with("/tenants/")
                && path.ends_with("/system/extensions/.installations/state.json")
            {
                snapshots.insert(path.to_string());
            }
        }
        if received < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(received as u64);
    }
    snapshots
        .into_iter()
        .map(|path| virtual_path(&path))
        .collect()
}

/// Verify that every channel binding still resolves to a durable canonical
/// thread after thread materialization and projection rebuilds complete.
pub(crate) async fn validate_channel_thread_references<F>(
    filesystem: &F,
    report: &ChannelRootMigrationReport,
) -> Result<(), ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    if report.referenced_threads.is_empty() {
        return Ok(());
    }

    let tenants_root = virtual_path("/tenants")?;
    let mut headers = Vec::new();
    let mut offset = 0u64;
    loop {
        let rows = filesystem
            .query(
                &tenants_root,
                &Filter::All,
                Page::new(offset, Page::MAX_LIMIT),
            )
            .await?;
        if rows.is_empty() {
            break;
        }
        let received = rows.len();
        for row in rows {
            if !row.path.as_str().ends_with("/thread.json") {
                continue;
            }
            let header =
                serde_json::from_slice::<ironclaw_threads::SessionThreadRecord>(&row.entry.body)
                    .map_err(|error| ReleasePairMigrationError::Domain {
                        domain: "channel canonical-thread verification",
                        reason: format!("malformed canonical thread header: {error}"),
                    })?;
            headers.push(header);
        }
        if received < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(received as u64);
    }

    for reference in &report.referenced_threads {
        let found = headers.iter().any(|header| {
            header.scope.tenant_id == reference.tenant_id
                && header.thread_id == reference.thread_id
                && reference
                    .agent_id
                    .as_ref()
                    .is_none_or(|agent| header.scope.agent_id == *agent)
                && header.scope.project_id == reference.project_id
        });
        if !found {
            return Err(ReleasePairMigrationError::Domain {
                domain: "channel canonical-thread verification",
                reason: "a migrated channel binding references a missing canonical thread"
                    .to_string(),
            });
        }
    }
    Ok(())
}

fn virtual_path(raw: &str) -> Result<VirtualPath, ReleasePairMigrationError> {
    VirtualPath::new(raw).map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))
}

pub(crate) fn redacted_core_report(
    process: &ironclaw_processes::LegacyProcessMigrationReport,
    threads: &ironclaw_threads::ThreadStartupMigrationReport,
    channels: &ChannelRootMigrationReport,
    oauth: &ironclaw_auth::OAuthProviderAliasMigrationReport,
    installations: Option<&ironclaw_extensions::Rc1SnapshotMigrationReport>,
    extension_state: Option<&ironclaw_extension_host::Rc1ChannelStateMigrationReport>,
) -> Value {
    let installations = installations.copied().unwrap_or_default();
    let extension_state = extension_state.cloned().unwrap_or_default();
    let thread_scopes = threads
        .scopes
        .iter()
        .map(|scope| {
            json!({
                "migrated": scope.migrated,
                "unchanged": scope.unchanged,
                "skipped": scope.skipped,
                "conflicting": scope.conflicting,
                "failed": scope.failed,
                "append_events_scanned": scope.append_events_scanned,
                "append_messages_materialized": scope.append_messages_materialized,
                "append_messages_unchanged": scope.append_messages_unchanged,
                "transcript_rows_projected": scope.transcript_rows_projected,
            })
        })
        .collect::<Vec<_>>();
    let channel_scopes = channels
        .scopes
        .iter()
        .map(|scope| {
            json!({
                "migrated": scope.migrated,
                "unchanged": scope.unchanged,
                "skipped": scope.skipped,
                "conflicting": scope.conflicting,
                "failed": scope.failed,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "processes": {
            "already_complete": process.already_complete,
            "migrated": process.imported_journal_entries,
            "unchanged": usize::from(process.already_complete),
            "skipped": 0,
            "conflicting": 0,
            "failed": 0,
            "legacy_events_superseded": process.disposition.legacy_events_superseded,
            "active_locks_expired": process.disposition.active_locks_expired,
            "checkpoint_metadata_superseded": process.disposition.checkpoint_metadata_superseded,
            "admission_reservations_expired": process.disposition.admission_reservations_expired,
        },
        "threads": {
            "migrated": threads.append_messages_materialized
                .saturating_add(threads.transcript_rows_projected),
            "unchanged": threads.append_messages_unchanged
                .saturating_add(threads.transcript_scopes_unchanged),
            "skipped": 0,
            "conflicting": 0,
            "failed": 0,
            "discovered_scopes": threads.discovered_scopes,
            "thread_rows": threads.thread_rows,
            "scopes": thread_scopes,
        },
        "channel_conversations": {
            "migrated": channels.conversation_items_migrated,
            "unchanged": channels.conversation_items_unchanged,
            "skipped": 0,
            "conflicting": 0,
            "failed": 0,
            "source_items": channels.conversation_source_items,
            "tenants": channels.tenants,
            "scopes": channel_scopes,
        },
        "channel_idempotency": {
            "migrated": channels.idempotency_actions_migrated,
            "unchanged": channels.idempotency_actions_unchanged,
            "skipped": channels.idempotency_transient_leases_expired,
            "conflicting": 0,
            "failed": 0,
            "scanned": channels.idempotency_actions_scanned,
        },
        "oauth_provider_alias": {
            "migrated": oauth.account_rows_migrated.saturating_add(oauth.flow_rows_migrated),
            "unchanged": oauth.account_rows_already_current
                .saturating_add(oauth.flow_rows_already_current),
            "skipped": 0,
            "conflicting": 0,
            "failed": 0,
            "expired_flows": oauth.incomplete_flows_expired,
            "backups_created": oauth.rollback_backups_created,
            "backups_reused": oauth.rollback_backups_reused,
        },
        "extension_installations": {
            "migrated": installations.manifests_migrated
                .saturating_add(installations.installations_migrated),
            "unchanged": installations.manifests_unchanged
                .saturating_add(installations.installations_unchanged),
            "skipped": 0,
            "conflicting": 0,
            "failed": 0,
            "sources_migrated": installations.sources_migrated,
            "sources_unchanged": installations.sources_unchanged,
        },
        "channel_extension_state": {
            "migrated": extension_state.configuration_values
                .saturating_add(extension_state.identities)
                .saturating_add(extension_state.route_values)
                .saturating_add(extension_state.dm_targets),
            "unchanged": 0,
            "skipped": extension_state.proof_code_pairing_challenges_expired
                .saturating_add(extension_state.proof_code_pending_completions_expired)
                .saturating_add(extension_state.oauth_channel_stale_connections_expired)
                .saturating_add(extension_state.oauth_channel_active_connections_superseded)
                .saturating_add(extension_state.oauth_channel_disconnected_connections_superseded),
            "conflicting": 0,
            "failed": 0,
            "configuration_values": extension_state.configuration_values,
            "identities": extension_state.identities,
            "route_values": extension_state.route_values,
            "dm_targets": extension_state.dm_targets,
            "oauth_channel_connections_unchanged": extension_state.oauth_channel_connections_unchanged,
            "oauth_channel_active_connections_superseded": extension_state
                .oauth_channel_active_connections_superseded,
            "oauth_channel_stale_connections_expired": extension_state
                .oauth_channel_stale_connections_expired,
            "oauth_channel_disconnected_connections_superseded": extension_state
                .oauth_channel_disconnected_connections_superseded,
            "proof_code_pairing_challenges_expired": extension_state
                .proof_code_pairing_challenges_expired,
            "proof_code_pending_completions_expired": extension_state
                .proof_code_pending_completions_expired,
            "proof_code_pairing_rows_unchanged": extension_state
                .proof_code_pairing_rows_unchanged,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ids::{AgentId, TenantId, ThreadId};
    use ironclaw_threads::{SessionThreadRecord, ThreadScope};

    #[tokio::test]
    async fn database_wide_lease_fails_concurrent_startup_and_allows_failed_retry() {
        let backend = Arc::new(InMemoryBackend::new());
        let first = ReleasePairMigrationLease::acquire(Arc::clone(&backend))
            .await
            .expect("first startup acquires lease");
        let second = match ReleasePairMigrationLease::acquire(Arc::clone(&backend)).await {
            Ok(_) => panic!("concurrent startup must fail closed"),
            Err(error) => error,
        };
        assert!(matches!(
            second,
            ReleasePairMigrationError::ConcurrentStartup
        ));

        first.fail().await.expect("failed attempt releases lease");
        let retry = ReleasePairMigrationLease::acquire(Arc::clone(&backend))
            .await
            .expect("failed startup is immediately retryable");
        retry
            .complete(json!({"threads": {"migrated": 1}}))
            .await
            .expect("retry publishes completion");

        let verify = ReleasePairMigrationLease::acquire(Arc::clone(&backend))
            .await
            .expect("completed migration can be reverified on restart");
        verify
            .complete(json!({"threads": {"migrated": 0, "unchanged": 1}}))
            .await
            .expect("reverification publishes zero-change report");
        let stored = backend
            .get(&migration_path().expect("migration path"))
            .await
            .expect("read completion")
            .expect("completion exists");
        let record: MigrationRecord =
            serde_json::from_slice(&stored.entry.body).expect("decode completion");
        assert_eq!(record.status, MigrationStatus::Complete);
        assert_eq!(
            record.report,
            Some(json!({"threads": {"migrated": 0, "unchanged": 1}}))
        );
        assert!(record.rollback.old_authorities_retained);

        let domain_path =
            virtual_path(&format!("{DOMAIN_MIGRATION_ROOT}/threads-v1.complete.json"))
                .expect("domain completion path");
        let domain = backend
            .get(&domain_path)
            .await
            .expect("read domain completion")
            .expect("thread domain completion exists");
        let domain: DomainCompletionRecord =
            serde_json::from_slice(&domain.entry.body).expect("decode domain completion");
        assert_eq!(domain.domain, "threads");
        assert_eq!(domain.status, MigrationStatus::Complete);
        assert_eq!(domain.report, json!({"migrated": 1}));
    }

    #[tokio::test]
    async fn unsupported_release_pair_fails_before_replacing_record() {
        let backend = Arc::new(InMemoryBackend::new());
        let now = Utc::now();
        let incompatible = MigrationRecord {
            schema: MIGRATION_SCHEMA.to_string(),
            source_release: "0.29.0".to_string(),
            target_release: TARGET_RELEASE.to_string(),
            status: MigrationStatus::Complete,
            attempt_id: "incompatible".to_string(),
            started_at: now,
            lease_expires_at: now,
            finished_at: Some(now),
            source_fingerprint: SourceFingerprint::default(),
            report: None,
            rollback: RollbackDisposition {
                old_authorities_retained: true,
                in_place_rows_backward_readable: true,
            },
        };
        backend
            .put(
                &migration_path().expect("migration path"),
                migration_entry(&incompatible).expect("migration entry"),
                CasExpectation::Absent,
            )
            .await
            .expect("seed incompatible record");

        let error = match ReleasePairMigrationLease::acquire(backend).await {
            Ok(_) => panic!("unsupported release pair must fail closed"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ReleasePairMigrationError::UnsupportedReleasePair
        ));
    }

    #[tokio::test]
    async fn dropped_startup_lease_is_failed_for_immediate_retry() {
        let backend = Arc::new(InMemoryBackend::new());
        let lease = ReleasePairMigrationLease::acquire(Arc::clone(&backend))
            .await
            .expect("startup acquires lease");
        drop(lease);

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let Some(row) = backend
                    .get(&migration_path().expect("migration path"))
                    .await
                    .expect("read lease")
                {
                    let record: MigrationRecord =
                        serde_json::from_slice(&row.entry.body).expect("decode lease");
                    if record.status == MigrationStatus::Failed {
                        break;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("drop guard publishes failure");

        let retry = ReleasePairMigrationLease::acquire(backend)
            .await
            .expect("retry does not wait for the prior lease timeout");
        retry.fail().await.expect("release retry lease");
    }

    #[tokio::test]
    async fn channel_reference_barrier_requires_the_same_canonical_thread() {
        let backend = InMemoryBackend::new();
        let tenant = TenantId::new("tenant-a").expect("tenant");
        let agent = AgentId::new("agent-a").expect("agent");
        let thread = ThreadId::new("thread-a").expect("thread");
        let report = ChannelRootMigrationReport {
            referenced_threads: vec![ironclaw_conversations::ConversationThreadReference {
                tenant_id: tenant.clone(),
                thread_id: thread.clone(),
                agent_id: Some(agent.clone()),
                project_id: None,
            }],
            ..ChannelRootMigrationReport::default()
        };

        let missing = validate_channel_thread_references(&backend, &report)
            .await
            .expect_err("missing canonical thread must fail the startup barrier");
        assert!(matches!(missing, ReleasePairMigrationError::Domain { .. }));

        let header = SessionThreadRecord {
            scope: ThreadScope {
                tenant_id: tenant,
                agent_id: agent,
                project_id: None,
                owner_user_id: None,
                mission_id: None,
            },
            thread_id: thread,
            created_by_actor_id: "actor-a".to_string(),
            title: None,
            metadata_json: None,
            goal: None,
            created_at: None,
            updated_at: None,
        };
        let path = virtual_path(
            "/tenants/tenant-a/users/__system__/threads/agents/agent-a/owners/__system__/thread-a/thread.json",
        )
        .expect("header path");
        let kind = RecordKind::new("thread").expect("thread kind");
        let header = serde_json::to_value(header).expect("serialize header");
        backend
            .put(
                &path,
                Entry::record(kind, &header).expect("header entry"),
                CasExpectation::Absent,
            )
            .await
            .expect("seed header");

        validate_channel_thread_references(&backend, &report)
            .await
            .expect("matching canonical thread opens barrier");
    }
}
