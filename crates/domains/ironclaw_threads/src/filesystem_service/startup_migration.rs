//! Eager discovery and migration of every durable thread scope.
//!
//! Composition calls this after the root filesystem is open and before any
//! runtime writer starts. The thread crate owns both the persisted path grammar
//! and the exact wire reader; composition only sequences the migration.

use std::{collections::BTreeMap, sync::Arc};

use ironclaw_filesystem::{Filter, Page, RecordKind, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::path::VirtualPath;

use crate::{FilesystemSessionThreadService, SessionThreadError};

use super::{
    SESSION_THREAD_KIND, StoredThreadRecord, deserialize, scope_axes_string, thread_record_path,
};

/// Redacted aggregate for one eager startup pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ThreadStartupMigrationReport {
    pub examined_rows: usize,
    pub thread_rows: usize,
    pub discovered_scopes: usize,
    pub transcript_scopes_migrated: usize,
    pub transcript_scopes_unchanged: usize,
    pub append_events_scanned: usize,
    pub append_messages_materialized: usize,
    pub append_messages_unchanged: usize,
    pub transcript_rows_projected: usize,
    /// One redacted entry per discovered thread scope. Scope identities are
    /// deliberately omitted from startup telemetry.
    pub scopes: Vec<ThreadScopeMigrationReport>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ThreadScopeMigrationReport {
    pub migrated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub conflicting: usize,
    pub failed: usize,
    pub append_events_scanned: usize,
    pub append_messages_materialized: usize,
    pub append_messages_unchanged: usize,
    pub transcript_rows_projected: usize,
}

/// Discover every scope from authoritative `thread.json` records and migrate
/// it before live traffic starts.
pub async fn migrate_all_thread_scopes<F>(
    root: Arc<F>,
    scoped: Arc<ScopedFilesystem<F>>,
) -> Result<ThreadStartupMigrationReport, SessionThreadError>
where
    F: RootFilesystem + 'static,
{
    let tenants_root = VirtualPath::new("/tenants").map_err(|error| {
        SessionThreadError::Backend(format!("invalid thread discovery root: {error}"))
    })?;
    let mut offset = 0u64;
    let mut scopes = BTreeMap::new();
    let mut report = ThreadStartupMigrationReport::default();

    loop {
        let rows = root
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
        report.examined_rows = report.examined_rows.saturating_add(received);
        for row in rows {
            if !looks_like_thread_record_path(row.path.as_str()) {
                continue;
            }
            if row.entry.kind.as_ref().map(RecordKind::as_str) != Some(SESSION_THREAD_KIND) {
                return Err(SessionThreadError::Backend(
                    "thread discovery found a thread record with the wrong kind".to_string(),
                ));
            }
            let stored = deserialize::<StoredThreadRecord>(&row.entry.body)?;
            let relative = thread_record_path(&stored.record.scope, &stored.record.thread_id)?;
            let expected = scoped.resolve(&stored.record.scope.to_resource_scope(), &relative)?;
            if expected != row.path {
                return Err(SessionThreadError::Backend(
                    "thread record scope does not match its durable path".to_string(),
                ));
            }
            report.thread_rows = report.thread_rows.saturating_add(1);
            // `scope_axes_string` is intentionally alias-relative and omits
            // tenant identity. Startup scans the deployment-wide root, so its
            // dedup key must restore that axis or equal agent/project/owner
            // tuples from two tenants collapse and only one gets migrated.
            let scope_key = format!(
                "{}:{}",
                stored.record.scope.tenant_id.as_str(),
                scope_axes_string(&stored.record.scope)
            );
            scopes.insert(scope_key, stored.record.scope);
        }
        if received < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(received as u64);
    }

    report.discovered_scopes = scopes.len();
    let service = FilesystemSessionThreadService::new(scoped);
    for scope in scopes.into_values() {
        let transcript = service.migrate_transcript_for_scope(&scope).await?;
        let scope_migrated = transcript
            .append
            .materialized
            .saturating_add(transcript.projected_rows);
        let scope_unchanged = transcript
            .append
            .unchanged
            .saturating_add(usize::from(transcript.already_complete));
        if transcript.already_complete {
            report.transcript_scopes_unchanged =
                report.transcript_scopes_unchanged.saturating_add(1);
        } else {
            report.transcript_scopes_migrated = report.transcript_scopes_migrated.saturating_add(1);
        }
        report.append_events_scanned = report
            .append_events_scanned
            .saturating_add(transcript.append.scanned);
        report.append_messages_materialized = report
            .append_messages_materialized
            .saturating_add(transcript.append.materialized);
        report.append_messages_unchanged = report
            .append_messages_unchanged
            .saturating_add(transcript.append.unchanged);
        report.transcript_rows_projected = report
            .transcript_rows_projected
            .saturating_add(transcript.projected_rows);
        service.ensure_thread_index_query(&scope, true).await?;
        report.scopes.push(ThreadScopeMigrationReport {
            migrated: scope_migrated,
            unchanged: scope_unchanged,
            skipped: 0,
            conflicting: 0,
            failed: 0,
            append_events_scanned: transcript.append.scanned,
            append_messages_materialized: transcript.append.materialized,
            append_messages_unchanged: transcript.append.unchanged,
            transcript_rows_projected: transcript.projected_rows,
        });
    }
    Ok(report)
}

fn looks_like_thread_record_path(path: &str) -> bool {
    path.starts_with("/tenants/")
        && path.contains("/users/")
        && path.contains("/threads/agents/")
        && path.ends_with("/thread.json")
}
