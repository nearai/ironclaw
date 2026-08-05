//! Lossless migration of release-candidate channel idempotency ledgers.

use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, Page, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::path::VirtualPath;
use thiserror::Error;

use crate::ProductInboundAction;

use super::{entry_for_action, path::action_path_suffix};

const LEGACY_ACTION_KIND: &str = "product_workflow_action";
const CURRENT_ACTION_KIND: &str = "product_surface_action";
const LEGACY_PRUNE_LEASE_KIND: &str = "product_workflow_prune_lease";
const CURRENT_PRUNE_LEASE_KIND: &str = "product_surface_prune_lease";
const MIGRATION_CAS_RETRIES: usize = 5;

fn log_malformed_source(error: impl std::fmt::Display) -> IdempotencyLedgerMigrationError {
    tracing::error!(%error, "idempotency ledger migration source decode failed");
    IdempotencyLedgerMigrationError::MalformedSource
}

fn log_malformed_target(error: impl std::fmt::Display) -> IdempotencyLedgerMigrationError {
    tracing::error!(%error, "idempotency ledger migration target decode failed");
    IdempotencyLedgerMigrationError::MalformedTarget
}

fn log_invalid_path(error: impl std::fmt::Display) -> IdempotencyLedgerMigrationError {
    tracing::error!(%error, "idempotency ledger migration path construction failed");
    IdempotencyLedgerMigrationError::InvalidPath
}

fn log_storage(error: impl std::fmt::Display) -> IdempotencyLedgerMigrationError {
    tracing::error!(%error, "idempotency ledger migration storage operation failed");
    IdempotencyLedgerMigrationError::Storage
}

/// Redacted aggregate evidence for one idempotency-ledger root migration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdempotencyLedgerMigrationReport {
    pub scanned_actions: usize,
    pub migrated_actions: usize,
    pub unchanged_actions: usize,
    pub skipped_transient_leases: usize,
}

/// Sanitized fail-closed errors from idempotency-ledger migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IdempotencyLedgerMigrationError {
    #[error("idempotency ledger migration paths are invalid")]
    InvalidPath,
    #[error("idempotency ledger migration source record is malformed")]
    MalformedSource,
    #[error("idempotency ledger migration target record is malformed")]
    MalformedTarget,
    #[error("idempotency ledger migration found divergent action state")]
    Conflict,
    #[error("idempotency ledger migration source changed while migration was running")]
    SourceChanged,
    #[error("idempotency ledger migration storage operation failed")]
    Storage,
    #[error("idempotency ledger migration compare-and-swap retries were exhausted")]
    Contention,
}

struct PlannedAction {
    source: VersionedEntry,
    target_path: VirtualPath,
    action: ProductInboundAction,
    desired: Entry,
    target: Option<VersionedEntry>,
}

