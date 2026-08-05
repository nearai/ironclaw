//! One-time durable product-auth migrations.
//!
//! The 1.0.0-rc.1 Slack OAuth recipe used the provider id
//! `slack_personal`; the unified 1.1.0-rc.1 recipe uses `slack`. This module
//! owns that persisted-wire fold so composition only has to sequence it before
//! auth workers and request serving start.

use std::sync::Arc;

use ironclaw_filesystem::{
    CasExpectation, FileType, FilesystemError, Filter, Page, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::path::VirtualPath;
use thiserror::Error;

use crate::{
    AuthErrorCode, AuthFlowRecord, AuthFlowStatus, AuthProviderId, CredentialAccount, Timestamp,
    is_terminal_status,
};

use super::paths::{account_path, flow_path};

const LEGACY_SLACK_PROVIDER_ID: &str = "slack_personal";
const CURRENT_SLACK_PROVIDER_ID: &str = "slack";
const TENANTS_ROOT: &str = "/tenants";
const BACKUP_SUFFIX: &str = ".ironclaw-1.0.0-rc1-slack-oauth-backup";
const MAX_MIGRATION_TENANTS: usize = 100_000;
const MAX_MIGRATION_USERS_PER_TENANT: usize = 100_000;
const MAX_MIGRATION_CANDIDATES: usize = 1_000_000;

/// Redacted aggregate for the startup coordinator and operator diagnostics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OAuthProviderAliasMigrationReport {
    pub examined_rows: usize,
    pub account_rows_migrated: usize,
    pub account_rows_already_current: usize,
    pub flow_rows_migrated: usize,
    pub flow_rows_already_current: usize,
    pub incomplete_flows_expired: usize,
    pub rollback_backups_created: usize,
    pub rollback_backups_reused: usize,
}

/// Sanitized fail-closed outcomes from the provider-id migration.
#[derive(Debug, Error)]
pub enum OAuthProviderAliasMigrationError {
    #[error("product-auth migration backend unavailable")]
    Backend,
    #[error("product-auth migration found a malformed durable record")]
    MalformedRecord,
    #[error("product-auth migration found a record whose scope does not match its path")]
    ScopePathMismatch,
    #[error("product-auth migration rollback backup conflicts with the source record")]
    BackupConflict,
    #[error("product-auth migration lost a compare-and-swap race")]
    WriteConflict,
}

fn malformed_record(error: impl std::fmt::Display) -> OAuthProviderAliasMigrationError {
    tracing::error!(%error, "product-auth migration record validation failed");
    OAuthProviderAliasMigrationError::MalformedRecord
}

fn backend_error(error: impl std::fmt::Display) -> OAuthProviderAliasMigrationError {
    tracing::error!(%error, "product-auth migration backend operation failed");
    OAuthProviderAliasMigrationError::Backend
}

