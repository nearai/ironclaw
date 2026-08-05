//! Startup coordination for the `1.0.0-rc.1` -> `1.1.0-rc.1` release pair.
//!
//! Domain crates own persisted wire transforms. This module serializes them,
//! fingerprints the source layout, and publishes a redacted completion record
//! only after domain-level read-back verification succeeds.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::{
    ReleasePairMigrationError,
    lifecycle::{ReleasePair, ReleasePairMigrationLease},
    workspace::{
        LegacyWorkspaceMigrationInput, LegacyWorkspaceMigrationReport,
        migrate_legacy_workspace_snapshot,
    },
};
use chrono::Utc;
use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::{FileType, Filter, Page, RecordKind, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::path::VirtualPath;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_MIGRATION_TENANTS: usize = 100_000;
const MAX_CHANNEL_THREAD_REFERENCES: usize = 1_000_000;

const RC1_TO_1_1: ReleasePair = ReleasePair {
    schema: "release-pair-migration-v1",
    source_release: "1.0.0-rc.1",
    target_release: "1.1.0-rc.1",
    migration_path: "/tenants/__system__/shared/startup-migrations/1.0.0-rc.1-to-1.1.0-rc.1.json",
    domain_migration_root: "/tenants/__system__/shared/startup-migrations/1.0.0-rc.1-to-1.1.0-rc.1/domains",
    old_authorities_retained: true,
    in_place_rows_backward_readable: true,
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct SourceFingerprint {
    examined_rows: usize,
    rc1_thread_rows: usize,
    rc1_channel_rows: usize,
    rc1_process_rows: usize,
    current_migration_markers: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ChannelRootMigrationReport {
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
pub struct ChannelScopeMigrationReport {
    pub migrated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflicting: usize,
    pub failed: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionInstallationMigrationReport {
    pub sources_migrated: usize,
    pub sources_unchanged: usize,
    pub manifests_migrated: usize,
    pub manifests_unchanged: usize,
    pub installations_migrated: usize,
    pub installations_unchanged: usize,
}

/// Verified results from the release-pair migrations that do not require the
/// fully assembled extension runtime.
struct Rc1To11CoreMigration<F>
where
    F: RootFilesystem + 'static,
{
    pub oauth: ironclaw_auth::OAuthProviderAliasMigrationReport,
    pub channels: ChannelRootMigrationReport,
    pub process: ironclaw_processes::LegacyProcessMigrationReport,
    pub threads: ironclaw_threads::ThreadStartupMigrationReport,
    pub process_journal_store: Arc<ironclaw_processes::ProcessJournalStore<F>>,
}

/// Run and verify the core, cross-domain portion of the rc1 release migration.
///
/// The caller owns the database-wide lease because extension migration runs
/// later, after the extension runtime and secret authority have been assembled.
async fn migrate_core_rc1_to_1_1<F>(
    filesystem: Arc<F>,
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    process_journal_store: Arc<ironclaw_processes::ProcessJournalStore<F>>,
) -> Result<Rc1To11CoreMigration<F>, ReleasePairMigrationError>
where
    F: RootFilesystem + 'static,
{
    let oauth = ironclaw_auth::migrate_legacy_oauth_provider_alias(
        Arc::clone(&filesystem),
        Arc::clone(&scoped_filesystem),
        Utc::now(),
    )
    .await
    .map_err(|error| ReleasePairMigrationError::Domain {
        domain: "OAuth provider-alias",
        reason: error.to_string(),
    })?;
    let channels = migrate_channel_roots(filesystem.as_ref()).await?;
    let process = process_journal_store
        .migrate_legacy_journal_with_report()
        .await
        .map_err(|error| ReleasePairMigrationError::Domain {
            domain: "process journal",
            reason: error.to_string(),
        })?;
    tracing::info!(
        already_complete = process.already_complete,
        imported_journal_entries = process.imported_journal_entries,
        legacy_events_superseded = process.disposition.legacy_events_superseded,
        active_locks_expired = process.disposition.active_locks_expired,
        checkpoint_metadata_superseded = process.disposition.checkpoint_metadata_superseded,
        admission_reservations_expired = process.disposition.admission_reservations_expired,
        "process journal startup migration completed"
    );
    let threads = ironclaw_threads::migrate_all_thread_scopes(
        Arc::clone(&filesystem),
        Arc::clone(&scoped_filesystem),
    )
    .await
    .map_err(|error| ReleasePairMigrationError::Domain {
        domain: "thread",
        reason: error.to_string(),
    })?;
    tracing::info!(
        discovered_scopes = threads.discovered_scopes,
        thread_rows = threads.thread_rows,
        transcript_scopes_migrated = threads.transcript_scopes_migrated,
        transcript_scopes_unchanged = threads.transcript_scopes_unchanged,
        append_events_scanned = threads.append_events_scanned,
        append_messages_materialized = threads.append_messages_materialized,
        append_messages_unchanged = threads.append_messages_unchanged,
        transcript_rows_projected = threads.transcript_rows_projected,
        "thread startup migration completed"
    );
    validate_channel_thread_references(filesystem.as_ref(), &channels).await?;

    Ok(Rc1To11CoreMigration {
        oauth,
        channels,
        process,
        threads,
        process_journal_store,
    })
}

/// Inputs available before production runtime writers are assembled.
pub struct Rc1To11MigrationInput<F>
where
    F: RootFilesystem + 'static,
{
    pub filesystem: Arc<F>,
    pub scoped_filesystem: Arc<ScopedFilesystem<F>>,
    pub process_journal_store: Arc<ironclaw_processes::ProcessJournalStore<F>>,
    pub workspace: Option<LegacyWorkspaceMigrationInput>,
}

/// Reports produced after extension services become available.
#[derive(Default)]
pub struct Rc1To11ExtensionReports {
    pub installations: Option<ExtensionInstallationMigrationReport>,
    pub channel_state: Option<ironclaw_extension_host::Rc1ChannelStateMigrationReport>,
}

/// In-progress rc1 -> 1.1 migration barrier.
///
/// Holding this value keeps the database-wide lease alive. Completion consumes
/// it, so composition cannot accidentally reuse a completed migration session.
pub struct Rc1To11Migration<F>
where
    F: RootFilesystem + 'static,
{
    lease: ReleasePairMigrationLease<F>,
    core: Rc1To11CoreMigration<F>,
    workspace: Option<LegacyWorkspaceMigrationReport>,
}

impl<F> Rc1To11Migration<F>
where
    F: RootFilesystem + 'static,
{
    pub async fn begin(input: Rc1To11MigrationInput<F>) -> Result<Self, ReleasePairMigrationError> {
        let Rc1To11MigrationInput {
            filesystem,
            scoped_filesystem,
            process_journal_store,
            workspace,
        } = input;
        let lease = acquire_release_pair_lease(Arc::clone(&filesystem)).await?;
        let migration = async {
            let workspace = match workspace {
                Some(input) => {
                    let report = migrate_legacy_workspace_snapshot(input).await?;
                    tracing::info!(
                        directories_verified = report.directories_verified,
                        files_migrated = report.files_migrated,
                        files_unchanged = report.files_unchanged,
                        bytes_verified = report.bytes_verified,
                        "workspace artifact startup migration completed"
                    );
                    Some(report)
                }
                None => None,
            };
            let core = Box::pin(migrate_core_rc1_to_1_1(
                filesystem,
                scoped_filesystem,
                process_journal_store,
            ))
            .await?;
            Ok::<_, ReleasePairMigrationError>((core, workspace))
        }
        .await;
        match migration {
            Ok((core, workspace)) => Ok(Self {
                lease,
                core,
                workspace,
            }),
            Err(error) => {
                lease.fail_and_log().await;
                Err(error)
            }
        }
    }

    pub fn process_journal_store(&self) -> Arc<ironclaw_processes::ProcessJournalStore<F>> {
        Arc::clone(&self.core.process_journal_store)
    }

    pub async fn complete(
        self,
        extensions: Rc1To11ExtensionReports,
    ) -> Result<(), ReleasePairMigrationError> {
        let report = redacted_core_report(
            &self.core.process,
            &self.core.threads,
            &self.core.channels,
            &self.core.oauth,
            extensions.installations.as_ref(),
            extensions.channel_state.as_ref(),
            self.workspace.as_ref(),
        );
        self.lease.complete(report).await
    }

    pub async fn fail_and_log(self) {
        self.lease.fail_and_log().await;
    }
}

async fn acquire_release_pair_lease<F>(
    filesystem: Arc<F>,
) -> Result<ReleasePairMigrationLease<F>, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized + 'static,
{
    let fingerprint_filesystem = Arc::clone(&filesystem);
    ReleasePairMigrationLease::acquire(filesystem, RC1_TO_1_1, async move {
        let fingerprint = fingerprint_source(fingerprint_filesystem.as_ref()).await?;
        serde_json::to_value(fingerprint)
            .map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))
    })
    .await
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

pub async fn migrate_channel_roots<F>(
    filesystem: &F,
) -> Result<ChannelRootMigrationReport, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let tenant_segments = tenant_segments(filesystem).await?;

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
    if aggregate.referenced_threads.len() > MAX_CHANNEL_THREAD_REFERENCES {
        return Err(ReleasePairMigrationError::Domain {
            domain: "channel canonical-thread verification",
            reason: "channel thread-reference bound exceeded".to_string(),
        });
    }
    Ok(aggregate)
}

/// Discover every hosted rc1 installation snapshot. The released hosted
/// profile selected a tenant-specific authority while 1.1 selects the global
/// normalized installation root, so relying on the configured owner would
/// silently strand other tenants.
pub async fn discover_rc1_hosted_extension_snapshots<F>(
    filesystem: &F,
) -> Result<Vec<VirtualPath>, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let mut snapshots = BTreeSet::new();
    for tenant in tenant_segments(filesystem).await? {
        let path = format!("/tenants/{tenant}/system/extensions/.installations/state.json");
        let snapshot = virtual_path(&path)?;
        if filesystem.get(&snapshot).await?.is_some() {
            snapshots.insert(path);
        }
    }
    snapshots
        .into_iter()
        .map(|path| virtual_path(&path))
        .collect()
}

/// Import every hosted rc1 installation authority and return the aggregate
/// redacted report. Keeping the loop here prevents composition from owning a
/// release-specific restoration policy.
pub async fn migrate_rc1_hosted_extension_snapshots<F>(
    filesystem: &F,
    store: &mut ExtensionInstallationStore,
) -> Result<ExtensionInstallationMigrationReport, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    for snapshot in discover_rc1_hosted_extension_snapshots(filesystem).await? {
        store
            .import_rc1_snapshot_at(&snapshot)
            .await
            .map_err(|error| ReleasePairMigrationError::Domain {
                domain: "extension installation",
                reason: error.to_string(),
            })?;
    }
    let report = store.rc1_snapshot_migration_report();
    Ok(ExtensionInstallationMigrationReport {
        sources_migrated: report.sources_migrated,
        sources_unchanged: report.sources_unchanged,
        manifests_migrated: report.manifests_migrated,
        manifests_unchanged: report.manifests_unchanged,
        installations_migrated: report.installations_migrated,
        installations_unchanged: report.installations_unchanged,
    })
}

async fn tenant_segments<F>(filesystem: &F) -> Result<BTreeSet<String>, ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let tenants_root = virtual_path("/tenants")?;
    let entries = filesystem
        .list_dir_bounded(&tenants_root, MAX_MIGRATION_TENANTS.saturating_add(1))
        .await?;
    if entries.len() > MAX_MIGRATION_TENANTS {
        return Err(ReleasePairMigrationError::Domain {
            domain: "tenant discovery",
            reason: "tenant bound exceeded".to_string(),
        });
    }
    Ok(entries
        .into_iter()
        .filter(|entry| entry.file_type == FileType::Directory && entry.name != "__system__")
        .map(|entry| entry.name)
        .collect())
}

