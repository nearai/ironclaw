//! Durable host state for the Telegram channel host.
//!
//! Tenant-scoped (`/tenant-shared/telegram-*`) because ingress starts before a
//! Telegram actor is bound to a Reborn user. Backed by the same
//! `ScopedFilesystem` plane as the Slack host state (libSQL/Postgres/local
//! disk per host configuration). One struct implements every telegram store
//! trait plus the shared identity-lookup read side.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_common::hashing::sha256_hex;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::channel_identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};
use crate::telegram::telegram_pairing::{
    TelegramBindingError, TelegramDmTarget, TelegramDmTargetStore, TelegramPairingError,
    TelegramPairingRecord, TelegramPairingStore, TelegramUserBindingStore,
};
use crate::telegram::telegram_setup::{
    TelegramInstallationSetup, TelegramInstallationSetupStore, TelegramSetupError,
};

pub(crate) const TELEGRAM_INSTALLATION_SETUP_PATH: &str =
    "/tenant-shared/telegram-setup/installation.json";
const TELEGRAM_PAIRING_CODE_ROOT: &str = "/tenant-shared/telegram-pairing/codes";
const TELEGRAM_PAIRING_USER_ROOT: &str = "/tenant-shared/telegram-pairing/users";
const TELEGRAM_BINDING_ROOT: &str = "/tenant-shared/telegram-binding/identities";
const TELEGRAM_BINDING_USER_ROOT: &str = "/tenant-shared/telegram-binding/users";
const TELEGRAM_DM_TARGET_ROOT: &str = "/tenant-shared/telegram-dm-targets";
const PATH_HASH_LEN: usize = 24;

pub(crate) struct FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl<F> Clone for FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    fn clone(&self) -> Self {
        Self {
            filesystem: Arc::clone(&self.filesystem),
            scope: self.scope.clone(),
            locks: Arc::clone(&self.locks),
        }
    }
}

impl<F> std::fmt::Debug for FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemTelegramHostState")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl<F> FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            filesystem,
            scope: ResourceScope {
                tenant_id,
                user_id,
                agent_id: Some(agent_id),
                project_id,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            locks: Arc::new(Mutex::new(HashMap::new())),
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
        path: &ScopedPath,
    ) -> Result<Option<(T, RecordVersion)>, FilesystemError>
    where
        T: DeserializeOwned,
    {
        let Some(versioned) = self.filesystem.get(&self.scope, path).await? else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&versioned.entry.body).map_err(|_| {
            FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::ReadFile,
                reason: "Telegram host-state record is invalid JSON".into(),
            }
        })?;
        Ok(Some((value, versioned.version)))
    }

    async fn write_record<T>(
        &self,
        path: &ScopedPath,
        value: &T,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError>
    where
        T: Serialize,
    {
        let body =
            serde_json::to_vec(value).map_err(|_| FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::WriteFile,
                reason: "Telegram host-state record could not be serialized".into(),
            })?;
        self.filesystem
            .put(
                &self.scope,
                path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                cas,
            )
            .await
    }

    async fn delete_record(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        self.filesystem.delete(&self.scope, path).await
    }

    fn setup_path() -> Result<ScopedPath, FilesystemError> {
        scoped_path(TELEGRAM_INSTALLATION_SETUP_PATH.to_string())
    }

    fn pairing_code_path(code: &str) -> Result<ScopedPath, FilesystemError> {
        scoped_path(format!(
            "{TELEGRAM_PAIRING_CODE_ROOT}/{}.json",
            code.to_ascii_uppercase()
        ))
    }

    fn pairing_user_path(user_id: &UserId) -> Result<ScopedPath, FilesystemError> {
        scoped_path(format!(
            "{TELEGRAM_PAIRING_USER_ROOT}/{}.json",
            hashed_segment(user_id.as_str())
        ))
    }

    fn binding_path(provider_user_id: &str) -> Result<ScopedPath, FilesystemError> {
        scoped_path(format!(
            "{TELEGRAM_BINDING_ROOT}/{}.json",
            hashed_segment(provider_user_id)
        ))
    }

    fn binding_user_index_path(user_id: &UserId) -> Result<ScopedPath, FilesystemError> {
        scoped_path(format!(
            "{TELEGRAM_BINDING_USER_ROOT}/{}.json",
            hashed_segment(user_id.as_str())
        ))
    }

    fn dm_target_path(
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(format!(
            "{TELEGRAM_DM_TARGET_ROOT}/{}/{}.json",
            hashed_segment(installation_id.as_str()),
            hashed_segment(user_id.as_str())
        ))
    }
}