/// Fold every durable 1.0.0-rc.1 Slack OAuth account and flow forward.
///
/// Exact source entries are copied beside the live record with
/// [`BACKUP_SUFFIX`] appended before their live records are rewritten. Keeping
/// the backup in the same tenant/user subtree preserves storage isolation; the
/// non-`.json` suffix keeps both release readers from treating it as a live
/// account or flow. Removing the suffix yields the exact rollback destination.
/// Credential account ids and secret handles are serialized back unchanged.
/// Non-terminal legacy flows cannot be safely resumed against the renamed
/// callback/provider contract, so they are made explicitly `expired` and
/// included in the report.
///
/// The full candidate set is discovered before the first write. Every live
/// rewrite uses the queried record version as its CAS fence. A retry after any
/// interruption either resumes from the exact backup or observes an already
/// current record.
pub async fn migrate_legacy_oauth_provider_alias<F>(
    root: Arc<F>,
    scoped: Arc<ScopedFilesystem<F>>,
    migrated_at: Timestamp,
) -> Result<OAuthProviderAliasMigrationReport, OAuthProviderAliasMigrationError>
where
    F: RootFilesystem + 'static,
{
    let candidates = discover_candidates(root.as_ref()).await?;
    let current_provider =
        AuthProviderId::new(CURRENT_SLACK_PROVIDER_ID).map_err(malformed_record)?;
    let mut report = OAuthProviderAliasMigrationReport {
        examined_rows: candidates.examined_rows,
        ..OAuthProviderAliasMigrationReport::default()
    };

    for row in candidates.rows {
        match classify_path(row.path.as_str()) {
            Some(AuthRecordKind::Account) => {
                match provider_candidate(&row.entry.body)? {
                    ProviderCandidate::Legacy => {}
                    ProviderCandidate::Current => {
                        report.account_rows_already_current =
                            report.account_rows_already_current.saturating_add(1);
                        continue;
                    }
                    ProviderCandidate::Other => continue,
                }
                let mut account: CredentialAccount =
                    serde_json::from_slice(&row.entry.body).map_err(malformed_record)?;
                verify_account_path(scoped.as_ref(), &row, &account)?;
                if account.provider.as_str() == CURRENT_SLACK_PROVIDER_ID {
                    report.account_rows_already_current =
                        report.account_rows_already_current.saturating_add(1);
                    continue;
                }
                if account.provider.as_str() != LEGACY_SLACK_PROVIDER_ID {
                    continue;
                }

                account.provider = current_provider.clone();
                archive_and_replace(root.as_ref(), &row, &account, &mut report).await?;
                report.account_rows_migrated = report.account_rows_migrated.saturating_add(1);
            }
            Some(AuthRecordKind::Flow) => {
                match provider_candidate(&row.entry.body)? {
                    ProviderCandidate::Legacy => {}
                    ProviderCandidate::Current => {
                        report.flow_rows_already_current =
                            report.flow_rows_already_current.saturating_add(1);
                        continue;
                    }
                    ProviderCandidate::Other => continue,
                }
                let mut flow: AuthFlowRecord =
                    serde_json::from_slice(&row.entry.body).map_err(malformed_record)?;
                verify_flow_path(scoped.as_ref(), &row, &flow)?;
                if flow.provider.as_str() == CURRENT_SLACK_PROVIDER_ID {
                    report.flow_rows_already_current =
                        report.flow_rows_already_current.saturating_add(1);
                    continue;
                }
                if flow.provider.as_str() != LEGACY_SLACK_PROVIDER_ID {
                    continue;
                }

                flow.provider = current_provider.clone();
                if !is_terminal_status(flow.status) {
                    flow.status = AuthFlowStatus::Expired;
                    flow.error = Some(AuthErrorCode::UnknownOrExpiredFlow);
                    flow.updated_at = migrated_at.max(flow.updated_at);
                    report.incomplete_flows_expired =
                        report.incomplete_flows_expired.saturating_add(1);
                }
                archive_and_replace(root.as_ref(), &row, &flow, &mut report).await?;
                report.flow_rows_migrated = report.flow_rows_migrated.saturating_add(1);
            }
            None => {}
        }
    }

    Ok(report)
}

struct MigrationCandidates {
    examined_rows: usize,
    rows: Vec<VersionedEntry>,
}

async fn discover_candidates<F>(
    root: &F,
) -> Result<MigrationCandidates, OAuthProviderAliasMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let tenants = VirtualPath::new(TENANTS_ROOT).map_err(backend_error)?;
    let mut examined_rows = 0usize;
    let mut rows = Vec::new();
    for tenant in bounded_directories(root, &tenants, MAX_MIGRATION_TENANTS).await? {
        let users = VirtualPath::new(format!("/tenants/{tenant}/users")).map_err(backend_error)?;
        for user in bounded_directories(root, &users, MAX_MIGRATION_USERS_PER_TENANT).await? {
            let auth_root = VirtualPath::new(format!(
                "/tenants/{tenant}/users/{user}/secrets/product-auth"
            ))
            .map_err(backend_error)?;
            let mut offset = 0u64;
            loop {
                let page = match root
                    .query(&auth_root, &Filter::All, Page::new(offset, Page::MAX_LIMIT))
                    .await
                {
                    Ok(page) => page,
                    Err(FilesystemError::NotFound { .. }) => break,
                    Err(error) => return Err(map_filesystem_error(error)),
                };
                if page.is_empty() {
                    break;
                }
                let received = page.len();
                examined_rows = examined_rows.saturating_add(received);
                for row in page {
                    if classify_path(row.path.as_str()).is_some() {
                        if rows.len() >= MAX_MIGRATION_CANDIDATES {
                            return Err(OAuthProviderAliasMigrationError::Backend);
                        }
                        rows.push(row);
                    }
                }
                if received < Page::MAX_LIMIT as usize {
                    break;
                }
                offset = offset.saturating_add(received as u64);
            }
        }
    }
    Ok(MigrationCandidates {
        examined_rows,
        rows,
    })
}