/// Verify that every channel binding still resolves to a durable canonical
/// thread after thread materialization and projection rebuilds complete.
pub async fn validate_channel_thread_references<F>(
    filesystem: &F,
    report: &ChannelRootMigrationReport,
) -> Result<(), ReleasePairMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    if report.referenced_threads.is_empty() {
        return Ok(());
    }

    let mut remaining =
        BTreeMap::<(String, String, Option<String>), BTreeSet<Option<String>>>::new();
    for reference in &report.referenced_threads {
        remaining
            .entry((
                reference.tenant_id.as_str().to_string(),
                reference.thread_id.as_str().to_string(),
                reference
                    .project_id
                    .as_ref()
                    .map(|value| value.as_str().to_string()),
            ))
            .or_default()
            .insert(
                reference
                    .agent_id
                    .as_ref()
                    .map(|value| value.as_str().to_string()),
            );
    }
    let referenced_tenants = remaining
        .keys()
        .map(|(tenant, _, _)| tenant.clone())
        .collect::<BTreeSet<_>>();
    for tenant in referenced_tenants {
        let tenant_root = virtual_path(&format!("/tenants/{tenant}"))?;
        let mut offset = 0u64;
        loop {
            let rows = filesystem
                .query(
                    &tenant_root,
                    &Filter::All,
                    Page::new(offset, Page::MAX_LIMIT),
                )
                .await?;
            if rows.is_empty() {
                break;
            }
            let received = rows.len();
            for row in rows {
                if !row.path.as_str().ends_with("/thread.json")
                    || row.entry.kind.as_ref().map(RecordKind::as_str) != Some("session_thread")
                {
                    continue;
                }
                let header = serde_json::from_slice::<ironclaw_threads::SessionThreadRecord>(
                    &row.entry.body,
                )
                .map_err(|error| ReleasePairMigrationError::Domain {
                    domain: "channel canonical-thread verification",
                    reason: format!("malformed canonical thread header: {error}"),
                })?;
                let key = (
                    header.scope.tenant_id.as_str().to_string(),
                    header.thread_id.as_str().to_string(),
                    header
                        .scope
                        .project_id
                        .as_ref()
                        .map(|value| value.as_str().to_string()),
                );
                if let Some(agents) = remaining.get_mut(&key) {
                    agents.remove(&None);
                    agents.remove(&Some(header.scope.agent_id.as_str().to_string()));
                    if agents.is_empty() {
                        remaining.remove(&key);
                    }
                }
            }
            if received < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.saturating_add(received as u64);
        }
    }

    if !remaining.is_empty() {
        return Err(ReleasePairMigrationError::Domain {
            domain: "channel canonical-thread verification",
            reason: "a migrated channel binding references a missing canonical thread".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn virtual_path(raw: &str) -> Result<VirtualPath, ReleasePairMigrationError> {
    VirtualPath::new(raw).map_err(|error| ReleasePairMigrationError::Malformed(error.to_string()))
}

fn redacted_core_report(
    process: &ironclaw_processes::LegacyProcessMigrationReport,
    threads: &ironclaw_threads::ThreadStartupMigrationReport,
    channels: &ChannelRootMigrationReport,
    oauth: &ironclaw_auth::OAuthProviderAliasMigrationReport,
    installations: Option<&ExtensionInstallationMigrationReport>,
    extension_state: Option<&ironclaw_extension_host::Rc1ChannelStateMigrationReport>,
    workspace: Option<&LegacyWorkspaceMigrationReport>,
) -> Value {
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
    let mut report = json!({
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
        }
    });
    if let (Value::Object(domains), Some(installations)) = (&mut report, installations) {
        domains.insert(
            "extension_installations".to_string(),
            json!({
                "migrated": installations.manifests_migrated
                    .saturating_add(installations.installations_migrated),
                "unchanged": installations.manifests_unchanged
                    .saturating_add(installations.installations_unchanged),
                "skipped": 0,
                "conflicting": 0,
                "failed": 0,
                "sources_migrated": installations.sources_migrated,
                "sources_unchanged": installations.sources_unchanged,
            }),
        );
    }
    if let (Value::Object(domains), Some(workspace)) = (&mut report, workspace) {
        domains.insert(
            "workspace_artifacts".to_string(),
            json!({
                "migrated": workspace.files_migrated,
                "unchanged": workspace.files_unchanged,
                "skipped": 0,
                "conflicting": 0,
                "failed": 0,
                "directories_verified": workspace.directories_verified,
                "bytes_verified": workspace.bytes_verified,
                "source_retained": true,
            }),
        );
    }
    if let (Value::Object(domains), Some(extension_state)) = (&mut report, extension_state) {
        let extension_state_scopes = extension_state
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
        domains.insert("channel_extension_state".to_string(), json!({
            "migrated": extension_state.configuration_values
                .saturating_add(extension_state.identities)
                .saturating_add(extension_state.route_values)
                .saturating_add(extension_state.dm_targets),
            "unchanged": extension_state.oauth_channel_connections_unchanged
                .saturating_add(extension_state.proof_code_pairing_rows_unchanged),
            "skipped": extension_state.proof_code_pairing_challenges_expired
                .saturating_add(extension_state.proof_code_pending_completions_expired)
                .saturating_add(extension_state.unbound_dm_targets_skipped)
                .saturating_add(extension_state.oauth_channel_stale_connections_expired)
                .saturating_add(extension_state.oauth_channel_active_connections_superseded)
                .saturating_add(extension_state.oauth_channel_disconnected_connections_superseded),
            "conflicting": 0,
            "failed": 0,
            "configuration_values": extension_state.configuration_values,
            "identities": extension_state.identities,
            "route_values": extension_state.route_values,
            "dm_targets": extension_state.dm_targets,
            "unbound_dm_targets_skipped": extension_state.unbound_dm_targets_skipped,
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
            "scopes": extension_state_scopes,
        }));
    }
    report
}

#[cfg(test)]
mod tests;