fn scoped_path(path: String) -> Result<ScopedPath, FilesystemError> {
    ScopedPath::new(path).map_err(|_| FilesystemError::BackendInfrastructure {
        operation: FilesystemOperation::ReadFile,
        reason: "Telegram host-state path is invalid".into(),
    })
}

fn hashed_segment(value: &str) -> String {
    let digest = sha256_hex(value.as_bytes());
    digest[..PATH_HASH_LEN].to_string()
}

/// Durable binding record: `{installation}:{telegram_user_id}` → user.
/// `epoch` is the pairing code that created the binding; the actor resolver
/// treats it as the binding epoch so unpair/re-pair invalidates in-flight
/// resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTelegramBinding {
    provider_user_id: String,
    user_id: String,
    epoch: String,
}

/// Per-user index of bound provider ids (bounded: one bot, usually one
/// account) so unpair never scans the identity root.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredTelegramBindingUserIndex {
    provider_user_ids: Vec<String>,
}

/// Per-user pointer at the live pairing code so rotation can invalidate the
/// previous record without scanning the code root.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPairingUserPointer {
    code: String,
}

fn map_fs_setup(error: FilesystemError) -> TelegramSetupError {
    tracing::debug!(%error, "telegram setup store filesystem error");
    TelegramSetupError::StoreUnavailable
}

fn map_fs_pairing(error: FilesystemError) -> TelegramPairingError {
    TelegramPairingError::StoreUnavailable {
        reason: error.to_string(),
    }
}

fn map_fs_binding(error: FilesystemError) -> TelegramBindingError {
    TelegramBindingError::StoreUnavailable {
        reason: error.to_string(),
    }
}

#[async_trait]
impl<F> TelegramInstallationSetupStore for FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn get_telegram_installation_setup(
        &self,
    ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
        let path = Self::setup_path().map_err(map_fs_setup)?;
        Ok(self
            .read_record::<TelegramInstallationSetup>(&path)
            .await
            .map_err(map_fs_setup)?
            .map(|(record, _)| record))
    }

    async fn put_telegram_installation_setup(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<(), TelegramSetupError> {
        let path = Self::setup_path().map_err(map_fs_setup)?;
        let _guard = self.lock_for("telegram-setup".to_string());
        let _held = _guard.lock().await;
        self.write_record(&path, setup, CasExpectation::Any)
            .await
            .map_err(map_fs_setup)?;
        Ok(())
    }

    async fn delete_telegram_installation_setup(&self) -> Result<(), TelegramSetupError> {
        let path = Self::setup_path().map_err(map_fs_setup)?;
        match self.delete_record(&path).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_fs_setup(error)),
        }
    }
}