async fn bounded_directories<F>(
    root: &F,
    path: &VirtualPath,
    max_entries: usize,
) -> Result<Vec<String>, OAuthProviderAliasMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let entries = match root
        .list_dir_bounded(path, max_entries.saturating_add(1))
        .await
    {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(map_filesystem_error(error)),
    };
    if entries.len() > max_entries {
        return Err(OAuthProviderAliasMigrationError::Backend);
    }
    Ok(entries
        .into_iter()
        .filter(|entry| entry.file_type == FileType::Directory)
        .map(|entry| entry.name)
        .collect())
}

enum ProviderCandidate {
    Legacy,
    Current,
    Other,
}

fn provider_candidate(body: &[u8]) -> Result<ProviderCandidate, OAuthProviderAliasMigrationError> {
    let value: serde_json::Value = serde_json::from_slice(body).map_err(malformed_record)?;
    Ok(
        match value.get("provider").and_then(serde_json::Value::as_str) {
            Some(LEGACY_SLACK_PROVIDER_ID) => ProviderCandidate::Legacy,
            Some(CURRENT_SLACK_PROVIDER_ID) => ProviderCandidate::Current,
            _ => ProviderCandidate::Other,
        },
    )
}

#[derive(Clone, Copy)]
enum AuthRecordKind {
    Account,
    Flow,
}

fn classify_path(path: &str) -> Option<AuthRecordKind> {
    if !path.starts_with("/tenants/") || !path.contains("/product-auth/") {
        return None;
    }
    let mut segments = path.rsplit('/');
    let file = segments.next()?;
    if !file.ends_with(".json") {
        return None;
    }
    match segments.next()? {
        "accounts" => Some(AuthRecordKind::Account),
        "flows" => Some(AuthRecordKind::Flow),
        _ => None,
    }
}

fn verify_account_path<F>(
    scoped: &ScopedFilesystem<F>,
    row: &VersionedEntry,
    account: &CredentialAccount,
) -> Result<(), OAuthProviderAliasMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let relative = account_path(&account.scope, account.id).map_err(malformed_record)?;
    let expected = scoped
        .resolve(&account.scope.resource, &relative)
        .map_err(backend_error)?;
    if expected != row.path {
        return Err(OAuthProviderAliasMigrationError::ScopePathMismatch);
    }
    Ok(())
}

fn verify_flow_path<F>(
    scoped: &ScopedFilesystem<F>,
    row: &VersionedEntry,
    flow: &AuthFlowRecord,
) -> Result<(), OAuthProviderAliasMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let relative = flow_path(&flow.scope, flow.id).map_err(malformed_record)?;
    let expected = scoped
        .resolve(&flow.scope.resource, &relative)
        .map_err(backend_error)?;
    if expected != row.path {
        return Err(OAuthProviderAliasMigrationError::ScopePathMismatch);
    }
    Ok(())
}

async fn archive_and_replace<F, T>(
    root: &F,
    source: &VersionedEntry,
    replacement: &T,
    report: &mut OAuthProviderAliasMigrationReport,
) -> Result<(), OAuthProviderAliasMigrationError>
where
    F: RootFilesystem + ?Sized,
    T: serde::Serialize,
{
    match ensure_exact_backup(root, source).await? {
        BackupDisposition::Created => {
            report.rollback_backups_created = report.rollback_backups_created.saturating_add(1);
        }
        BackupDisposition::Reused => {
            report.rollback_backups_reused = report.rollback_backups_reused.saturating_add(1);
        }
    }

    let body = serde_json::to_vec(replacement).map_err(malformed_record)?;
    let mut replacement_entry = source.entry.clone();
    replacement_entry.body = body;
    root.put(
        &source.path,
        replacement_entry,
        CasExpectation::Version(source.version),
    )
    .await
    .map_err(|error| match error {
        FilesystemError::VersionMismatch { .. } => OAuthProviderAliasMigrationError::WriteConflict,
        error => backend_error(error),
    })?;
    Ok(())
}

