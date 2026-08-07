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
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, UserId},
    path::ScopedPath,
    resource::ResourceScope,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    AuthContinuationRef, AuthFlowId, AuthFlowOwnerScope, AuthFlowRecord, AuthProductError,
    AuthSessionId, AuthSurface, CredentialAccount, CredentialAccountId,
    CredentialAccountOwnerScope, CredentialAccountSelectionRequest, CredentialAccountStatus,
    NewCredentialAccount,
};
use ironclaw_host_api::path::VirtualPath;

use self::domain::validate_new_credential_account;
use self::paths::{
    account_migration_marker_path, account_path, account_root, flow_path, flow_root, fs_error,
    join_scoped, legacy_account_root, legacy_agents_root, legacy_projects_root,
    surface_sessions_root,
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
/// Bound on legacy agent/project directories walked during migration.
const MAX_LEGACY_OWNER_DIRS_PER_LEVEL: usize = 1024;
/// Total directory listings one owner's legacy migration may perform.
///
/// The per-level caps above bound each level but not the walk, whose size is
/// their product: agents x projects x surfaces x sessions. This is the single
/// budget spent across every level, so a pathological layout costs a bounded
/// number of round-trips on a read path a user is waiting on rather than the
/// product of the caps.
const MAX_LEGACY_MIGRATION_LISTINGS: usize = 4096;
const MAX_OWNER_RECORDS_PER_ROOT: usize = 1024;

/// Durable evidence that an owner's accounts have been copied out of the
/// pre-migration roots. Durable rather than process-local so the legacy scan
/// runs once per owner across restarts instead of once per process.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct AccountMigrationMarker {
    migrated: usize,
    /// False when the scan hit its budget and stopped early. Recorded so the
    /// incompleteness is visible rather than inferred from a missing marker.
    #[serde(default = "migration_marker_complete_default")]
    complete: bool,
}

/// Pre-budget markers predate the `complete` flag and always recorded a scan
/// that ran to completion.
fn migration_marker_complete_default() -> bool {
    true
}

fn flow_requires_lifecycle_cleanup(flow: &AuthFlowRecord) -> bool {
    !crate::is_terminal_status(flow.status)
        || (flow.continuation_emitted_at.is_none()
            && matches!(
                flow.continuation,
                AuthContinuationRef::TurnGateResume { .. }
            ))
}

pub use provider::UnavailableAuthProviderClient;

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
//   * `crate::CredentialAccount` — product-auth UX record stored
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
pub struct FilesystemAuthProductServices<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    /// Raw root filesystem held separately for deployment-wide scans (B1).
    ///
    /// `ScopedFilesystem` does not expose its inner `RootFilesystem`, so
    /// this field is wired explicitly by the factory (`new_with_root`).
    /// `None` in test/standalone paths that do not need cross-tenant listing —
    /// `list_refresh_candidates` returns an empty vec in that case (safe: no
    /// accounts are refreshed, which is benign for local/test deployments).
    root: Option<Arc<F>>,
    secret_store: Arc<dyn ironclaw_secrets::SecretStorePort>,
    locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
}

