#![allow(
    dead_code,
    reason = "durable product-auth is staged for production/webui composition; clippy can check this crate before those callers are enabled"
)]

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use futures::{StreamExt as _, TryStreamExt as _, stream};

use chrono::Utc;
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FileType, FilesystemError,
    RecordVersion, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    AgentId, ProjectId, ResourceScope, ScopedPath, SecretHandle, TenantId, UserId,
};
use ironclaw_secrets::SecretStore;
use serde::{Serialize, de::DeserializeOwned};

use ironclaw_auth::{
    AuthContinuationRef, AuthFlowId, AuthFlowOwnerScope, AuthFlowRecord, AuthProductError,
    AuthSessionId, AuthSurface, CredentialAccount, CredentialAccountId,
    CredentialAccountOwnerScope, CredentialAccountSelectionRequest, CredentialAccountStatus,
    NewCredentialAccount,
};
use ironclaw_host_api::VirtualPath;

use self::domain::validate_new_credential_account;
use self::paths::{
    account_path, account_root, flow_path, flow_root, fs_error, join_scoped, surface_sessions_root,
};

mod accounts;
mod cleanup;
mod domain;
mod flows;
mod interactions;
mod paths;
mod provider;
#[cfg(test)]
mod tests;

const MAX_OWNER_SESSION_ROOTS_PER_SURFACE: usize = 1024;
const MAX_OWNER_RECORDS_PER_ROOT: usize = 1024;
const MAX_ACCOUNT_DISCOVERY_ENTRIES_PER_ROOT: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccountEnumerationMode {
    BestEffortKeepalive,
    StrictMigration,
}

struct LocatedCredentialAccount {
    located_scope: ironclaw_auth::AuthProductScope,
    path: ScopedPath,
    account: CredentialAccount,
    #[allow(
        dead_code,
        reason = "strict discovery records the observed version for auditability"
    )]
    version: RecordVersion,
}

fn flow_requires_lifecycle_cleanup(flow: &AuthFlowRecord) -> bool {
    !ironclaw_auth::is_terminal_status(flow.status)
        || (flow.continuation_emitted_at.is_none()
            && matches!(
                flow.continuation,
                AuthContinuationRef::TurnGateResume { .. }
            ))
}

fn migration_cas_error(error: CasUpdateError<AuthProductError>) -> AuthProductError {
    match error {
        CasUpdateError::Apply(error) => error,
        CasUpdateError::RetriesExhausted => AuthProductError::BackendConflict,
        CasUpdateError::Timeout | CasUpdateError::CasUnsupported | CasUpdateError::Backend(_) => {
            AuthProductError::BackendUnavailable
        }
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) use provider::UnavailableAuthProviderClient;

/// Durable production implementation of the product-auth ports.
///
/// Records live under the caller's scoped `/secrets/product-auth` tree. Raw
/// provider tokens and manual token values are stored only through
/// [`SecretStore`] and represented here by opaque secret handles.
//
// TODO(#4175 follow-up): project completed product-auth accounts into
// `ironclaw_secrets::CredentialAccountStore` so the runtime credential
// broker shares one source of truth with the product-auth UX layer.
//
// Today two `CredentialAccount` records coexist:
//   * `ironclaw_auth::CredentialAccount` — product-auth UX record stored
//     here (provider id, label, owner_extension, grants, status,
//     provider_scopes, access/refresh secret handles). Read/written by
//     setup, OAuth callback, manual-token submit, uninstall cleanup.
//   * `ironclaw_secrets::CredentialAccount` — runtime broker record
//     consumed on every extension HTTP call to issue
//     `CredentialSessionRequest`s (invocation_id, capability_id,
//     extension_id, method, url, expires_at, max_uses).
//
// They are deliberately separate stores (see
// `docs/reborn/contracts/auth-product.md` → "Durable Production Slice")
// because their consumers, lifecycles, and access patterns differ. The
// missing link is a one-way projection product-auth → broker on flow
// completion / account update / cleanup, so the two universes cannot
// drift. Until that lands, broker-account population stays the caller's
// responsibility and drift is not policed here.
pub(crate) struct FilesystemAuthProductServices<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    /// Raw root filesystem held separately for deployment-wide scans (B1).
    ///
    /// `ScopedFilesystem` does not expose its inner `RootFilesystem`, so
    /// this field is wired explicitly by the factory (`new_with_root`).
    /// `None` in test/local-dev paths that do not need cross-tenant listing —
    /// `list_refresh_candidates` returns an empty vec in that case (safe: no
    /// accounts are refreshed, which is benign for local/test deployments).
    root: Option<Arc<F>>,
    secret_store: Arc<dyn SecretStore>,
    locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    #[cfg(test)]
    filtered_scan_records_before_sort: AtomicUsize,
}