enum BackupDisposition {
    Created,
    Reused,
}

async fn ensure_exact_backup<F>(
    root: &F,
    source: &VersionedEntry,
) -> Result<BackupDisposition, OAuthProviderAliasMigrationError>
where
    F: RootFilesystem + ?Sized,
{
    let backup_path = backup_path(&source.path)?;
    match root.get(&backup_path).await.map_err(map_filesystem_error)? {
        Some(existing) => {
            if existing.entry != source.entry {
                return Err(OAuthProviderAliasMigrationError::BackupConflict);
            }
            Ok(BackupDisposition::Reused)
        }
        None => match root
            .put(&backup_path, source.entry.clone(), CasExpectation::Absent)
            .await
        {
            Ok(_) => Ok(BackupDisposition::Created),
            Err(FilesystemError::VersionMismatch { .. }) => {
                let existing = root
                    .get(&backup_path)
                    .await
                    .map_err(map_filesystem_error)?
                    .ok_or(OAuthProviderAliasMigrationError::BackupConflict)?;
                if existing.entry != source.entry {
                    return Err(OAuthProviderAliasMigrationError::BackupConflict);
                }
                Ok(BackupDisposition::Reused)
            }
            Err(error) => Err(backend_error(error)),
        },
    }
}

fn backup_path(source: &VirtualPath) -> Result<VirtualPath, OAuthProviderAliasMigrationError> {
    VirtualPath::new(format!("{}{BACKUP_SUFFIX}", source.as_str())).map_err(backend_error)
}

