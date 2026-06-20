#![allow(
    dead_code,
    reason = "durable product-auth is staged for production/webui composition; clippy can check this crate before those callers are enabled"
)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use futures::{StreamExt as _, TryStreamExt as _, stream};

use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FileType, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId};
use ironclaw_secrets::SecretStore;
use serde::{Serialize, de::DeserializeOwned};

use ironclaw_auth::{
    AuthFlowId, AuthFlowOwnerScope, AuthFlowRecord, AuthProductError, AuthSessionId, AuthSurface,
    CredentialAccount, CredentialAccountId, CredentialAccountOwnerScope,
    CredentialAccountSelectionRequest, CredentialAccountStatus, NewCredentialAccount,
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
        }
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
        let mut flows = Vec::new();
        for surface in AuthSurface::ALL {
            let scope = ironclaw_auth::AuthProductScope::new(resource.clone(), surface);
            flows.extend(
                self.flow_records_under_scope_root(&scope)
                    .await?
                    .into_iter()
                    .map(|(flow, _)| flow)
                    .filter(|flow| owner.matches(flow)),
            );
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
                let Ok(session_id) = AuthSessionId::new(entry.name) else {
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
                        .filter(|flow| owner.matches(flow)),
                );
            }
        }
        flows.sort_by_key(|flow| flow.id);
        flows.dedup_by_key(|flow| flow.id);
        Ok(flows)
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
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .collect();
        accounts.sort_by_key(|account| account.id);
        Ok(accounts)
    }

    async fn account_scopes_for_owner(
        &self,
        owner: &CredentialAccountOwnerScope,
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
                let Ok(session_id) = AuthSessionId::new(entry.name) else {
                    continue;
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
        let mut accounts = Vec::new();
        for scope in self.account_scopes_for_owner(owner).await? {
            accounts.extend(
                self.account_records_under_scope_root_with_limit(
                    &scope,
                    Some(MAX_OWNER_RECORDS_PER_ROOT),
                )
                .await?
                .into_iter()
                .filter(|account| owner.matches(account)),
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

    /// Enumerate all Google OAuth accounts eligible for proactive keepalive
    /// refresh across all tenants, users, agents, and projects.
    ///
    /// Filters in-memory to provider == `GOOGLE_PROVIDER_ID`, status ==
    /// `Configured`, and `refresh_secret.is_some()`. Idle-threshold filtering
    /// (by `updated_at`) is left to the caller (the credential-refresh worker).
    /// Returns an empty vec when the root filesystem was not wired (local-dev /
    /// test path). The returned `CredentialAccount` records carry the
    /// `access_secret`/`refresh_secret` *handles* (opaque references, never the
    /// raw token material) because the worker needs them to drive the refresh.
    /// Callers MUST NOT log or serialize these records; only the handle is ever
    /// present, and it must stay internal to the refresh path.
    ///
    /// # Owner-scope enumeration
    ///
    /// The method mirrors every path shape that `product_auth_base_root` in
    /// `paths.rs` can produce, ensuring no subtree is missed:
    ///
    /// - plain:           `/secrets/product-auth`
    /// - agent-only:      `/secrets/agents/<a>/product-auth`
    /// - agent+project:   `/secrets/agents/<a>/projects/<p>/product-auth`
    /// - project-only:    `/secrets/projects/<p>/product-auth`
    ///
    /// For each discovered owner scope, the canonical `account_records_for_owner`
    /// reader is reused (it already enumerates surfaces + sessions, applies the
    /// per-root record cap, and deduplicates). This function then filters to
    /// Google + Configured + has refresh secret and deduplicates the combined set.
    ///
    /// Per-directory and per-owner errors are silently skipped (annotated below)
    /// so one bad subtree never aborts the sweep.
    pub(crate) async fn list_refresh_candidates(&self) -> Vec<CredentialAccount> {
        let Some(root) = &self.root else {
            // Local-dev / test path: no root wired, nothing to enumerate.
            return Vec::new();
        };

        // Walk /tenants → /tenants/<t>/users to discover (tenant, user) pairs.
        let tenants_path = match VirtualPath::new("/tenants") {
            Ok(p) => p,
            Err(error) => {
                tracing::debug!(%error, "list_refresh_candidates: /tenants is not a valid virtual path");
                return Vec::new();
            }
        };
        let tenant_entries = match root.list_dir(&tenants_path).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. } | FilesystemError::Unsupported { .. }) => {
                return Vec::new();
            }
            Err(error) => {
                tracing::debug!(%error, "list_refresh_candidates: failed to list /tenants");
                return Vec::new();
            }
        };

        let mut candidates = Vec::new();
        for tenant_entry in tenant_entries {
            if tenant_entry.file_type != FileType::Directory {
                continue;
            }
            let Ok(tenant_id) = TenantId::new(&tenant_entry.name) else {
                continue; // silent-ok: unparseable tenant directory name; skip
            };
            let users_path_str = format!("/tenants/{}/users", tenant_entry.name);
            let users_path = match VirtualPath::new(&users_path_str) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let user_entries = match root.list_dir(&users_path).await {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. } | FilesystemError::Unsupported { .. }) => {
                    continue;
                }
                Err(error) => {
                    tracing::debug!(
                        tenant = %tenant_entry.name,
                        %error,
                        "list_refresh_candidates: failed to list users for tenant"
                    );
                    continue;
                }
            };
            for user_entry in user_entries {
                if user_entry.file_type != FileType::Directory {
                    continue;
                }
                let Ok(user_id) = UserId::new(&user_entry.name) else {
                    continue; // silent-ok: unparseable user directory name; skip
                };

                // Collect every owner scope for this (tenant, user):
                //   1. plain (no agent, no project)
                //   2. for each agent dir: agent-only
                //   3. for each agent+project dir: agent+project
                //   4. for each project dir (top-level): project-only
                let mut owner_scopes: Vec<CredentialAccountOwnerScope> = Vec::new();

                // 1. Plain user scope.
                owner_scopes.push(CredentialAccountOwnerScope {
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                    agent_id: None,
                    project_id: None,
                    mission_id: None,
                    thread_id: None,
                    session_id: None,
                });

                // 2 + 3. Enumerate /tenants/<t>/users/<u>/secrets/agents/
                let agents_dir = format!(
                    "/tenants/{}/users/{}/secrets/agents",
                    tenant_entry.name, user_entry.name
                );
                if let Ok(agents_path) = VirtualPath::new(&agents_dir) {
                    match root.list_dir(&agents_path).await {
                        Ok(agent_entries) => {
                            for agent_entry in agent_entries {
                                if agent_entry.file_type != FileType::Directory {
                                    continue;
                                }
                                let Ok(agent_id) = AgentId::new(&agent_entry.name) else {
                                    continue; // silent-ok: unparseable agent dir; skip
                                };
                                // 2. Agent-only scope.
                                owner_scopes.push(CredentialAccountOwnerScope {
                                    tenant_id: tenant_id.clone(),
                                    user_id: user_id.clone(),
                                    agent_id: Some(agent_id.clone()),
                                    project_id: None,
                                    mission_id: None,
                                    thread_id: None,
                                    session_id: None,
                                });
                                // 3. Agent+project scopes.
                                let agent_projects_dir =
                                    format!("{}/{}/projects", agents_dir, agent_entry.name);
                                if let Ok(ap_path) = VirtualPath::new(&agent_projects_dir) {
                                    match root.list_dir(&ap_path).await {
                                        Ok(proj_entries) => {
                                            for proj_entry in proj_entries {
                                                if proj_entry.file_type != FileType::Directory {
                                                    continue;
                                                }
                                                let Ok(project_id) =
                                                    ProjectId::new(&proj_entry.name)
                                                else {
                                                    continue; // silent-ok: unparseable project dir; skip
                                                };
                                                owner_scopes.push(CredentialAccountOwnerScope {
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
                                        Err(
                                            FilesystemError::NotFound { .. }
                                            | FilesystemError::Unsupported { .. },
                                        ) => {}
                                        Err(error) => {
                                            tracing::debug!(
                                                tenant = %tenant_entry.name,
                                                user = %user_entry.name,
                                                agent = %agent_entry.name,
                                                %error,
                                                "list_refresh_candidates: failed to list agent/projects dir; skipping"
                                                // silent-ok: one bad agent subtree must not abort the sweep
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        Err(
                            FilesystemError::NotFound { .. } | FilesystemError::Unsupported { .. },
                        ) => {}
                        Err(error) => {
                            tracing::debug!(
                                tenant = %tenant_entry.name,
                                user = %user_entry.name,
                                %error,
                                "list_refresh_candidates: failed to list agents dir; skipping"
                                // silent-ok: one bad user subtree must not abort the sweep
                            );
                        }
                    }
                }

                // 4. Top-level project-only scopes.
                // /tenants/<t>/users/<u>/secrets/projects/
                let projects_dir = format!(
                    "/tenants/{}/users/{}/secrets/projects",
                    tenant_entry.name, user_entry.name
                );
                if let Ok(projects_path) = VirtualPath::new(&projects_dir) {
                    match root.list_dir(&projects_path).await {
                        Ok(proj_entries) => {
                            for proj_entry in proj_entries {
                                if proj_entry.file_type != FileType::Directory {
                                    continue;
                                }
                                let Ok(project_id) = ProjectId::new(&proj_entry.name) else {
                                    continue; // silent-ok: unparseable project dir; skip
                                };
                                owner_scopes.push(CredentialAccountOwnerScope {
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
                        Err(
                            FilesystemError::NotFound { .. } | FilesystemError::Unsupported { .. },
                        ) => {}
                        Err(error) => {
                            tracing::debug!(
                                tenant = %tenant_entry.name,
                                user = %user_entry.name,
                                %error,
                                "list_refresh_candidates: failed to list projects dir; skipping"
                                // silent-ok: one bad user subtree must not abort the sweep
                            );
                        }
                    }
                }

                // For each discovered owner scope, use the canonical reader to
                // enumerate all surfaces + sessions, then filter to keepalive
                // candidates (Google + Configured + has refresh secret).
                for owner in owner_scopes {
                    let records = match self.account_records_for_owner(&owner).await {
                        Ok(r) => r,
                        Err(error) => {
                            tracing::debug!(
                                tenant = %tenant_entry.name,
                                user = %user_entry.name,
                                %error,
                                "list_refresh_candidates: account_records_for_owner failed; skipping owner"
                                // silent-ok: one bad owner subtree must not abort the sweep
                            );
                            continue;
                        }
                    };
                    for account in records {
                        if account.provider.as_str() != ironclaw_auth::GOOGLE_PROVIDER_ID {
                            continue;
                        }
                        if account.status != CredentialAccountStatus::Configured {
                            continue;
                        }
                        if account.refresh_secret.is_none() {
                            continue;
                        }
                        candidates.push(account);
                    }
                }
            }
        }
        // Stable ordering by account id; dedup in case the same account appeared
        // under multiple enumerated owner scopes (e.g. plain + agent-scoped read).
        candidates.sort_by_key(|a| a.id);
        candidates.dedup_by_key(|a| a.id);
        candidates
    }

    async fn create_account_with_id(
        &self,
        account_id: CredentialAccountId,
        request: NewCredentialAccount,
        cas: CasExpectation,
    ) -> Result<CredentialAccount, AuthProductError> {
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
            created_at: now,
            updated_at: now,
        };
        self.write_account(&account, cas).await?;
        Ok(account)
    }
}

use ironclaw_auth::{credential_status_for_completed_flow, is_terminal_status, scope_matches};