impl<F> FilesystemAuthProductServices<F>
where
    F: RootFilesystem,
{
    pub(crate) fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            filesystem,
            root: None,
            secret_store,
            locks: Mutex::new(HashMap::new()),
            #[cfg(test)]
            filtered_scan_records_before_sort: AtomicUsize::new(0),
        }
    }

    /// Create the service with explicit access to the backing `RootFilesystem`.
    ///
    /// Production composition calls this so `list_refresh_candidates` (B1) can
    /// enumerate accounts across all owners without going through the per-user
    /// `ResourceScope` resolution layer. Pass the same `Arc<F>` that was used
    /// to construct the `ScopedFilesystem`.
    pub(crate) fn new_with_root(
        filesystem: Arc<ScopedFilesystem<F>>,
        root: Arc<F>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            filesystem,
            root: Some(root),
            secret_store,
            locks: Mutex::new(HashMap::new()),
            #[cfg(test)]
            filtered_scan_records_before_sort: AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    fn filtered_scan_records_before_sort(&self) -> usize {
        self.filtered_scan_records_before_sort
            .load(Ordering::SeqCst)
    }

    fn lock_for(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
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

    async fn purge_secret_handle(&self, scope: &ResourceScope, handle: &SecretHandle) {
        if let Err(error) = self.secret_store.delete(scope, handle).await {
            tracing::debug!(
                secret_store_reason = error.stable_reason(),
                "best-effort secret cleanup failed"
            );
        }
    }

    async fn read_record<T>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<(T, RecordVersion)>, AuthProductError>
    where
        T: DeserializeOwned,
    {
        let Some(versioned) = self.filesystem.get(scope, path).await.map_err(fs_error)? else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&versioned.entry.body)
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        Ok(Some((value, versioned.version)))
    }

    async fn read_account_record_for_scan(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<CredentialAccount>, AuthProductError> {
        let Some(versioned) = self.filesystem.get(scope, path).await.map_err(fs_error)? else {
            return Ok(None);
        };
        serde_json::from_slice(&versioned.entry.body)
            .map(Some)
            .map_err(|_| AuthProductError::BackendUnavailable)
    }

    async fn write_record<T>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        value: &T,
        cas: CasExpectation,
    ) -> Result<RecordVersion, AuthProductError>
    where
        T: Serialize,
    {
        let body = serde_json::to_vec(value).map_err(|_| AuthProductError::BackendUnavailable)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(scope, path, entry, cas)
            .await
            .map_err(fs_error)
    }

    async fn read_flow(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<(AuthFlowRecord, RecordVersion)>, AuthProductError> {
        self.read_record(&scope.resource, &flow_path(scope, flow_id)?)
            .await
    }

    async fn write_flow(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        record: &AuthFlowRecord,
        cas: CasExpectation,
    ) -> Result<RecordVersion, AuthProductError> {
        self.write_record(&scope.resource, &flow_path(scope, record.id)?, record, cas)
            .await
    }

    async fn flows_for_scope(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
    ) -> Result<Vec<(AuthFlowRecord, RecordVersion)>, AuthProductError> {
        let mut flows = self.flow_records_under_scope_root(scope).await?;
        flows.retain(|(flow, _)| scope_matches(scope, &flow.scope));
        flows.sort_by_key(|(flow, _)| flow.id);
        Ok(flows)
    }

    async fn flow_records_under_scope_root(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
    ) -> Result<Vec<(AuthFlowRecord, RecordVersion)>, AuthProductError> {
        let root = flow_root(scope)?;
        let entries = match self.filesystem.list_dir(&scope.resource, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_error(error)),
        };
        const MAX_CONCURRENT_READS: usize = 16;
        let mut flows: Vec<(AuthFlowRecord, RecordVersion)> = stream::iter(
            entries
                .into_iter()
                .filter(|e| e.name.ends_with(".json"))
                .map(|entry| {
                    let path = join_scoped(&root, &entry.name);
                    async move {
                        let path = path?;
                        self.read_record::<AuthFlowRecord>(&scope.resource, &path)
                            .await
                    }
                }),
        )
        .buffer_unordered(MAX_CONCURRENT_READS)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect();
        flows.sort_by_key(|(flow, _)| flow.id);
        Ok(flows)
    }

    async fn flow_records_for_owner(
        &self,
        owner: &AuthFlowOwnerScope,
    ) -> Result<Vec<AuthFlowRecord>, AuthProductError> {
        let resource = ResourceScope {
            tenant_id: owner.tenant_id.clone(),
            user_id: owner.user_id.clone(),
            agent_id: owner.agent_id.clone(),
            project_id: owner.project_id.clone(),
            mission_id: None,
            thread_id: Some(owner.thread_id.clone()),
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        self.flow_records_for_resource_filtered(&resource, |flow| owner.matches(flow))
            .await
    }

    async fn flow_records_for_resource_filtered<P>(
        &self,
        resource: &ResourceScope,
        predicate: P,
    ) -> Result<Vec<AuthFlowRecord>, AuthProductError>
    where
        P: Fn(&AuthFlowRecord) -> bool + Sync,
    {
        let mut flows = Vec::new();
        for surface in AuthSurface::ALL {
            let scope = ironclaw_auth::AuthProductScope::new(resource.clone(), surface);
            flows.extend(
                self.flow_records_under_scope_root(&scope)
                    .await?
                    .into_iter()
                    .map(|(flow, _)| flow)
                    .filter(|flow| predicate(flow)),
            );
            let sessions_root = surface_sessions_root(resource, surface)?;
            let mut entries = match self
                .filesystem
                .list_dir_bounded(
                    resource,
                    &sessions_root,
                    MAX_OWNER_SESSION_ROOTS_PER_SURFACE.saturating_add(1),
                )
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => return Err(fs_error(error)),
            };
            if entries.len() > MAX_OWNER_SESSION_ROOTS_PER_SURFACE {
                return Err(AuthProductError::BackendUnavailable);
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            for entry in entries {
                if entry.file_type != FileType::Directory {
                    continue;
                }
                let Ok(session_id) = AuthSessionId::new(entry.name) else {
                    // silent-ok: ignore an unexpected non-session directory under the bounded root.
                    continue;
                };
                let mut session_scope =
                    ironclaw_auth::AuthProductScope::new(resource.clone(), surface);
                session_scope.session_id = Some(session_id);
                flows.extend(
                    self.flow_records_under_scope_root(&session_scope)
                        .await?
                        .into_iter()
                        .map(|(flow, _)| flow)
                        .filter(|flow| predicate(flow)),
                );
            }
        }
        flows.sort_by_key(|flow| flow.id);
        flows.dedup_by_key(|flow| flow.id);
        Ok(flows)
    }

    /// Auth-flow records still requiring lifecycle cleanup for a credential
    /// owner + provider, walked across surfaces/sessions but not one thread.
    ///
    /// The lifecycle/disconnect analogue of [`Self::account_records_for_owner`]:
    /// flow storage is keyed by agent/project/surface/session (see `flow_root`)
    /// and never by thread, so a channel disconnect — which carries no thread —
    /// can still reach every pending flow a connect created, including
    /// thread-less setup flows and thread-scoped turn-gate flows. Used by
    /// lifecycle cleanup to cancel the disconnected provider's stale flows so
    /// they cannot wedge the next connect. Provider-agnostic by construction.
    async fn lifecycle_flows_for_owner_provider(
        &self,
        resource: &ResourceScope,
        provider: &ironclaw_auth::AuthProviderId,
    ) -> Result<Vec<AuthFlowRecord>, AuthProductError> {
        let resource = ResourceScope {
            tenant_id: resource.tenant_id.clone(),
            user_id: resource.user_id.clone(),
            agent_id: resource.agent_id.clone(),
            project_id: resource.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        self.flow_records_for_resource_filtered(&resource, |flow| {
            &flow.provider == provider && flow_requires_lifecycle_cleanup(flow)
        })
        .await
    }

    async fn read_account(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        account_id: CredentialAccountId,
    ) -> Result<Option<(CredentialAccount, RecordVersion)>, AuthProductError> {
        self.read_record(&scope.resource, &account_path(scope, account_id)?)
            .await
    }

    async fn write_account(
        &self,
        account: &CredentialAccount,
        cas: CasExpectation,
    ) -> Result<RecordVersion, AuthProductError> {
        self.write_record(
            &account.scope.resource,
            &account_path(&account.scope, account.id)?,
            account,
            cas,
        )
        .await
    }

    /// Returns all credential accounts for `scope`, reading records concurrently.
    async fn accounts_for_scope(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        let mut accounts = self
            .account_records_under_scope_root(scope)
            .await?
            .into_iter()
            .filter(|account| scope_matches(scope, &account.scope))
            .collect::<Vec<_>>();
        accounts.sort_by_key(|account| account.id);
        Ok(accounts)
    }

    /// Returns all credential accounts stored under `scope`'s durable root.
    ///
    /// Normal product-auth lookups still apply exact `AuthProductScope`
    /// filtering through `accounts_for_scope`; runtime credential selection uses
    /// this lower-level scan because setup and runtime invocations necessarily
    /// carry different invocation ids.
    async fn account_records_under_scope_root(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        self.account_records_under_scope_root_with_limit(scope, None)
            .await
    }

    async fn account_records_under_scope_root_with_limit(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        max_records: Option<usize>,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        self.account_records_under_scope_root_filtered_with_limit(scope, max_records, |_| true)
            .await
    }

    async fn account_records_under_scope_root_filtered_with_limit<P>(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        max_records: Option<usize>,
        predicate: P,
    ) -> Result<Vec<CredentialAccount>, AuthProductError>
    where
        P: Fn(&CredentialAccount) -> bool,
    {
        let root = account_root(scope)?;
        let entries = match max_records {
            Some(max_records) => {
                self.filesystem
                    .list_dir_bounded(&scope.resource, &root, max_records.saturating_add(1))
                    .await
            }
            None => self.filesystem.list_dir(&scope.resource, &root).await,
        };
        let entries = match entries {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_error(error)),
        };
        if max_records.is_some_and(|max_records| entries.len() > max_records) {
            return Err(AuthProductError::BackendUnavailable);
        }
        // Read records concurrently, capped at 16 in-flight ops to avoid
        // exhausting file-descriptor or connection limits on large scopes.
        const MAX_CONCURRENT_READS: usize = 16;
        let mut accounts: Vec<CredentialAccount> = stream::iter(
            entries
                .into_iter()
                .filter(|e| e.name.ends_with(".json"))
                .map(|entry| {
                    let path = join_scoped(&root, &entry.name);
                    async move {
                        let path = path?;
                        self.read_account_record_for_scan(&scope.resource, &path)
                            .await
                    }
                }),
        )
        .buffer_unordered(MAX_CONCURRENT_READS)
        .try_filter_map(|account| {
            let selected = account.filter(|account| predicate(account));
            #[cfg(test)]
            if selected.is_some() {
                self.filtered_scan_records_before_sort
                    .fetch_add(1, Ordering::SeqCst);
            }
            std::future::ready(Ok(selected))
        })
        .try_collect::<Vec<CredentialAccount>>()
        .await?;
        accounts.sort_by_key(|account| account.id);
        Ok(accounts)
    }

    async fn account_scopes_for_owner(
        &self,
        owner: &CredentialAccountOwnerScope,
    ) -> Result<Vec<ironclaw_auth::AuthProductScope>, AuthProductError> {
        self.account_scopes_for_owner_with_mode(owner, AccountEnumerationMode::BestEffortKeepalive)
            .await
    }

    async fn account_scopes_for_owner_with_mode(
        &self,
        owner: &CredentialAccountOwnerScope,
        mode: AccountEnumerationMode,
    ) -> Result<Vec<ironclaw_auth::AuthProductScope>, AuthProductError> {
        let resource = ResourceScope {
            tenant_id: owner.tenant_id.clone(),
            user_id: owner.user_id.clone(),
            agent_id: owner.agent_id.clone(),
            project_id: owner.project_id.clone(),
            mission_id: owner.mission_id.clone(),
            thread_id: owner.thread_id.clone(),
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let mut scopes = Vec::new();
        for surface in AuthSurface::ALL {
            scopes.push(ironclaw_auth::AuthProductScope::new(
                resource.clone(),
                surface,
            ));
            if let Some(session_id) = &owner.session_id {
                scopes.push(
                    ironclaw_auth::AuthProductScope::new(resource.clone(), surface)
                        .with_session_id(session_id.clone()),
                );
                continue;
            }
            let sessions_root = surface_sessions_root(&resource, surface)?;
            let mut entries = match self
                .filesystem
                .list_dir_bounded(
                    &resource,
                    &sessions_root,
                    MAX_OWNER_SESSION_ROOTS_PER_SURFACE.saturating_add(1),
                )
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => return Err(fs_error(error)),
            };
            if entries.len() > MAX_OWNER_SESSION_ROOTS_PER_SURFACE {
                return Err(AuthProductError::BackendUnavailable);
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            for entry in entries {
                if entry.file_type != FileType::Directory {
                    continue;
                }
                let session_id = match AuthSessionId::new(entry.name) {
                    Ok(session_id) => session_id,
                    Err(_) if mode == AccountEnumerationMode::BestEffortKeepalive => continue,
                    Err(_) => return Err(AuthProductError::BackendUnavailable),
                };
                scopes.push(
                    ironclaw_auth::AuthProductScope::new(resource.clone(), surface)
                        .with_session_id(session_id),
                );
            }
        }
        Ok(scopes)
    }

    async fn account_records_for_owner(
        &self,
        owner: &CredentialAccountOwnerScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        self.account_records_for_owner_filtered(owner, |_| true)
            .await
    }

    async fn account_records_for_owner_filtered<P>(
        &self,
        owner: &CredentialAccountOwnerScope,
        predicate: P,
    ) -> Result<Vec<CredentialAccount>, AuthProductError>
    where
        P: Fn(&CredentialAccount) -> bool,
    {
        let mut accounts = Vec::new();
        for scope in self.account_scopes_for_owner(owner).await? {
            accounts.extend(
                self.account_records_under_scope_root_filtered_with_limit(
                    &scope,
                    Some(MAX_OWNER_RECORDS_PER_ROOT),
                    |account| owner.matches(account) && predicate(account),
                )
                .await?,
            );
        }
        accounts.sort_by_key(|account| account.id);
        accounts.dedup_by_key(|account| account.id);
        Ok(accounts)
    }

    async fn select_configured_account_for_owner(
        &self,
        request: CredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        let owner = CredentialAccountOwnerScope::from_scope(&request.scope);
        let mut saw_configured = false;
        let mut selected = None;
        for scope in self.account_scopes_for_owner(&owner).await? {
            for account in self
                .account_records_under_scope_root_with_limit(
                    &scope,
                    Some(MAX_OWNER_RECORDS_PER_ROOT),
                )
                .await?
            {
                if !owner.matches(&account)
                    || account.provider != request.provider
                    || account.status != CredentialAccountStatus::Configured
                {
                    continue;
                }
                saw_configured = true;
                if !account.is_authorized_for_requester(request.requester_extension.as_ref()) {
                    continue;
                }
                if selected.is_some() {
                    return Err(AuthProductError::AccountSelectionRequired);
                }
                selected = Some(account);
            }
        }
        match (selected, saw_configured) {
            (Some(account), _) => Ok(account),
            (None, true) => Err(AuthProductError::CrossScopeDenied),
            (None, false) => Err(AuthProductError::CredentialMissing),
        }
    }

    async fn discovery_entries(
        &self,
        path: &VirtualPath,
        mode: AccountEnumerationMode,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, AuthProductError> {
        let Some(root) = &self.root else {
            return match mode {
                AccountEnumerationMode::BestEffortKeepalive => Ok(Vec::new()),
                AccountEnumerationMode::StrictMigration => {
                    Err(AuthProductError::BackendUnavailable)
                }
            };
        };
        let mut entries = match root
            .list_dir_bounded(
                path,
                MAX_ACCOUNT_DISCOVERY_ENTRIES_PER_ROOT.saturating_add(1),
            )
            .await
        {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => {
                if mode == AccountEnumerationMode::StrictMigration {
                    return Err(fs_error(error));
                }
                tracing::debug!(%error, path = %path, "account keepalive: discovery subtree skipped");
                return Ok(Vec::new());
            }
        };
        if entries.len() > MAX_ACCOUNT_DISCOVERY_ENTRIES_PER_ROOT {
            return match mode {
                AccountEnumerationMode::BestEffortKeepalive => Ok(Vec::new()),
                AccountEnumerationMode::StrictMigration => {
                    Err(AuthProductError::BackendUnavailable)
                }
            };
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    fn discovery_path(
        raw: &str,
        mode: AccountEnumerationMode,
    ) -> Result<Option<VirtualPath>, AuthProductError> {
        match VirtualPath::new(raw) {
            Ok(path) => Ok(Some(path)),
            Err(_) if mode == AccountEnumerationMode::BestEffortKeepalive => Ok(None),
            Err(_) => Err(AuthProductError::BackendUnavailable),
        }
    }

    fn parse_discovery_id<T>(
        result: Result<T, ironclaw_host_api::HostApiError>,
        mode: AccountEnumerationMode,
    ) -> Result<Option<T>, AuthProductError> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(_) if mode == AccountEnumerationMode::BestEffortKeepalive => Ok(None),
            Err(_) => Err(AuthProductError::BackendUnavailable),
        }
    }

    async fn discover_account_owners(
        &self,
        mode: AccountEnumerationMode,
    ) -> Result<Vec<CredentialAccountOwnerScope>, AuthProductError> {
        let Some(tenants_path) = Self::discovery_path("/tenants", mode)? else {
            return Ok(Vec::new());
        };
        let tenant_entries = self.discovery_entries(&tenants_path, mode).await?;
        let mut owners = Vec::new();
        for tenant_entry in tenant_entries {
            if tenant_entry.file_type != FileType::Directory {
                continue;
            }
            let Some(tenant_id) =
                Self::parse_discovery_id(TenantId::new(&tenant_entry.name), mode)?
            else {
                continue;
            };
            let Some(users_path) =
                Self::discovery_path(&format!("/tenants/{}/users", tenant_entry.name), mode)?
            else {
                continue;
            };
            for user_entry in self.discovery_entries(&users_path, mode).await? {
                if user_entry.file_type != FileType::Directory {
                    continue;
                }
                let Some(user_id) = Self::parse_discovery_id(UserId::new(&user_entry.name), mode)?
                else {
                    continue;
                };
                owners.push(CredentialAccountOwnerScope {
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                    agent_id: None,
                    project_id: None,
                    mission_id: None,
                    thread_id: None,
                    session_id: None,
                });

                let agents_raw = format!(
                    "/tenants/{}/users/{}/secrets/agents",
                    tenant_entry.name, user_entry.name
                );
                let Some(agents_path) = Self::discovery_path(&agents_raw, mode)? else {
                    continue;
                };
                for agent_entry in self.discovery_entries(&agents_path, mode).await? {
                    if agent_entry.file_type != FileType::Directory {
                        continue;
                    }
                    let Some(agent_id) =
                        Self::parse_discovery_id(AgentId::new(&agent_entry.name), mode)?
                    else {
                        continue;
                    };
                    owners.push(CredentialAccountOwnerScope {
                        tenant_id: tenant_id.clone(),
                        user_id: user_id.clone(),
                        agent_id: Some(agent_id.clone()),
                        project_id: None,
                        mission_id: None,
                        thread_id: None,
                        session_id: None,
                    });
                    let Some(projects_path) = Self::discovery_path(
                        &format!("{}/{}/projects", agents_raw, agent_entry.name),
                        mode,
                    )?
                    else {
                        continue;
                    };
                    for project_entry in self.discovery_entries(&projects_path, mode).await? {
                        if project_entry.file_type != FileType::Directory {
                            continue;
                        }
                        let Some(project_id) =
                            Self::parse_discovery_id(ProjectId::new(&project_entry.name), mode)?
                        else {
                            continue;
                        };
                        owners.push(CredentialAccountOwnerScope {
                            tenant_id: tenant_id.clone(),
                            user_id: user_id.clone(),
                            agent_id: Some(agent_id.clone()),
                            project_id: Some(project_id),
                            mission_id: None,
                            thread_id: None,
                            session_id: None,
                        });
                    }
                }

                let Some(projects_path) = Self::discovery_path(
                    &format!(
                        "/tenants/{}/users/{}/secrets/projects",
                        tenant_entry.name, user_entry.name
                    ),
                    mode,
                )?
                else {
                    continue;
                };
                for project_entry in self.discovery_entries(&projects_path, mode).await? {
                    if project_entry.file_type != FileType::Directory {
                        continue;
                    }
                    let Some(project_id) =
                        Self::parse_discovery_id(ProjectId::new(&project_entry.name), mode)?
                    else {
                        continue;
                    };
                    owners.push(CredentialAccountOwnerScope {
                        tenant_id: tenant_id.clone(),
                        user_id: user_id.clone(),
                        agent_id: None,
                        project_id: Some(project_id),
                        mission_id: None,
                        thread_id: None,
                        session_id: None,
                    });
                }
            }
        }
        Ok(owners)
    }

    fn validate_located_account(
        located_scope: &ironclaw_auth::AuthProductScope,
        path: &ScopedPath,
        located_id: CredentialAccountId,
        account: &CredentialAccount,
    ) -> Result<(), AuthProductError> {
        let located_resource = &located_scope.resource;
        let embedded_resource = &account.scope.resource;
        if account.id != located_id
            || embedded_resource.tenant_id != located_resource.tenant_id
            || embedded_resource.user_id != located_resource.user_id
            || embedded_resource.agent_id != located_resource.agent_id
            || embedded_resource.project_id != located_resource.project_id
            || account.scope.surface != located_scope.surface
            || account.scope.session_id != located_scope.session_id
            || account_path(located_scope, located_id)? != *path
        {
            return Err(AuthProductError::BackendUnavailable);
        }
        Ok(())
    }

    async fn strict_migration_accounts_for_owner(
        &self,
        owner: &CredentialAccountOwnerScope,
    ) -> Result<Vec<LocatedCredentialAccount>, AuthProductError> {
        let mut located = Vec::new();
        for scope in self
            .account_scopes_for_owner_with_mode(owner, AccountEnumerationMode::StrictMigration)
            .await?
        {
            let root = account_root(&scope)?;
            let mut entries = match self
                .filesystem
                .list_dir_bounded(
                    &scope.resource,
                    &root,
                    MAX_OWNER_RECORDS_PER_ROOT.saturating_add(1),
                )
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => return Err(fs_error(error)),
            };
            if entries.len() > MAX_OWNER_RECORDS_PER_ROOT {
                return Err(AuthProductError::BackendUnavailable);
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            for entry in entries {
                let Some(raw_id) = entry.name.strip_suffix(".json") else {
                    continue;
                };
                if entry.file_type != FileType::File {
                    return Err(AuthProductError::BackendUnavailable);
                }
                let located_id = uuid::Uuid::parse_str(raw_id)
                    .map(CredentialAccountId::from_uuid)
                    .map_err(|_| AuthProductError::BackendUnavailable)?;
                let path = join_scoped(&root, &entry.name)?;
                let Some((account, version)) = self
                    .read_record::<CredentialAccount>(&scope.resource, &path)
                    .await?
                else {
                    return Err(AuthProductError::BackendUnavailable);
                };
                Self::validate_located_account(&scope, &path, located_id, &account)?;
                if account.provider.as_str() == "slack_personal" {
                    located.push(LocatedCredentialAccount {
                        located_scope: scope.clone(),
                        path,
                        account,
                        version,
                    });
                }
            }
        }
        Ok(located)
    }

    /// Enumerate configured Google OAuth accounts with refresh-secret handles.
    ///
    /// This operational sweep is deliberately best-effort: a malformed or
    /// unavailable subtree is skipped so another owner can still be refreshed.
    /// Filtering happens within each owner traversal before records are added to
    /// the global candidate set. The returned records contain opaque secret
    /// handles only; callers must not log or serialize them.
    pub(crate) async fn list_refresh_candidates(&self) -> Vec<CredentialAccount> {
        let owners = match self
            .discover_account_owners(AccountEnumerationMode::BestEffortKeepalive)
            .await
        {
            Ok(owners) => owners,
            Err(error) => {
                tracing::debug!(%error, "account keepalive: owner discovery failed");
                return Vec::new();
            }
        };
        let mut candidates = Vec::new();
        for owner in owners {
            match self
                .account_records_for_owner_filtered(&owner, |account| {
                    account.provider.as_str() == ironclaw_auth::GOOGLE_PROVIDER_ID
                        && account.status == CredentialAccountStatus::Configured
                        && account.refresh_secret.is_some()
                })
                .await
            {
                Ok(records) => candidates.extend(records),
                Err(error) => {
                    tracing::debug!(%error, "account keepalive: owner subtree skipped");
                }
            }
        }
        candidates.sort_by_key(|account| account.id);
        candidates.dedup_by_key(|account| account.id);
        candidates
    }

    /// One-time forward migration (NEA-25 unified Slack extension): the Slack
    /// user-OAuth credential authority was renamed from `slack_personal` to
    /// `slack` when the Slack channel and tools unified under one extension
    /// identity. Rewrites persisted credential-account records in place and
    /// returns the number migrated. Idempotent: after the first run no record
    /// matches, so subsequent boots rewrite nothing. This is a data
    /// migration executed at composition build, not a runtime alias — no code
    /// path resolves the retired provider id.
    pub(crate) async fn migrate_retired_slack_personal_provider(
        &self,
    ) -> Result<usize, AuthProductError> {
        let mut retired = Vec::new();
        for owner in self
            .discover_account_owners(AccountEnumerationMode::StrictMigration)
            .await?
        {
            retired.extend(self.strict_migration_accounts_for_owner(&owner).await?);
        }
        let unified_provider = ironclaw_auth::AuthProviderId::new("slack")
            .map_err(|_| AuthProductError::BackendUnavailable)?;
        let mut migrated = 0usize;
        for located in retired {
            let resource = located.located_scope.resource.clone();
            let located_scope = located.located_scope;
            let path = located.path;
            let located_id = located.account.id;
            let absent_snapshot = located.account;
            let provider = unified_provider.clone();
            let validation_path = path.clone();
            migrated += cas_update(
                &self.filesystem,
                &resource,
                &path,
                |body| {
                    serde_json::from_slice::<CredentialAccount>(body)
                        .map_err(|_| AuthProductError::BackendUnavailable)
                },
                |next| {
                    let body = serde_json::to_vec(next)
                        .map_err(|_| AuthProductError::BackendUnavailable)?;
                    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
                },
                move |current| {
                    let mut snapshot = match current {
                        Some(snapshot) => snapshot,
                        None => {
                            return std::future::ready(Ok(CasApply::no_op(
                                absent_snapshot.clone(),
                                0,
                            )));
                        }
                    };
                    if let Err(error) = Self::validate_located_account(
                        &located_scope,
                        &validation_path,
                        located_id,
                        &snapshot,
                    ) {
                        return std::future::ready(Err(error));
                    }
                    if snapshot.provider.as_str() != "slack_personal" {
                        return std::future::ready(Ok(CasApply::no_op(snapshot, 0)));
                    }
                    snapshot.provider = provider.clone();
                    std::future::ready(Ok(CasApply::new(snapshot, 1)))
                },
            )
            .await
            .map_err(migration_cas_error)?;
        }
        Ok(migrated)
    }

    async fn create_account_with_id(
        &self,
        account_id: CredentialAccountId,
        request: NewCredentialAccount,
        cas: CasExpectation,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.create_account_with_id_and_provider_identity(account_id, request, None, cas)
            .await
    }

    async fn create_account_with_id_and_provider_identity(
        &self,
        account_id: CredentialAccountId,
        request: NewCredentialAccount,
        provider_identity: Option<ironclaw_auth::OAuthProviderIdentity>,
        cas: CasExpectation,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.create_account_with_id_and_provider_identity_versioned(
            account_id,
            request,
            provider_identity,
            cas,
        )
        .await
        .map(|(account, _)| account)
    }

    async fn create_account_with_id_and_provider_identity_versioned(
        &self,
        account_id: CredentialAccountId,
        request: NewCredentialAccount,
        provider_identity: Option<ironclaw_auth::OAuthProviderIdentity>,
        cas: CasExpectation,
    ) -> Result<(CredentialAccount, RecordVersion), AuthProductError> {
        validate_new_credential_account(&request)?;
        let now = Utc::now();
        let account = CredentialAccount {
            id: account_id,
            scope: request.scope,
            provider: request.provider,
            label: request.label,
            status: request.status,
            ownership: request.ownership,
            owner_extension: request.owner_extension,
            granted_extensions: request.granted_extensions,
            access_secret: request.access_secret,
            refresh_secret: request.refresh_secret,
            scopes: request.scopes,
            provider_identity,
            created_at: now,
            updated_at: now,
        };
        let version = self.write_account(&account, cas).await?;
        Ok((account, version))
    }
}

use ironclaw_auth::{credential_status_for_completed_flow, is_terminal_status, scope_matches};
