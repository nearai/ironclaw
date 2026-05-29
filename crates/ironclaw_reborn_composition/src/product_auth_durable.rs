use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

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
    CredentialAccountStatus, NewCredentialAccount,
};

use self::domain::validate_new_credential_account;
use self::paths::{account_path, account_root, flow_path, fs_error, join_scoped};

mod accounts;
mod cleanup;
mod domain;
mod flows;
mod interactions;
mod paths;
mod provider;
#[cfg(test)]
mod tests;

pub(crate) use provider::UnavailableAuthProviderClient;

/// Durable production implementation of the product-auth ports.
///
/// Records live under the caller's scoped `/secrets/product-auth` tree. Raw
/// provider tokens and manual token values are stored only through
/// [`SecretStore`] and represented here by opaque secret handles.
pub(crate) struct FilesystemAuthProductServices<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
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
        let mut accounts = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let path = join_scoped(&root, &entry.name)?;
            let Some((account, _)) = self
                .read_record::<CredentialAccount>(&scope.resource, &path)
                .await?
            else {
                continue;
            };
            if scope_matches(scope, &account.scope) {
                accounts.push(account);
            }
        }
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

fn scope_matches(
    left: &ironclaw_auth::AuthProductScope,
    right: &ironclaw_auth::AuthProductScope,
) -> bool {
    left == right
}

fn is_terminal_status(status: ironclaw_auth::AuthFlowStatus) -> bool {
    matches!(
        status,
        ironclaw_auth::AuthFlowStatus::Completed
            | ironclaw_auth::AuthFlowStatus::Failed
            | ironclaw_auth::AuthFlowStatus::Expired
            | ironclaw_auth::AuthFlowStatus::Canceled
    )
}

fn credential_status_for_completed_flow() -> CredentialAccountStatus {
    CredentialAccountStatus::Configured
}
