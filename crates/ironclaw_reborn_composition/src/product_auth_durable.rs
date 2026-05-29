use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use futures::{StreamExt as _, TryStreamExt as _, stream};

use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_secrets::SecretStore;
use serde::{Serialize, de::DeserializeOwned};

use ironclaw_auth::{
    AuthFlowId, AuthFlowRecord, AuthProductError, CredentialAccount, CredentialAccountId,
    NewCredentialAccount,
};

use self::broker_projection::{BrokerAccountProjector, CredentialBrokerProjector};
use self::domain::validate_new_credential_account;
use self::paths::{account_path, account_root, flow_path, fs_error, join_scoped};

mod accounts;
mod broker_projection;
mod cleanup;
mod domain;
mod flows;
mod interactions;
mod paths;
mod provider;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use broker_projection::NoopBrokerAccountProjector;
pub(crate) use provider::UnavailableAuthProviderClient;

/// Construct a default product-auth broker projector wired to the
/// supplied [`CredentialAccountStore`].  Composition callers use this
/// to wire the runtime credential broker as the projection sink; tests
/// and substrate-only configurations may pass
/// [`NoopBrokerAccountProjector`] instead.
pub(crate) fn default_broker_projector(
    store: Arc<dyn ironclaw_secrets::CredentialAccountStore>,
) -> Arc<dyn BrokerAccountProjector> {
    Arc::new(CredentialBrokerProjector::new(store))
}

/// Durable production implementation of the product-auth ports.
///
/// Records live under the caller's scoped `/secrets/product-auth` tree. Raw
/// provider tokens and manual token values are stored only through
/// [`SecretStore`] and represented here by opaque secret handles.
//
// Two `CredentialAccount` records coexist:
//   * `ironclaw_auth::CredentialAccount` — product-auth UX record stored
//     here (provider id, label, owner_extension, grants, status,
//     provider_scopes, access/refresh secret handles).  Read/written by
//     setup, OAuth callback, manual-token submit, uninstall cleanup.
//   * `ironclaw_secrets::CredentialAccount` — runtime broker record
//     consumed on every extension HTTP call to issue
//     `CredentialSessionRequest`s (invocation_id, capability_id,
//     extension_id, method, url, expires_at, max_uses).
//
// They are deliberately separate stores (see
// `docs/reborn/contracts/auth-product.md` → "Durable Production Slice")
// because their consumers, lifecycles, and access patterns differ.
// Drift is now policed by a one-way projection: every successful
// `write_account` call is followed by a best-effort broker projection
// through `broker_projection::BrokerAccountProjector`.  Projection
// failure is logged at `warn` and does not block the product-auth flow
// — product-auth remains source of truth.  See the
// `broker_projection` module docs for known gaps (allowed_targets,
// multi-extension grants, broker delete semantics).
pub(crate) struct FilesystemAuthProductServices<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    secret_store: Arc<dyn SecretStore>,
    locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    broker_projector: Arc<dyn BrokerAccountProjector>,
}

impl<F> FilesystemAuthProductServices<F>
where
    F: RootFilesystem,
{
    pub(crate) fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        secret_store: Arc<dyn SecretStore>,
        broker_projector: Arc<dyn BrokerAccountProjector>,
    ) -> Self {
        Self {
            filesystem,
            secret_store,
            locks: Mutex::new(HashMap::new()),
            broker_projector,
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

    async fn read_account(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
        account_id: CredentialAccountId,
    ) -> Result<Option<(CredentialAccount, RecordVersion)>, AuthProductError> {
        self.read_record(&scope.resource, &account_path(scope, account_id)?)
            .await
    }

    /// Persist `account` to the product-auth durable store and project
    /// the post-write state into the runtime credential broker.
    ///
    /// **Projection contract:** every successful product-auth account
    /// write must invoke the broker projector exactly once with the
    /// persisted record.  Projection is best-effort (see
    /// [`broker_projection`] module docs); failures are logged but do
    /// not fail the product-auth write.  Callers therefore see this
    /// method behave identically to a plain `write_record` — the
    /// projection is a side effect, not a contract obligation.
    ///
    /// All call sites within this module must route through
    /// `write_account` rather than calling `write_record` against an
    /// account path directly, so the projection chokepoint stays
    /// single-source.
    async fn write_account(
        &self,
        account: &CredentialAccount,
        cas: CasExpectation,
    ) -> Result<RecordVersion, AuthProductError> {
        let version = self
            .write_record(
                &account.scope.resource,
                &account_path(&account.scope, account.id)?,
                account,
                cas,
            )
            .await?;
        self.broker_projector.project_account(account).await;
        Ok(version)
    }

    /// Returns all credential accounts for `scope`, reading records concurrently.
    async fn accounts_for_scope(
        &self,
        scope: &ironclaw_auth::AuthProductScope,
    ) -> Result<Vec<CredentialAccount>, AuthProductError> {
        let root = account_root(scope)?;
        let entries = match self.filesystem.list_dir(&scope.resource, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_error(error)),
        };
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
                        self.read_record::<CredentialAccount>(&scope.resource, &path)
                            .await
                    }
                }),
        )
        .buffer_unordered(MAX_CONCURRENT_READS)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .flatten()
        .filter_map(|(account, _)| scope_matches(scope, &account.scope).then_some(account))
        .collect();
        accounts.sort_by_key(|account| account.id);
        Ok(accounts)
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