fn map_filesystem_error(error: FilesystemError) -> OAuthProviderAliasMigrationError {
    match error {
        FilesystemError::VersionMismatch { .. } => OAuthProviderAliasMigrationError::WriteConflict,
        error => backend_error(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{TimeZone as _, Utc};
    use ironclaw_filesystem::{
        CasExpectation, ContentType, Entry, InMemoryBackend, RootFilesystem, ScopedFilesystem,
    };
    use ironclaw_host_api::{
        error::HostApiError,
        ids::SecretHandle,
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
    };
    use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStorePort};
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::{CredentialAccountRecordSource as _, FilesystemAuthProductServices};

    const RC1_ACCOUNT_WIRE: &str = r#"{"id":"11111111-1111-4111-8111-111111111111","scope":{"resource":{"tenant_id":"acme","user_id":"alice","agent_id":null,"project_id":null,"mission_id":null,"thread_id":null,"invocation_id":"22222222-2222-4222-8222-222222222222"},"surface":"web"},"provider":"slack_personal","label":"Alice Slack","status":"configured","ownership":"user_reusable","owner_extension":null,"granted_extensions":["slack"],"access_secret":"slack-access-handle","refresh_secret":"slack-refresh-handle","scopes":["channels:read","search:read"],"provider_identity":{"subject":"U123","team_id":"T123","enterprise_id":null,"app_id":"A123"},"created_at":"2026-07-01T10:00:00Z","updated_at":"2026-07-01T10:01:00Z"}"#;
    const RC1_INCOMPLETE_FLOW_WIRE: &str = r#"{"id":"33333333-3333-4333-8333-333333333333","scope":{"resource":{"tenant_id":"acme","user_id":"alice","agent_id":null,"project_id":null,"mission_id":null,"thread_id":"44444444-4444-4444-8444-444444444444","invocation_id":"55555555-5555-4555-8555-555555555555"},"surface":"web","session_id":"slack-setup-session"},"kind":"integration_credential","status":"awaiting_user","provider":"slack_personal","challenge":{"type":"o_auth_url","authorization_url":"https://slack.com/oauth/v2/authorize?client_id=fixture","expires_at":"2026-08-01T10:30:00Z"},"continuation":{"type":"setup_only"},"credential_account_id":null,"credential_secret_fingerprint":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","update_binding":null,"opaque_state_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","pkce_verifier_hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","authorization_code_hash":null,"error":null,"continuation_emitted_at":null,"created_at":"2026-08-01T10:00:00Z","updated_at":"2026-08-01T10:00:00Z","expires_at":"2026-08-01T10:30:00Z"}"#;

    fn invocation_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/secrets")?,
            VirtualPath::new(format!(
                "/tenants/{}/users/{}/secrets",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        )])
    }

    async fn seed_exact_wire(backend: &InMemoryBackend, path: &str, wire: &str) -> VersionedEntry {
        let path = VirtualPath::new(path).expect("valid fixture path");
        backend
            .put(
                &path,
                Entry::bytes(wire.as_bytes().to_vec()).with_content_type(ContentType::json()),
                CasExpectation::Absent,
            )
            .await
            .expect("seed rc1 wire");
        backend
            .get(&path)
            .await
            .expect("read fixture")
            .expect("fixture exists")
    }

    #[tokio::test]
    async fn rc1_slack_oauth_migration_preserves_handles_expires_flow_and_is_idempotent() {
        let fixture_account: CredentialAccount =
            serde_json::from_str(RC1_ACCOUNT_WIRE).expect("rc1 account fixture parses");
        let _: AuthFlowRecord =
            serde_json::from_str(RC1_INCOMPLETE_FLOW_WIRE).expect("rc1 flow fixture parses");
        let backend = Arc::new(InMemoryBackend::new());
        let secret_store = Arc::new(SecretStore::ephemeral_over(Arc::clone(&backend)));
        for (handle, material) in [
            ("slack-access-handle", "xoxp-access-rc1"),
            ("slack-refresh-handle", "xoxe-refresh-rc1"),
        ] {
            secret_store
                .put(
                    fixture_account.scope.resource.clone(),
                    SecretHandle::new(handle).expect("secret handle"),
                    SecretMaterial::from(material.to_string()),
                    None,
                )
                .await
                .expect("seed encrypted rc1 credential");
        }
        let scoped = Arc::new(ScopedFilesystem::new(
            Arc::clone(&backend),
            invocation_mount_view,
        ));
        let account_path = "/tenants/acme/users/alice/secrets/product-auth/web/accounts/11111111-1111-4111-8111-111111111111.json";
        let flow_path = "/tenants/acme/users/alice/secrets/product-auth/web/sessions/slack-setup-session/flows/33333333-3333-4333-8333-333333333333.json";
        let old_account = seed_exact_wire(&backend, account_path, RC1_ACCOUNT_WIRE).await;
        let old_flow = seed_exact_wire(&backend, flow_path, RC1_INCOMPLETE_FLOW_WIRE).await;

        // Secret material is deliberately outside the product-auth record. A
        // byte-for-byte sentinel proves this migration never re-encrypts or
        // otherwise touches the referenced secret-store authority.
        let secret_path =
            VirtualPath::new("/tenants/acme/users/alice/secrets/entries/slack-access-handle.json")
                .expect("secret path");
        let secret_entry = Entry::bytes(b"encrypted-secret-ciphertext".to_vec());
        backend
            .put(&secret_path, secret_entry.clone(), CasExpectation::Absent)
            .await
            .expect("seed secret ciphertext");
        let secret_before = backend
            .get(&secret_path)
            .await
            .expect("read secret")
            .expect("secret exists");

        let migrated_at = Utc
            .with_ymd_and_hms(2026, 8, 5, 12, 0, 0)
            .single()
            .expect("timestamp");
        let report = migrate_legacy_oauth_provider_alias(
            Arc::clone(&backend),
            Arc::clone(&scoped),
            migrated_at,
        )
        .await
        .expect("migration succeeds");
        assert_eq!(report.account_rows_migrated, 1);
        assert_eq!(report.flow_rows_migrated, 1);
        assert_eq!(report.incomplete_flows_expired, 1);
        assert_eq!(report.rollback_backups_created, 2);

        let migrated_account: CredentialAccount = serde_json::from_slice(
            &backend
                .get(&VirtualPath::new(account_path).expect("account path"))
                .await
                .expect("read account")
                .expect("account exists")
                .entry
                .body,
        )
        .expect("current account wire");
        assert_eq!(
            migrated_account.provider.as_str(),
            CURRENT_SLACK_PROVIDER_ID
        );
        assert_eq!(
            migrated_account.id.to_string(),
            "11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(
            migrated_account
                .access_secret
                .as_ref()
                .map(|value| value.as_str()),
            Some("slack-access-handle")
        );
        assert_eq!(
            migrated_account
                .refresh_secret
                .as_ref()
                .map(|value| value.as_str()),
            Some("slack-refresh-handle")
        );

        let migrated_flow: AuthFlowRecord = serde_json::from_slice(
            &backend
                .get(&VirtualPath::new(flow_path).expect("flow path"))
                .await
                .expect("read flow")
                .expect("flow exists")
                .entry
                .body,
        )
        .expect("current flow wire");
        assert_eq!(migrated_flow.provider.as_str(), CURRENT_SLACK_PROVIDER_ID);
        assert_eq!(migrated_flow.status, AuthFlowStatus::Expired);
        assert_eq!(
            migrated_flow.error,
            Some(AuthErrorCode::UnknownOrExpiredFlow)
        );
        assert_eq!(migrated_flow.updated_at, migrated_at);

        assert_eq!(
            backend
                .get(&backup_path(&old_account.path).expect("account backup path"))
                .await
                .expect("read account backup")
                .expect("account backup")
                .entry,
            old_account.entry
        );
        assert_eq!(
            backend
                .get(&backup_path(&old_flow.path).expect("flow backup path"))
                .await
                .expect("read flow backup")
                .expect("flow backup")
                .entry,
            old_flow.entry
        );
        assert_eq!(
            backend
                .get(&secret_path)
                .await
                .expect("read secret after migration")
                .expect("secret still exists"),
            secret_before
        );

        // Reopen the production account reader over the durable filesystem,
        // then consume both retained handles. This is the caller-facing proof
        // that a configured rc1 Slack account remains selectable and usable
        // after the provider rename without starting a new OAuth flow.
        let restarted_accounts = FilesystemAuthProductServices::new_with_root(
            Arc::new(ScopedFilesystem::new(
                Arc::clone(&backend),
                invocation_mount_view,
            )),
            Arc::clone(&backend),
            Arc::clone(&secret_store) as Arc<dyn SecretStorePort>,
        );
        let reopened = restarted_accounts
            .accounts_for_owner(&migrated_account.scope)
            .await
            .expect("list configured accounts after restart");
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened[0].id, migrated_account.id);
        assert_eq!(reopened[0].provider.as_str(), CURRENT_SLACK_PROVIDER_ID);
        for (handle, expected) in [
            (
                reopened[0]
                    .access_secret
                    .as_ref()
                    .expect("access handle retained"),
                "xoxp-access-rc1",
            ),
            (
                reopened[0]
                    .refresh_secret
                    .as_ref()
                    .expect("refresh handle retained"),
                "xoxe-refresh-rc1",
            ),
        ] {
            let lease = secret_store
                .lease_once(&fixture_account.scope.resource, handle)
                .await
                .expect("lease retained credential");
            let material = secret_store
                .consume(&fixture_account.scope.resource, lease.id)
                .await
                .expect("decrypt retained credential");
            assert_eq!(material.expose_secret(), expected);
        }

        let account_after_first = backend
            .get(&VirtualPath::new(account_path).expect("account path"))
            .await
            .expect("read account")
            .expect("account exists");
        let flow_after_first = backend
            .get(&VirtualPath::new(flow_path).expect("flow path"))
            .await
            .expect("read flow")
            .expect("flow exists");
        let second = migrate_legacy_oauth_provider_alias(backend.clone(), scoped, migrated_at)
            .await
            .expect("second pass succeeds");
        assert_eq!(second.account_rows_migrated, 0);
        assert_eq!(second.flow_rows_migrated, 0);
        assert_eq!(second.incomplete_flows_expired, 0);
        assert_eq!(second.account_rows_already_current, 1);
        assert_eq!(second.flow_rows_already_current, 1);
        assert_eq!(
            backend
                .get(&account_after_first.path)
                .await
                .expect("read account after second pass")
                .expect("account exists"),
            account_after_first
        );
        assert_eq!(
            backend
                .get(&flow_after_first.path)
                .await
                .expect("read flow after second pass")
                .expect("flow exists"),
            flow_after_first
        );
    }

    #[tokio::test]
    async fn empty_backend_has_no_oauth_alias_state_to_migrate() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = Arc::new(ScopedFilesystem::new(
            Arc::clone(&backend),
            invocation_mount_view,
        ));

        let report = migrate_legacy_oauth_provider_alias(
            backend,
            scoped,
            Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0)
                .single()
                .expect("timestamp"),
        )
        .await
        .expect("an empty installation has no legacy OAuth state");

        assert_eq!(report, OAuthProviderAliasMigrationReport::default());
    }

    #[tokio::test]
    async fn rc1_slack_oauth_migration_fails_closed_on_backup_collision() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = Arc::new(ScopedFilesystem::new(
            Arc::clone(&backend),
            invocation_mount_view,
        ));
        let account_path = "/tenants/acme/users/alice/secrets/product-auth/web/accounts/11111111-1111-4111-8111-111111111111.json";
        let source = seed_exact_wire(&backend, account_path, RC1_ACCOUNT_WIRE).await;
        backend
            .put(
                &backup_path(&source.path).expect("backup path"),
                Entry::bytes(b"different source authority".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .expect("seed collision");

        let error = migrate_legacy_oauth_provider_alias(backend.clone(), scoped, Utc::now())
            .await
            .expect_err("collision must stop migration");
        assert!(matches!(
            error,
            OAuthProviderAliasMigrationError::BackupConflict
        ));
        let untouched = backend
            .get(&source.path)
            .await
            .expect("read source")
            .expect("source remains");
        assert_eq!(untouched.entry, source.entry);
        assert_eq!(untouched.version, source.version);
    }

    #[tokio::test]
    async fn rc1_slack_oauth_migration_resumes_after_backup_before_live_rewrite() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = Arc::new(ScopedFilesystem::new(
            Arc::clone(&backend),
            invocation_mount_view,
        ));
        let account_path = "/tenants/acme/users/alice/secrets/product-auth/web/accounts/11111111-1111-4111-8111-111111111111.json";
        let source = seed_exact_wire(&backend, account_path, RC1_ACCOUNT_WIRE).await;
        let backup = backup_path(&source.path).expect("backup path");
        backend
            .put(&backup, source.entry.clone(), CasExpectation::Absent)
            .await
            .expect("simulate interrupted backup write");

        let report = migrate_legacy_oauth_provider_alias(backend.clone(), scoped, Utc::now())
            .await
            .expect("migration resumes");
        assert_eq!(report.account_rows_migrated, 1);
        assert_eq!(report.rollback_backups_created, 0);
        assert_eq!(report.rollback_backups_reused, 1);

        let current = backend
            .get(&source.path)
            .await
            .expect("read current")
            .expect("current exists");
        let account: CredentialAccount =
            serde_json::from_slice(&current.entry.body).expect("current account");
        assert_eq!(account.provider.as_str(), CURRENT_SLACK_PROVIDER_ID);
        assert_eq!(
            backend
                .get(&backup)
                .await
                .expect("read backup")
                .expect("backup exists")
                .entry,
            source.entry
        );
    }

    #[tokio::test]
    async fn rc1_slack_oauth_migration_discovers_records_beyond_one_backend_page() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = Arc::new(ScopedFilesystem::new(
            Arc::clone(&backend),
            invocation_mount_view,
        ));
        for index in 0..Page::MAX_LIMIT {
            let path = VirtualPath::new(format!(
                "/tenants/acme/users/alice/secrets/product-auth/aaa-unrelated/{index:04}.bin"
            ))
            .expect("unrelated path");
            backend
                .put(&path, Entry::bytes(Vec::new()), CasExpectation::Absent)
                .await
                .expect("seed unrelated row");
        }
        let account_path = "/tenants/acme/users/alice/secrets/product-auth/web/accounts/11111111-1111-4111-8111-111111111111.json";
        seed_exact_wire(&backend, account_path, RC1_ACCOUNT_WIRE).await;

        let report = migrate_legacy_oauth_provider_alias(backend, scoped, Utc::now())
            .await
            .expect("migration scans the second page");
        assert_eq!(report.examined_rows, Page::MAX_LIMIT as usize + 1);
        assert_eq!(report.account_rows_migrated, 1);
    }
}