impl<F> FilesystemAuthProductServices<F>
where
    F: RootFilesystem,
{
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        secret_store: Arc<dyn ironclaw_secrets::SecretStorePort>,
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
    pub fn new_with_root(
        filesystem: Arc<ScopedFilesystem<F>>,
        root: Arc<F>,
        secret_store: Arc<dyn ironclaw_secrets::SecretStorePort>,
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

    /// Best-effort secret deletion for rotation/cleanup paths: the handle is
    /// already unreachable via the account record, so a failure only leaves
    /// orphaned material — log it instead of dropping the error silently.
    async fn purge_secret_handle(
        &self,
        scope: &ironclaw_host_api::resource::ResourceScope,
        handle: &ironclaw_host_api::ids::SecretHandle,
    ) {
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
        scope: &crate::AuthProductScope,
        flow_id: AuthFlowId,
    ) -> Result<Option<(AuthFlowRecord, RecordVersion)>, AuthProductError> {
        self.read_record(&scope.resource, &flow_path(scope, flow_id)?)
            .await
    }

    async fn write_flow(
        &self,
        scope: &crate::AuthProductScope,
        record: &AuthFlowRecord,
        cas: CasExpectation,
    ) -> Result<RecordVersion, AuthProductError> {
        self.write_record(&scope.resource, &flow_path(scope, record.id)?, record, cas)
            .await
    }

    async fn flows_for_scope(
        &self,
        scope: &crate::AuthProductScope,
    ) -> Result<Vec<(AuthFlowRecord, RecordVersion)>, AuthProductError> {
        let mut flows = self.flow_records_under_scope_root(scope).await?;
        flows.retain(|(flow, _)| scope_matches(scope, &flow.scope));
        flows.sort_by_key(|(flow, _)| flow.id);
        Ok(flows)
    }

    async fn flow_records_under_scope_root(
        &self,
        scope: &crate::AuthProductScope,
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
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        };
        self.flow_records_for_resource_filtered(&resource, |flow| owner.matches(flow))
            .await
    }

    /// Walks every auth-flow record for a credential owner across all surfaces
    /// and their session sub-roots, returning the ones matching `predicate`.
    ///
    /// Flow storage is keyed by agent/project/surface/session (see `flow_root`),
    /// so an owner-granularity read must enumerate each surface's flow root and
    /// each bounded session sub-root. Extracted so both the owner-scoped read
    /// (`flow_records_for_owner`) and the provider-scoped lifecycle read
    /// (`lifecycle_flows_for_owner_provider`) share one enumeration and differ
    /// only in the predicate — the walk is the security-critical part (a missed
    /// surface/session leaves a pending flow that can mint on a late callback).
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
            let scope = crate::AuthProductScope::new(resource.clone(), surface);
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
                let mut session_scope = crate::AuthProductScope::new(resource.clone(), surface);
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
    /// owner + provider, walked across surfaces/sessions but not one thread
    /// (A3 + F2).
    ///
    /// The lifecycle/disconnect analogue of [`Self::account_records_for_owner`]:
    /// flow storage is keyed by agent/project/surface/session (see `flow_root`)
    /// and never by thread, so a removal or channel disconnect — which arrives on
    /// the `Callback` surface with no thread — can still reach every pending flow
    /// a connect created, including thread-less setup flows and thread-scoped
    /// turn-gate flows. Used by lifecycle cleanup to cancel the removed
    /// provider's non-terminal flows so a late provider callback cannot mint a
    /// credential for a torn-down extension. Provider-agnostic by construction.
    ///
    /// Beyond the non-terminal test, `flow_requires_lifecycle_cleanup` also
    /// re-enumerates terminal flows whose `TurnGateResume` continuation was
    /// never acknowledged, so cleanup can report them for gate denial instead
    /// of leaving their blocked turns parked.
    async fn lifecycle_flows_for_owner_provider(
        &self,
        resource: &ResourceScope,
        provider: &crate::AuthProviderId,
    ) -> Result<Vec<AuthFlowRecord>, AuthProductError> {
        let resource = ResourceScope {
            tenant_id: resource.tenant_id.clone(),
            user_id: resource.user_id.clone(),
            agent_id: resource.agent_id.clone(),
            project_id: resource.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        };
        self.flow_records_for_resource_filtered(&resource, |flow| {
            &flow.provider == provider && flow_requires_lifecycle_cleanup(flow)
        })
        .await
    }

    /// The removed package's own auth flows, walked across surfaces/sessions
    /// like [`Self::lifecycle_flows_for_owner_provider`] but keyed by the
    /// `LifecycleActivation` continuation instead of the provider. Uninstall
    /// deliberately skips the provider selector when the provider is still
    /// used by another installed extension; this selector cancels the removed
    /// extension's own connect flows anyway, so a late callback cannot mint
    /// against — and its failure compensation cannot revoke — the shared
    /// credential.
    async fn lifecycle_flows_for_owner_package(
        &self,
        resource: &ResourceScope,
        package: &crate::LifecyclePackageRef,
    ) -> Result<Vec<AuthFlowRecord>, AuthProductError> {
        let resource = ResourceScope {
            tenant_id: resource.tenant_id.clone(),
            user_id: resource.user_id.clone(),
            agent_id: resource.agent_id.clone(),
            project_id: resource.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        };
        self.flow_records_for_resource_filtered(&resource, |flow| {
            matches!(
                &flow.continuation,
                AuthContinuationRef::LifecycleActivation { package_ref } if package_ref == package
            ) && flow_requires_lifecycle_cleanup(flow)
        })
        .await
    }

    /// Read one account by id, following the same nearest-first chain as
    /// [`Self::account_scopes_for_owner`].
    ///
    /// The fallback is load-bearing, not a convenience: a project caller can
    /// *select* an inherited user-level account for a bind, and the OAuth
    /// callback then reads it back by id under the project scope. Checking only
    /// the project root returned `CredentialMissing` there, so reconnecting an
    /// inherited credential from inside a project failed with the credential
    /// sitting readable one root up. The manual-token bound update reads
    /// through the same path.
    async fn read_account(
        &self,
        scope: &crate::AuthProductScope,
        account_id: CredentialAccountId,
    ) -> Result<Option<(CredentialAccount, RecordVersion)>, AuthProductError> {
        for candidate in
            Self::account_scopes_for_owner(&CredentialAccountOwnerScope::from_scope(scope))
        {
            if let Some(found) = self
                .read_record(
                    &candidate.resource,
                    &account_path(&candidate.resource, account_id)?,
                )
                .await?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    async fn write_account(
        &self,
        account: &CredentialAccount,
        cas: CasExpectation,
    ) -> Result<RecordVersion, AuthProductError> {
        self.write_record(
            &account.scope.resource,
            &account_path(&account.scope.resource, account.id)?,
            account,
            cas,
        )
        .await
    }

    /// Returns all credential accounts for `scope`, reading records concurrently.
    async fn accounts_for_scope(
        &self,
        scope: &crate::AuthProductScope,
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
        scope: &crate::AuthProductScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        self.account_records_under_scope_root_with_limit(scope, None)
            .await
    }

    async fn account_records_under_scope_root_with_limit(
        &self,
        scope: &crate::AuthProductScope,
        max_records: Option<usize>,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        let root = account_root(&scope.resource)?;
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

    fn owner_resource(owner: &CredentialAccountOwnerScope) -> ResourceScope {
        ResourceScope {
            tenant_id: owner.tenant_id.clone(),
            user_id: owner.user_id.clone(),
            // Not part of a credential's address — see
            // `CredentialAccountOwnerScope::matches`.
            agent_id: None,
            project_id: owner.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::ids::InvocationId::new(),
        }
    }

    /// The canonical roots a credential lookup reads, nearest scope first.
    ///
    /// At most two: the caller's project (when it has one) and the user-level
    /// default it inherits from. This replaced a blind fan-out over all seven
    /// `AuthSurface` variants times every session directory — a scan that
    /// existed only because the write side chose a partition the read side
    /// could not predict.
    fn account_scopes_for_owner(
        owner: &CredentialAccountOwnerScope,
    ) -> Vec<crate::AuthProductScope> {
        let resource = Self::owner_resource(owner);
        let mut scopes = vec![crate::AuthProductScope::new(
            resource.clone(),
            AuthSurface::Api,
        )];
        if resource.project_id.is_some() {
            let mut inherited = resource;
            inherited.project_id = None;
            scopes.push(crate::AuthProductScope::new(inherited, AuthSurface::Api));
        }
        scopes
    }

    /// Every pre-migration root an account could be sitting in: agent x project
    /// x surface x session. Read-only, and used only by the one-shot migration
    /// — this is the old steady-state read path, kept as the upgrade path.
    /// Directory names under `root`, or empty when the directory is absent.
    async fn legacy_child_names(
        &self,
        resource: &ResourceScope,
        root: &ScopedPath,
    ) -> Result<Vec<String>, AuthProductError> {
        let entries = match self
            .filesystem
            .list_dir_bounded(
                resource,
                root,
                MAX_LEGACY_OWNER_DIRS_PER_LEVEL.saturating_add(1),
            )
            .await
        {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_error(error)),
        };
        if entries.len() > MAX_LEGACY_OWNER_DIRS_PER_LEVEL {
            return Err(AuthProductError::BackendUnavailable);
        }
        let mut names = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        names.sort();
        Ok(names)
    }

    /// Every `(agent, project)` prefix the legacy layout could have written
    /// under for this user, discovered from disk.
    ///
    /// The reader's own agent is not enough: the whole point of the change is
    /// that a credential minted while agent A was running must be found by
    /// agent B, so the migration enumerates the agent directories instead of
    /// assuming one. Both are scoped inside this user's mount, so this walks
    /// one user's records, never another's.
    async fn legacy_owner_resources(
        &self,
        owner: &CredentialAccountOwnerScope,
        budget: &mut usize,
    ) -> Result<Vec<ResourceScope>, AuthProductError> {
        let base = Self::owner_resource(owner);
        if *budget == 0 {
            return Ok(Vec::new());
        }
        *budget = budget.saturating_sub(1);
        let mut agents: Vec<Option<String>> = vec![None];
        agents.extend(
            self.legacy_child_names(&base, &legacy_agents_root()?)
                .await?
                .into_iter()
                .map(Some),
        );

        let mut resources = Vec::new();
        for agent in agents {
            if *budget == 0 {
                break;
            }
            *budget = budget.saturating_sub(1);
            let mut projects: Vec<Option<String>> = vec![None];
            projects.extend(
                self.legacy_child_names(&base, &legacy_projects_root(agent.as_deref())?)
                    .await?
                    .into_iter()
                    .map(Some),
            );
            for project in projects {
                // silent-ok: a directory name the current validator rejects
                // cannot have been written by a current writer, so it holds
                // nothing to migrate — skip it the way every other directory
                // scan in this file does (see the agent scan in
                // `list_refresh_candidates`). Aborting instead poisoned the
                // whole migration: the marker is written only on completion, so
                // one unparseable directory made every credential read for that
                // user rescan, fail, and report "needs setup" for credentials
                // sitting valid on disk — the exact failure this change exists
                // to remove.
                let agent_id = match &agent {
                    Some(agent) => match AgentId::new(agent.clone()) {
                        Ok(agent_id) => Some(agent_id),
                        Err(error) => {
                            tracing::debug!(%error, "skipping unparseable legacy agent directory");
                            continue;
                        }
                    },
                    None => None,
                };
                let project_id = match &project {
                    Some(project) => match ProjectId::new(project.clone()) {
                        Ok(project_id) => Some(project_id),
                        Err(error) => {
                            tracing::debug!(%error, "skipping unparseable legacy project directory");
                            continue;
                        }
                    },
                    None => None,
                };
                let mut resource = base.clone();
                resource.agent_id = agent_id;
                resource.project_id = project_id;
                resources.push(resource);
            }
        }
        Ok(resources)
    }

    async fn legacy_account_scopes_for_owner(
        &self,
        owner: &CredentialAccountOwnerScope,
        budget: &mut usize,
    ) -> Result<Vec<crate::AuthProductScope>, AuthProductError> {
        let mut scopes = Vec::new();
        for resource in self.legacy_owner_resources(owner, budget).await? {
            if *budget == 0 {
                break;
            }
            scopes.extend(self.legacy_account_scopes_under(resource, budget).await?);
        }
        Ok(scopes)
    }

    async fn legacy_account_scopes_under(
        &self,
        resource: ResourceScope,
        budget: &mut usize,
    ) -> Result<Vec<crate::AuthProductScope>, AuthProductError> {
        let mut scopes = Vec::new();
        for surface in AuthSurface::ALL {
            scopes.push(crate::AuthProductScope::new(resource.clone(), surface));
            // Always enumerate sessions from disk. This inherited the old
            // steady-state read logic, which narrowed to the caller's own
            // session when it had one — correct for a read, wrong for a
            // one-shot migration: the marker is written afterwards, so a
            // session-bound first reader (manual-token completion, or a
            // blocked-turn gate scope) migrated only its own session and then
            // suppressed the walk forever, stranding every other session's
            // credentials permanently. That is the failure this change exists
            // to remove, reintroduced one layer down.
            if *budget == 0 {
                break;
            }
            *budget = budget.saturating_sub(1);
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
                    crate::AuthProductScope::new(resource.clone(), surface)
                        .with_session_id(session_id),
                );
            }
        }
        Ok(scopes)
    }

    /// Copy every account out of the pre-migration roots into the canonical
    /// one, exactly once per owner.
    ///
    /// Copy-forward, never delete: an interrupted run loses nothing and re-runs
    /// cleanly, and the legacy records stay readable if this needs to be rolled
    /// back. They become inert — nothing writes to those roots again — and are
    /// collected separately rather than removed on a read path.
    ///
    /// Everything lands at the **user level**, including records that were
    /// under `/agents/{a}/projects/{p}/`, into the canonical root its OWN
    /// provenance names — a record minted under project P lands in P's root,
    /// one minted with no project lands at user level.
    ///
    /// Deriving the destination from the reading caller instead (an earlier
    /// version of this) copied project A's credential into whichever project
    /// read first, and since ownership no longer compares project, that made it
    /// selectable there. It also made the resulting layout depend on who
    /// happened to read first. Provenance is reader-independent and is what
    /// `write_account` already uses, so read and write agree by construction.
    async fn migrate_legacy_accounts(
        &self,
        owner: &CredentialAccountOwnerScope,
    ) -> Result<(), AuthProductError> {
        // The marker lives at the owner's USER-level root, never the reading
        // caller's project root. One owner gets one marker regardless of who
        // reads first; keying it on the reader would let a project caller and a
        // user-level caller each run the full scan.
        let mut marker_resource = Self::owner_resource(owner);
        marker_resource.project_id = None;
        let marker = account_migration_marker_path(&marker_resource)?;
        if self
            .filesystem
            .get(&marker_resource, &marker)
            .await
            .map_err(fs_error)?
            .is_some()
        {
            return Ok(());
        }

        // Serialize per owner. Without this, concurrent first reads for one
        // owner each walk every legacy root: `CasExpectation::Absent` keeps the
        // records correct, but the duplicated scan is pure cost on a read path
        // a user is waiting on.
        let migration_lock = self.lock_for(format!(
            "account-migration:{}:{}",
            owner.tenant_id.as_str(),
            owner.user_id.as_str()
        ));
        let _migration_guard = migration_lock.lock().await;
        // Re-check under the lock: the writer we queued behind may have just
        // finished the whole migration.
        if self
            .filesystem
            .get(&marker_resource, &marker)
            .await
            .map_err(fs_error)?
            .is_some()
        {
            return Ok(());
        }

        let mut budget = MAX_LEGACY_MIGRATION_LISTINGS;
        let mut migrated = 0usize;
        let mut exhausted = false;
        for legacy_scope in self
            .legacy_account_scopes_for_owner(owner, &mut budget)
            .await?
        {
            if budget == 0 {
                exhausted = true;
                break;
            }
            budget = budget.saturating_sub(1);
            let legacy_root = legacy_account_root(&legacy_scope)?;
            let entries = match self
                .filesystem
                .list_dir_bounded(
                    &legacy_scope.resource,
                    &legacy_root,
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
            for entry in entries {
                if !entry.name.ends_with(".json") {
                    continue;
                }
                let legacy_path = join_scoped(&legacy_root, &entry.name)?;
                let Some(account) = self
                    .read_account_record_for_scan(&legacy_scope.resource, &legacy_path)
                    .await?
                else {
                    continue;
                };
                // The record is copied VERBATIM. Its `scope` is provenance, not
                // identity: it is what locates the account's secret material
                // (`secret_store.metadata(&account.scope.resource, ..)`), which
                // still lives under the agent/project prefix it was written
                // with. Rewriting the scope to match the new path would move
                // the record and orphan its tokens.
                //
                // The destination is derived from that same provenance, NOT
                // from the reading caller. Using the caller's scope copied a
                // project-A credential into whichever project happened to read
                // first — and since ownership no longer compares project, that
                // made project A's credential selectable by project B. Deriving
                // the destination from the record keeps each project's
                // credentials in its own root, and makes the write-back path
                // agree with the read path by construction: `write_account`
                // computes exactly this path from exactly this scope.
                let canonical_path = account_path(&account.scope.resource, account.id)?;
                // Create-if-absent: a record already migrated (or written fresh
                // at the canonical path) always wins over the legacy copy.
                match self
                    .write_record(
                        &account.scope.resource,
                        &canonical_path,
                        &account,
                        CasExpectation::Absent,
                    )
                    .await
                {
                    Ok(_) => migrated = migrated.saturating_add(1),
                    Err(AuthProductError::BackendConflict) => continue,
                    Err(error) => return Err(error),
                }
            }
        }

        if migrated > 0 {
            // `debug!`, not `info!`: this runs from `account_records_for_owner`,
            // which the keepalive sweep calls per owner, so an `info!` here
            // would fire from a background task and corrupt the REPL/TUI
            // (CLAUDE.md, "Logging levels matter for REPL/TUI").
            tracing::debug!(
                migrated,
                "migrated credential accounts to the canonical owner path"
            );
        }
        if exhausted {
            // Converge rather than rescan forever. A layout this large will not
            // shrink between reads, and repeating the walk would make every
            // credential read pay it. `warn!` so an operator sees it; the
            // marker records that the scan was incomplete.
            tracing::warn!(
                migrated,
                "legacy credential migration exceeded its scan budget; \
                 remaining legacy roots were not migrated"
            );
        }
        self.write_record(
            &marker_resource,
            &marker,
            &AccountMigrationMarker {
                migrated,
                complete: !exhausted,
            },
            CasExpectation::Any,
        )
        .await?;
        Ok(())
    }

    async fn account_records_for_owner(
        &self,
        owner: &CredentialAccountOwnerScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        self.migrate_legacy_accounts(owner).await?;
        let mut accounts: Vec<CredentialAccount> = Vec::new();
        // Nearest root wins, PER PROVIDER. The roots are ordered project-then-
        // user, but order alone is not precedence: merging both roots made a
        // project override and the user-level default it overrides look like two
        // candidates for one provider, which resolves as ambiguity (or picks the
        // most recent) instead of letting the project win. Skipping only the
        // providers a nearer root already answered keeps inheritance intact for
        // every other provider — a project that overrides Notion must still
        // inherit GitHub.
        let mut answered: std::collections::BTreeSet<crate::AuthProviderId> =
            std::collections::BTreeSet::new();
        for scope in Self::account_scopes_for_owner(owner) {
            let from_root = self
                .account_records_under_scope_root_with_limit(
                    &scope,
                    Some(MAX_OWNER_RECORDS_PER_ROOT),
                )
                .await?
                .into_iter()
                .filter(|account| owner.matches(account))
                .filter(|account| !answered.contains(&account.provider))
                .collect::<Vec<_>>();
            for account in &from_root {
                answered.insert(account.provider.clone());
            }
            accounts.extend(from_root);
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
        self.migrate_legacy_accounts(&owner).await?;
        let mut saw_configured = false;
        let mut selected = None;
        for scope in Self::account_scopes_for_owner(&owner) {
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

    /// Enumerate all durable credential accounts across all tenants, users,
    /// agents, and projects.
    ///
    /// Eligibility filtering (status, refresh handle) lives in
    /// `list_refresh_candidates`; idle-threshold filtering (by `updated_at`
    /// against the vendor's recipe-declared lifetime) is the engine keepalive
    /// sweep's job. Returns an empty vec when the root filesystem was not
    /// wired (standalone / test path). The returned `CredentialAccount` records
    /// carry the `access_secret`/`refresh_secret` *handles* (opaque
    /// references, never the raw token material) because the refresh path
    /// needs them. Callers MUST NOT log or serialize these records; only the
    /// handle is ever present, and it must stay internal to the refresh path.
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
    /// per-root record cap, and deduplicates). This function deduplicates the
    /// combined set; callers apply eligibility filters on top.
    ///
    /// Per-directory and per-owner errors are silently skipped (annotated below)
    /// so one bad subtree never aborts the sweep.
    pub(crate) async fn sweep_all_accounts(&self) -> Vec<CredentialAccount> {
        let Some(root) = &self.root else {
            // Standalone/test path: no root wired, nothing to enumerate.
            return Vec::new();
        };

        // Walk /tenants → /tenants/<t>/users to discover (tenant, user) pairs.
        let tenants_path = match VirtualPath::new("/tenants") {
            Ok(p) => p,
            Err(error) => {
                tracing::debug!(%error, "account sweep: /tenants is not a valid virtual path");
                return Vec::new();
            }
        };
        let tenant_entries = match root.list_dir(&tenants_path).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. } | FilesystemError::Unsupported { .. }) => {
                return Vec::new();
            }
            Err(error) => {
                tracing::debug!(%error, "account sweep: failed to list /tenants");
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
                        "account sweep: failed to list users for tenant"
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

                // Every owner scope for this (tenant, user): the user
                // itself, plus one per top-level project directory.
                //
                // The agent enumeration this used to do is gone with the
                // `agent_id` field: a credential is owned by a tenant+user (and
                // optionally a project), so an agent subtree names no owner the
                // sweep could refresh. Legacy accounts still sitting under
                // `/secrets/agents/**` are reached by the migration that
                // `account_records_for_owner` runs, not by widening this walk.
                let mut owner_scopes: Vec<CredentialAccountOwnerScope> = Vec::new();
                owner_scopes.push(CredentialAccountOwnerScope {
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                    project_id: None,
                });

                // One scope per top-level project directory.
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
                                    project_id: Some(project_id),
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
                                "account sweep: failed to list projects dir; skipping"
                                // silent-ok: one bad user subtree must not abort the sweep
                            );
                        }
                    }
                }

                // For each discovered owner scope, use the canonical reader to
                // enumerate all surfaces + sessions; callers filter to
                // keepalive candidates (Configured + has refresh secret).
                for owner in owner_scopes {
                    let records = match self.account_records_for_owner(&owner).await {
                        Ok(r) => r,
                        Err(error) => {
                            tracing::debug!(
                                tenant = %tenant_entry.name,
                                user = %user_entry.name,
                                %error,
                                "account sweep: account_records_for_owner failed; skipping owner"
                                // silent-ok: one bad owner subtree must not abort the sweep
                            );
                            continue;
                        }
                    };
                    candidates.extend(records);
                }
            }
        }
        // Stable ordering by account id; dedup in case the same account appeared
        // under multiple enumerated owner scopes (e.g. plain + agent-scoped read).
        candidates.sort_by_key(|a| a.id);
        candidates.dedup_by_key(|a| a.id);
        candidates
    }

    /// Keepalive candidates: Configured accounts with a refresh secret
    /// handle, filtered from the full durable-account sweep. Vendor-blind by
    /// design — idle lifetimes are per-vendor recipe data
    /// (`refresh.keepalive_idle_seconds`) applied by the engine-owned sweep,
    /// never a hardcoded vendor filter here.
    pub(crate) async fn list_refresh_candidates(&self) -> Vec<CredentialAccount> {
        self.sweep_all_accounts()
            .await
            .into_iter()
            .filter(|account| {
                account.status == CredentialAccountStatus::Configured
                    && account.refresh_secret.is_some()
            })
            .collect()
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
        provider_identity: Option<crate::OAuthProviderIdentity>,
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
            provider_identity,
            created_at: now,
            updated_at: now,
        };
        self.write_account(&account, cas).await?;
        Ok(account)
    }
}

use crate::{credential_status_for_completed_flow, is_terminal_status, scope_matches};

/// Production candidate source for the engine keepalive sweep
/// (`crate::keepalive`): the durable store enumerates every
/// `Configured`+refresh account across all owners; the sweep applies each
/// vendor's recipe-declared idle lifetime.
#[async_trait::async_trait]
impl<F> crate::KeepaliveCandidateSource for FilesystemAuthProductServices<F>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    async fn list_keepalive_candidates(&self) -> Vec<CredentialAccount> {
        FilesystemAuthProductServices::list_refresh_candidates(self).await
    }
}