#[async_trait]
impl<F> TelegramPairingStore for FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn upsert_pending_pairing(
        &self,
        record: TelegramPairingRecord,
    ) -> Result<(), TelegramPairingError> {
        let user_lock = self.lock_for(format!("telegram-pairing:{}", record.user_id.as_str()));
        let _held = user_lock.lock().await;
        let user_path = Self::pairing_user_path(&record.user_id).map_err(map_fs_pairing)?;
        if let Some((pointer, _)) = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
        {
            let previous_path = Self::pairing_code_path(&pointer.code).map_err(map_fs_pairing)?;
            match self.delete_record(&previous_path).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(map_fs_pairing(error)),
            }
        }
        let code_path = Self::pairing_code_path(&record.code).map_err(map_fs_pairing)?;
        self.write_record(&code_path, &record, CasExpectation::Any)
            .await
            .map_err(map_fs_pairing)?;
        self.write_record(
            &user_path,
            &StoredPairingUserPointer {
                code: record.code.to_ascii_uppercase(),
            },
            CasExpectation::Any,
        )
        .await
        .map_err(map_fs_pairing)?;
        Ok(())
    }

    async fn live_pairing_for_code(
        &self,
        code: &str,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
        let path = Self::pairing_code_path(code).map_err(map_fs_pairing)?;
        let record = self
            .read_record::<TelegramPairingRecord>(&path)
            .await
            .map_err(map_fs_pairing)?
            .map(|(record, _)| record);
        Ok(record.filter(|record| record.is_live(Utc::now())))
    }

    async fn live_pairing_for_user(
        &self,
        user_id: &UserId,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
        let user_path = Self::pairing_user_path(user_id).map_err(map_fs_pairing)?;
        let Some((pointer, _)) = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
        else {
            return Ok(None);
        };
        self.live_pairing_for_code(&pointer.code).await
    }

    async fn mark_consumed(&self, code: &str) -> Result<(), TelegramPairingError> {
        let path = Self::pairing_code_path(code).map_err(map_fs_pairing)?;
        let Some((mut record, version)) = self
            .read_record::<TelegramPairingRecord>(&path)
            .await
            .map_err(map_fs_pairing)?
        else {
            return Ok(());
        };
        record.consumed_at = Some(Utc::now());
        self.write_record(&path, &record, CasExpectation::Version(version))
            .await
            .map_err(map_fs_pairing)?;
        Ok(())
    }

    async fn invalidate_for_user(&self, user_id: &UserId) -> Result<(), TelegramPairingError> {
        let user_lock = self.lock_for(format!("telegram-pairing:{}", user_id.as_str()));
        let _held = user_lock.lock().await;
        let user_path = Self::pairing_user_path(user_id).map_err(map_fs_pairing)?;
        let Some((pointer, _)) = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
        else {
            return Ok(());
        };
        let code_path = Self::pairing_code_path(&pointer.code).map_err(map_fs_pairing)?;
        match self.delete_record(&code_path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => return Err(map_fs_pairing(error)),
        }
        match self.delete_record(&user_path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_fs_pairing(error)),
        }
    }
}

#[async_trait]
impl<F> TelegramUserBindingStore for FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn bind_telegram_user(
        &self,
        provider_user_id: &str,
        user_id: &UserId,
        epoch: &str,
    ) -> Result<(), TelegramBindingError> {
        let lock = self.lock_for(format!("telegram-binding:{provider_user_id}"));
        let _held = lock.lock().await;
        let path = Self::binding_path(provider_user_id).map_err(map_fs_binding)?;
        if let Some((existing, _)) = self
            .read_record::<StoredTelegramBinding>(&path)
            .await
            .map_err(map_fs_binding)?
            && existing.user_id != user_id.as_str()
        {
            return Err(TelegramBindingError::AlreadyBoundToOtherUser);
        }
        self.write_record(
            &path,
            &StoredTelegramBinding {
                provider_user_id: provider_user_id.to_string(),
                user_id: user_id.as_str().to_string(),
                epoch: epoch.to_string(),
            },
            CasExpectation::Any,
        )
        .await
        .map_err(map_fs_binding)?;

        let index_path = Self::binding_user_index_path(user_id).map_err(map_fs_binding)?;
        let mut index = self
            .read_record::<StoredTelegramBindingUserIndex>(&index_path)
            .await
            .map_err(map_fs_binding)?
            .map(|(index, _)| index)
            .unwrap_or_default();
        if !index
            .provider_user_ids
            .iter()
            .any(|existing| existing == provider_user_id)
        {
            index.provider_user_ids.push(provider_user_id.to_string());
        }
        self.write_record(&index_path, &index, CasExpectation::Any)
            .await
            .map_err(map_fs_binding)?;
        Ok(())
    }

    async fn unbind_telegram_users_for_user(
        &self,
        user_id: &UserId,
        installation_prefix: &str,
    ) -> Result<Vec<String>, TelegramBindingError> {
        let index_path = Self::binding_user_index_path(user_id).map_err(map_fs_binding)?;
        let Some((index, _)) = self
            .read_record::<StoredTelegramBindingUserIndex>(&index_path)
            .await
            .map_err(map_fs_binding)?
        else {
            return Ok(Vec::new());
        };
        let mut removed = Vec::new();
        let mut retained = Vec::new();
        for provider_user_id in index.provider_user_ids {
            if !provider_user_id.starts_with(installation_prefix) {
                retained.push(provider_user_id);
                continue;
            }
            let path = Self::binding_path(&provider_user_id).map_err(map_fs_binding)?;
            match self.delete_record(&path).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => {
                    removed.push(provider_user_id);
                }
                Err(error) => return Err(map_fs_binding(error)),
            }
        }
        let next = StoredTelegramBindingUserIndex {
            provider_user_ids: retained,
        };
        self.write_record(&index_path, &next, CasExpectation::Any)
            .await
            .map_err(map_fs_binding)?;
        Ok(removed)
    }

    async fn bound_user_for(
        &self,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, TelegramBindingError> {
        let path = Self::binding_path(provider_user_id).map_err(map_fs_binding)?;
        let Some((record, _)) = self
            .read_record::<StoredTelegramBinding>(&path)
            .await
            .map_err(map_fs_binding)?
        else {
            return Ok(None);
        };
        UserId::new(record.user_id).map(Some).map_err(|error| {
            TelegramBindingError::StoreUnavailable {
                reason: format!("stored telegram binding user id invalid: {error}"),
            }
        })
    }
}