/// Copy durable action outcomes from a released channel root to the generic
/// extension-keyed root while retaining the released rows for rollback.
///
/// The migration enumerates all source pages and preflights every target
/// collision before writing anything. It rewrites the old record kind and
/// indexed projection using the current domain serializer. Prune leases are
/// deliberately not copied because they are short-lived coordination state.
pub async fn migrate_idempotency_ledger_root<F>(
    filesystem: &F,
    source_root: &VirtualPath,
    target_root: &VirtualPath,
) -> Result<IdempotencyLedgerMigrationReport, IdempotencyLedgerMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    if source_root == target_root {
        return Err(IdempotencyLedgerMigrationError::InvalidPath);
    }
    let source_prefix = source_root.as_str().trim_end_matches('/');
    let target_prefix = target_root.as_str().trim_end_matches('/');
    if source_prefix.is_empty() || target_prefix.is_empty() {
        return Err(IdempotencyLedgerMigrationError::InvalidPath);
    }

    let entries = query_all(filesystem, source_root).await?;
    let mut report = IdempotencyLedgerMigrationReport::default();
    let mut plans = Vec::new();
    for source in entries {
        let relative = relative_path(source_prefix, &source.path)?;
        if is_prune_lease(relative, &source.entry) {
            report.skipped_transient_leases += 1;
            continue;
        }
        if !relative.starts_with("/actions/_scope/")
            || !matches!(
                source.entry.kind.as_ref().map(|kind| kind.as_str()),
                Some(LEGACY_ACTION_KIND | CURRENT_ACTION_KIND)
            )
        {
            return Err(IdempotencyLedgerMigrationError::MalformedSource);
        }
        let action: ProductInboundAction =
            source.entry.parse_json().map_err(log_malformed_source)?;
        if !source
            .path
            .as_str()
            .ends_with(&action_path_suffix(&action.fingerprint))
        {
            return Err(IdempotencyLedgerMigrationError::MalformedSource);
        }
        let target_path =
            VirtualPath::new(format!("{target_prefix}{relative}")).map_err(log_invalid_path)?;
        let desired = entry_for_action(&action).map_err(log_malformed_source)?;
        let target = filesystem.get(&target_path).await.map_err(log_storage)?;
        if let Some(existing) = &target {
            let existing_action: ProductInboundAction =
                existing.entry.parse_json().map_err(log_malformed_target)?;
            if existing_action != action {
                return Err(IdempotencyLedgerMigrationError::Conflict);
            }
        }
        report.scanned_actions += 1;
        plans.push(PlannedAction {
            source,
            target_path,
            action,
            desired,
            target,
        });
    }

    for plan in &plans {
        if plan
            .target
            .as_ref()
            .is_some_and(|target| target.entry == plan.desired)
        {
            report.unchanged_actions += 1;
            continue;
        }
        write_action(filesystem, plan).await?;
        report.migrated_actions += 1;
    }

    for plan in &plans {
        let source = filesystem
            .get(&plan.source.path)
            .await
            .map_err(log_storage)?
            .ok_or(IdempotencyLedgerMigrationError::SourceChanged)?;
        if source.version != plan.source.version || source.entry != plan.source.entry {
            return Err(IdempotencyLedgerMigrationError::SourceChanged);
        }
        let target = filesystem
            .get(&plan.target_path)
            .await
            .map_err(log_storage)?
            .ok_or(IdempotencyLedgerMigrationError::Storage)?;
        let target_action: ProductInboundAction =
            target.entry.parse_json().map_err(log_malformed_target)?;
        if target_action != plan.action || target.entry != plan.desired {
            return Err(IdempotencyLedgerMigrationError::Storage);
        }
    }
    Ok(report)
}

async fn query_all<F>(
    filesystem: &F,
    root: &VirtualPath,
) -> Result<Vec<VersionedEntry>, IdempotencyLedgerMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let mut rows = Vec::new();
    let mut offset = 0;
    loop {
        let page = filesystem
            .query(root, &Filter::All, Page::new(offset, Page::MAX_LIMIT))
            .await
            .map_err(log_storage)?;
        let received = page.len();
        rows.extend(page);
        if received < Page::MAX_LIMIT as usize {
            return Ok(rows);
        }
        offset += u64::from(Page::MAX_LIMIT);
    }
}

async fn write_action<F>(
    filesystem: &F,
    plan: &PlannedAction,
) -> Result<(), IdempotencyLedgerMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let mut current = plan.target.clone();
    for _ in 0..MIGRATION_CAS_RETRIES {
        let cas = current.as_ref().map_or(CasExpectation::Absent, |entry| {
            CasExpectation::Version(entry.version)
        });
        match filesystem
            .put(&plan.target_path, plan.desired.clone(), cas)
            .await
        {
            Ok(_) => return Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                current = filesystem
                    .get(&plan.target_path)
                    .await
                    .map_err(log_storage)?;
                if let Some(existing) = &current {
                    let action: ProductInboundAction =
                        existing.entry.parse_json().map_err(log_malformed_target)?;
                    if action != plan.action {
                        return Err(IdempotencyLedgerMigrationError::Conflict);
                    }
                    if existing.entry == plan.desired {
                        return Ok(());
                    }
                }
            }
            Err(error) => return Err(log_storage(error)),
        }
    }
    Err(IdempotencyLedgerMigrationError::Contention)
}

fn relative_path<'a>(
    source_prefix: &str,
    path: &'a VirtualPath,
) -> Result<&'a str, IdempotencyLedgerMigrationError> {
    let relative = path
        .as_str()
        .strip_prefix(source_prefix)
        .ok_or(IdempotencyLedgerMigrationError::MalformedSource)?;
    if !relative.starts_with('/') {
        return Err(IdempotencyLedgerMigrationError::MalformedSource);
    }
    Ok(relative)
}

fn is_prune_lease(relative: &str, entry: &Entry) -> bool {
    relative.ends_with("/_control/prune_lease.json")
        && matches!(
            entry.kind.as_ref().map(|kind| kind.as_str()),
            Some(LEGACY_PRUNE_LEASE_KIND | CURRENT_PRUNE_LEASE_KIND)
        )
}