#[async_trait]
impl<F> RebornUserIdentityLookup for FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        Ok(self
            .resolve_user_identity_with_binding_epoch(provider, provider_user_id)
            .await?
            .map(|(user_id, _)| user_id))
    }

    async fn resolve_user_identity_with_binding_epoch(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<
        Option<(
            UserId,
            Option<ironclaw_conversations::ExternalActorBindingEpoch>,
        )>,
        RebornUserIdentityLookupError,
    > {
        if provider != crate::telegram::telegram_actor_identity::TELEGRAM_IDENTITY_PROVIDER {
            return Ok(None);
        }
        let path = Self::binding_path(provider_user_id)
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        let Some((record, _)) = self
            .read_record::<StoredTelegramBinding>(&path)
            .await
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?
        else {
            return Ok(None);
        };
        let user_id = UserId::new(record.user_id)
            .map_err(|error| RebornUserIdentityLookupError::InvalidUserId(error.to_string()))?;
        let epoch = ironclaw_conversations::ExternalActorBindingEpoch::new(record.epoch)
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        Ok(Some((user_id, Some(epoch))))
    }

    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        self.user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
            .await
    }

    async fn user_has_provider_binding_with_provider_user_id_prefix(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        if provider != crate::telegram::telegram_actor_identity::TELEGRAM_IDENTITY_PROVIDER {
            return Ok(false);
        }
        let index_path = Self::binding_user_index_path(user_id)
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        let Some((index, _)) = self
            .read_record::<StoredTelegramBindingUserIndex>(&index_path)
            .await
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?
        else {
            return Ok(false);
        };
        Ok(index.provider_user_ids.iter().any(|candidate| {
            provider_user_id_prefix
                .map(|prefix| candidate.starts_with(prefix))
                .unwrap_or(true)
        }))
    }
}

#[async_trait]
impl<F> TelegramDmTargetStore for FilesystemTelegramHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn upsert_dm_target(
        &self,
        installation_id: &AdapterInstallationId,
        target: TelegramDmTarget,
    ) -> Result<(), TelegramPairingError> {
        let path =
            Self::dm_target_path(installation_id, &target.user_id).map_err(map_fs_pairing)?;
        self.write_record(&path, &target, CasExpectation::Any)
            .await
            .map_err(map_fs_pairing)?;
        Ok(())
    }

    async fn dm_target_for_user(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<Option<TelegramDmTarget>, TelegramPairingError> {
        let path = Self::dm_target_path(installation_id, user_id).map_err(map_fs_pairing)?;
        Ok(self
            .read_record::<TelegramDmTarget>(&path)
            .await
            .map_err(map_fs_pairing)?
            .map(|(target, _)| target))
    }

    async fn delete_dm_target_for_user(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<(), TelegramPairingError> {
        let path = Self::dm_target_path(installation_id, user_id).map_err(map_fs_pairing)?;
        match self.delete_record(&path).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_fs_pairing(error)),
        }
    }
}
