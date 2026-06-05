//! Durable host state for Slack host-beta personal binding.
//!
//! The Slack ingress path starts before a Slack actor is bound to a Reborn
//! user, so this state is tenant-scoped and lives under `/tenant-shared`.
//! The underlying `ScopedFilesystem` still routes through host APIs and is
//! backed by the selected durable root filesystem in libSQL/Postgres builds.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use ironclaw_product_adapters::AdapterInstallationId;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::slack_actor_identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};
use crate::slack_personal_binding::{
    RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
};
use crate::slack_personal_binding_pairing::{
    IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallenge,
    SlackPersonalBindingPairingChallengeStore, SlackPersonalBindingPairingCode,
    SlackPersonalBindingPairingError,
};
use crate::slack_serve::SlackUserId;

const SLACK_HOST_STATE_ROOT: &str = "/tenant-shared/slack-personal-binding";
const IDENTITY_ROOT: &str = "/tenant-shared/slack-personal-binding/identities";
const PAIRING_CODE_ROOT: &str = "/tenant-shared/slack-personal-binding/pairing/codes";
const PAIRING_ACTOR_ROOT: &str = "/tenant-shared/slack-personal-binding/pairing/actors";
const PAIRING_CODE_LEN: usize = 8;
const PAIRING_CODE_RETRIES: usize = 16;
const DEFAULT_PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
#[derive(Clone)]
pub(crate) struct FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    scope: ResourceScope,
    pairing_ttl: Duration,
    locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl<F> FilesystemSlackHostState<F>
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
            pairing_ttl: DEFAULT_PAIRING_TTL,
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    fn with_pairing_ttl(mut self, pairing_ttl: Duration) -> Self {
        self.pairing_ttl = pairing_ttl;
        self
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
                reason: "Slack host-state record is invalid JSON".into(),
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
                reason: "Slack host-state record could not be serialized".into(),
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

    fn identity_path(
        provider: &str,
        provider_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}.json",
            IDENTITY_ROOT,
            path_segment(provider),
            path_segment(provider_user_id)
        ))
    }

    fn pairing_code_path(
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!("{}/{}.json", PAIRING_CODE_ROOT, code.as_str()))
    }

    fn pairing_actor_path(
        challenge: &SlackPersonalBindingPairingChallenge,
    ) -> Result<ScopedPath, FilesystemError> {
        Self::pairing_actor_path_for(&challenge.installation_id, challenge.slack_user_id.as_str())
    }

    fn pairing_actor_path_for(
        installation_id: &AdapterInstallationId,
        slack_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{}/{}/{}.json",
            PAIRING_ACTOR_ROOT,
            path_segment(installation_id.as_str()),
            path_segment(slack_user_id)
        ))
    }
}

#[async_trait::async_trait]
impl<F> RebornUserIdentityLookup for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        let path = Self::identity_path(provider, provider_user_id).map_err(map_lookup_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackUserIdentity>(&path)
            .await
            .map_err(map_lookup_fs_error)?
        else {
            return Ok(None);
        };
        let user_id = UserId::new(record.user_id)
            .map_err(|error| RebornUserIdentityLookupError::InvalidUserId(error.to_string()))?;
        Ok(Some(user_id))
    }
}

#[async_trait::async_trait]
impl<F> RebornUserIdentityBindingStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let path =
            Self::identity_path(binding.provider.as_str(), binding.provider_user_id.as_str())
                .map_err(map_binding_fs_error)?;
        let lock = self.lock_for(format!(
            "identity:{}:{}",
            binding.provider.as_str(),
            binding.provider_user_id.as_str()
        ));
        let _guard = lock.lock().await;
        if let Some((existing, version)) = self
            .read_record::<StoredSlackUserIdentity>(&path)
            .await
            .map_err(map_binding_fs_error)?
        {
            if existing.user_id != binding.user_id.as_str() {
                return Err(RebornUserIdentityBindingError::Backend(
                    "Slack actor is already bound to a different user".into(),
                ));
            }
            let updated = StoredSlackUserIdentity::from_binding(&binding, existing.created_at);
            match self
                .write_record(&path, &updated, CasExpectation::Version(version))
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => {
                    self.reconcile_identity_version_mismatch(&path, &binding)
                        .await?;
                }
                Err(error) => return Err(map_binding_fs_error(error)),
            }
            return Ok(());
        }

        let record = StoredSlackUserIdentity::from_binding(&binding, Utc::now());
        match self
            .write_record(&path, &record, CasExpectation::Absent)
            .await
        {
            Ok(_) => {}
            Err(FilesystemError::VersionMismatch { .. }) => {
                self.reconcile_identity_version_mismatch(&path, &binding)
                    .await?;
            }
            Err(error) => return Err(map_binding_fs_error(error)),
        }
        Ok(())
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn reconcile_identity_version_mismatch(
        &self,
        path: &ScopedPath,
        binding: &RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let Some((existing, _)) = self
            .read_record::<StoredSlackUserIdentity>(path)
            .await
            .map_err(map_binding_fs_error)?
        else {
            return Err(RebornUserIdentityBindingError::Backend(
                "Slack actor binding changed concurrently".into(),
            ));
        };
        if existing.user_id == binding.user_id.as_str() {
            return Ok(());
        }
        Err(RebornUserIdentityBindingError::Backend(
            "Slack actor is already bound to a different user".into(),
        ))
    }
}

#[async_trait::async_trait]
impl<F> SlackPersonalBindingPairingChallengeStore for FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn issue_challenge(
        &self,
        challenge: SlackPersonalBindingPairingChallenge,
    ) -> Result<IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let actor_path = Self::pairing_actor_path(&challenge).map_err(map_pairing_fs_error)?;
        let actor_lock = self.lock_for(format!(
            "pairing-actor:{}:{}",
            challenge.installation_id.as_str(),
            challenge.slack_user_id.as_str()
        ));
        let _actor_guard = actor_lock.lock().await;
        let existing_actor = self
            .read_record::<StoredSlackPairingActorChallenge>(&actor_path)
            .await
            .map_err(map_pairing_fs_error)?;
        if let Some((actor_record, _)) = existing_actor.as_ref()
            && let Some(issued) = self
                .active_actor_pairing_challenge(actor_record, &challenge)
                .await?
        {
            return Ok(issued);
        }
        if let Some((actor_record, _)) = existing_actor.as_ref()
            && actor_record.expires_at <= Utc::now()
        {
            self.cleanup_actor_pairing_code_record(actor_record).await;
        }

        let expires_at = Utc::now()
            + chrono::Duration::from_std(self.pairing_ttl).map_err(|_| {
                SlackPersonalBindingPairingError::Backend(
                    "Slack pairing TTL could not be represented".into(),
                )
            })?;
        for _ in 0..PAIRING_CODE_RETRIES {
            let code = SlackPersonalBindingPairingCode::new(random_pairing_code())?;
            let path = Self::pairing_code_path(&code).map_err(map_pairing_fs_error)?;
            let record = StoredSlackPairingChallenge::pending(&code, &challenge, expires_at);
            match self
                .write_record(&path, &record, CasExpectation::Absent)
                .await
            {
                Ok(_) => {
                    let actor_record =
                        StoredSlackPairingActorChallenge::pending(&code, &challenge, expires_at);
                    let actor_cas = existing_actor
                        .as_ref()
                        .map(|(_, version)| CasExpectation::Version(*version))
                        .unwrap_or(CasExpectation::Absent);
                    match self
                        .write_record(&actor_path, &actor_record, actor_cas)
                        .await
                    {
                        Ok(_) => {}
                        Err(FilesystemError::VersionMismatch { .. }) => {
                            self.cleanup_pairing_code_record(&path).await;
                            let Some((winner, _)) = self
                                .read_record::<StoredSlackPairingActorChallenge>(&actor_path)
                                .await
                                .map_err(map_pairing_fs_error)?
                            else {
                                continue;
                            };
                            if let Some(issued) = self
                                .active_actor_pairing_challenge(&winner, &challenge)
                                .await?
                            {
                                return Ok(issued);
                            }
                            continue;
                        }
                        Err(error) => {
                            self.cleanup_pairing_code_record(&path).await;
                            return Err(map_pairing_fs_error(error));
                        }
                    }
                    return Ok(IssuedSlackPersonalBindingPairingChallenge { code, challenge });
                }
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(map_pairing_fs_error(error)),
            }
        }
        Err(SlackPersonalBindingPairingError::Backend(
            "could not allocate a unique Slack pairing code".into(),
        ))
    }

    async fn get_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let path = Self::pairing_code_path(code).map_err(map_pairing_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredSlackPairingChallenge>(&path)
            .await
            .map_err(map_pairing_fs_error)?
        else {
            return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
        };

        active_pairing_challenge(&record)
    }

    async fn consume_challenge(
        &self,
        code: &SlackPersonalBindingPairingCode,
    ) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
        let path = Self::pairing_code_path(code).map_err(map_pairing_fs_error)?;
        let lock = self.lock_for(format!("pairing:{}", code.as_str()));
        let _guard = lock.lock().await;
        let Some((mut record, version)) = self
            .read_record::<StoredSlackPairingChallenge>(&path)
            .await
            .map_err(map_pairing_fs_error)?
        else {
            return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
        };
        let challenge = active_pairing_challenge(&record)?;
        let actor_path = Self::pairing_actor_path_for(
            &challenge.installation_id,
            challenge.slack_user_id.as_str(),
        )
        .map_err(map_pairing_fs_error)?;
        let actor_lock = self.lock_for(format!(
            "pairing-actor:{}:{}",
            challenge.installation_id.as_str(),
            challenge.slack_user_id.as_str()
        ));
        let _actor_guard = actor_lock.lock().await;
        record.status = StoredSlackPairingStatus::Consumed;
        record.consumed_at = Some(Utc::now());
        match self
            .write_record(&path, &record, CasExpectation::Version(version))
            .await
        {
            Ok(_) => {}
            Err(FilesystemError::VersionMismatch { .. }) => {
                return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
            }
            Err(error) => return Err(map_pairing_fs_error(error)),
        }
        self.cleanup_pairing_code_record(&path).await;
        self.cleanup_pairing_actor_record(&actor_path, code).await;
        Ok(challenge)
    }
}

impl<F> FilesystemSlackHostState<F>
where
    F: RootFilesystem + 'static,
{
    async fn active_actor_pairing_challenge(
        &self,
        actor_record: &StoredSlackPairingActorChallenge,
        requested: &SlackPersonalBindingPairingChallenge,
    ) -> Result<Option<IssuedSlackPersonalBindingPairingChallenge>, SlackPersonalBindingPairingError>
    {
        if actor_record.installation_id != requested.installation_id.as_str()
            || actor_record.slack_user_id != requested.slack_user_id.as_str()
            || actor_record.expires_at <= Utc::now()
        {
            return Ok(None);
        }
        let code = SlackPersonalBindingPairingCode::new(actor_record.code.clone())?;
        let path = Self::pairing_code_path(&code).map_err(map_pairing_fs_error)?;
        let Some((code_record, _)) = self
            .read_record::<StoredSlackPairingChallenge>(&path)
            .await
            .map_err(map_pairing_fs_error)?
        else {
            return Ok(None);
        };
        let challenge = match active_pairing_challenge(&code_record) {
            Ok(challenge) => challenge,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        if challenge == *requested {
            return Ok(Some(IssuedSlackPersonalBindingPairingChallenge {
                code,
                challenge,
            }));
        }
        Ok(None)
    }

    async fn cleanup_pairing_code_record(&self, path: &ScopedPath) {
        if self.delete_record(path).await.is_err() {
            tracing::warn!("failed to delete Slack pairing code record");
        }
    }

    async fn cleanup_actor_pairing_code_record(
        &self,
        actor_record: &StoredSlackPairingActorChallenge,
    ) {
        let Ok(code) = SlackPersonalBindingPairingCode::new(actor_record.code.clone()) else {
            return;
        };
        let Ok(path) = Self::pairing_code_path(&code) else {
            return;
        };
        self.cleanup_pairing_code_record(&path).await;
    }

    async fn cleanup_pairing_actor_record(
        &self,
        actor_path: &ScopedPath,
        code: &SlackPersonalBindingPairingCode,
    ) {
        let Some((mut record, version)) = (match self
            .read_record::<StoredSlackPairingActorChallenge>(actor_path)
            .await
        {
            Ok(Some((record, version))) if record.code == code.as_str() => Some((record, version)),
            Ok(Some(_)) | Ok(None) => None,
            Err(_) => {
                tracing::warn!("failed to read Slack pairing actor record for cleanup");
                None
            }
        }) else {
            return;
        };
        let now = Utc::now();
        record.expires_at = now;
        record.updated_at = now;
        match self
            .write_record(actor_path, &record, CasExpectation::Version(version))
            .await
        {
            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(_) => {
                tracing::warn!("failed to expire Slack pairing actor record for cleanup");
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackUserIdentity {
    provider: String,
    provider_user_id: String,
    user_id: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackUserIdentity {
    fn from_binding(binding: &RebornUserIdentityBinding, created_at: DateTime<Utc>) -> Self {
        Self {
            provider: binding.provider.as_str().to_string(),
            provider_user_id: binding.provider_user_id.as_str().to_string(),
            user_id: binding.user_id.as_str().to_string(),
            created_at,
            updated_at: Utc::now(),
        }
    }

    #[cfg(test)]
    fn binding(&self) -> Option<RebornUserIdentityBinding> {
        Some(RebornUserIdentityBinding {
            provider: crate::slack_personal_binding::RebornIdentityProviderId::new(
                self.provider.clone(),
            )
            .ok()?,
            provider_user_id: crate::slack_personal_binding::RebornIdentityProviderUserId::new(
                self.provider_user_id.clone(),
            )
            .ok()?,
            user_id: UserId::new(self.user_id.clone()).ok()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSlackPairingStatus {
    Pending,
    Consumed,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackPairingChallenge {
    code: String,
    installation_id: String,
    slack_user_id: String,
    status: StoredSlackPairingStatus,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

impl StoredSlackPairingChallenge {
    fn pending(
        code: &SlackPersonalBindingPairingCode,
        challenge: &SlackPersonalBindingPairingChallenge,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            code: code.as_str().to_string(),
            installation_id: challenge.installation_id.as_str().to_string(),
            slack_user_id: challenge.slack_user_id.as_str().to_string(),
            status: StoredSlackPairingStatus::Pending,
            created_at: Utc::now(),
            expires_at,
            consumed_at: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSlackPairingActorChallenge {
    installation_id: String,
    slack_user_id: String,
    code: String,
    expires_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredSlackPairingActorChallenge {
    fn pending(
        code: &SlackPersonalBindingPairingCode,
        challenge: &SlackPersonalBindingPairingChallenge,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            installation_id: challenge.installation_id.as_str().to_string(),
            slack_user_id: challenge.slack_user_id.as_str().to_string(),
            code: code.as_str().to_string(),
            expires_at,
            updated_at: Utc::now(),
        }
    }
}

fn active_pairing_challenge(
    record: &StoredSlackPairingChallenge,
) -> Result<SlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingError> {
    if record.status != StoredSlackPairingStatus::Pending || record.expires_at <= Utc::now() {
        return Err(SlackPersonalBindingPairingError::ChallengeNotFound);
    }
    Ok(SlackPersonalBindingPairingChallenge {
        installation_id: AdapterInstallationId::new(record.installation_id.clone())
            .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?,
        slack_user_id: SlackUserId::new(record.slack_user_id.clone()),
    })
}

fn random_pairing_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0_u8; PAIRING_CODE_LEN];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|byte| ALPHABET[usize::from(*byte) % ALPHABET.len()] as char)
        .collect()
}

fn path_segment(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn scoped_path(raw: &str) -> Result<ScopedPath, FilesystemError> {
    ScopedPath::new(raw).map_err(|error| FilesystemError::BackendInfrastructure {
        operation: FilesystemOperation::WriteFile,
        reason: format!("invalid Slack host-state path under {SLACK_HOST_STATE_ROOT}: {error}"),
    })
}

fn map_lookup_fs_error(error: FilesystemError) -> RebornUserIdentityLookupError {
    RebornUserIdentityLookupError::Backend(error.to_string())
}

fn map_binding_fs_error(error: FilesystemError) -> RebornUserIdentityBindingError {
    RebornUserIdentityBindingError::Backend(error.to_string())
}

fn map_pairing_fs_error(error: FilesystemError) -> SlackPersonalBindingPairingError {
    SlackPersonalBindingPairingError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    use crate::slack_personal_binding::{RebornIdentityProviderId, RebornIdentityProviderUserId};

    #[tokio::test]
    async fn filesystem_slack_host_state_binds_and_resolves_identity() {
        let state = state();
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user("user:alice"),
        };

        state
            .bind_user_identity(binding.clone())
            .await
            .expect("bind succeeds");
        let resolved = state
            .resolve_user_identity("slack", "install-alpha:U123")
            .await
            .expect("resolve succeeds");

        assert_eq!(resolved, Some(user("user:alice")));
        let stored = read_identity(&state, "slack", "install-alpha:U123").await;
        assert_eq!(stored.binding(), Some(binding));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_rejects_rebinding_actor_to_different_user() {
        let state = state();
        state
            .bind_user_identity(binding("user:alice"))
            .await
            .expect("first bind succeeds");
        let error = state
            .bind_user_identity(binding("user:bob"))
            .await
            .expect_err("rebind should fail");

        assert!(matches!(error, RebornUserIdentityBindingError::Backend(_)));
        assert_eq!(
            state
                .resolve_user_identity("slack", "install-alpha:U123")
                .await
                .expect("resolve succeeds"),
            Some(user("user:alice"))
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_consumes_pairing_code_once() {
        let state = state();
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");

        let consumed = state
            .consume_challenge(&issued.code)
            .await
            .expect("consume succeeds");

        assert_eq!(consumed, challenge());
        assert!(matches!(
            state.consume_challenge(&issued.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_previews_pairing_code_without_consuming_it() {
        let state = state();
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");

        let preview = state
            .get_challenge(&issued.code)
            .await
            .expect("preview succeeds");
        let consumed = state
            .consume_challenge(&issued.code)
            .await
            .expect("consume succeeds");

        assert_eq!(preview, challenge());
        assert_eq!(consumed, challenge());
        assert!(matches!(
            state.get_challenge(&issued.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reuses_active_pairing_code_for_actor() {
        let state = state();

        let first = state
            .issue_challenge(challenge())
            .await
            .expect("first issue succeeds");
        let second = state
            .issue_challenge(challenge())
            .await
            .expect("second issue succeeds");

        assert_eq!(second.code, first.code);
        assert_eq!(second.challenge, challenge());
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_concurrent_consume_allows_exactly_one_success() {
        let state = Arc::new(state());
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");
        let first_state = Arc::clone(&state);
        let second_state = Arc::clone(&state);
        let first_code = issued.code.clone();
        let second_code = issued.code.clone();

        let (first, second) = tokio::join!(
            first_state.consume_challenge(&first_code),
            second_state.consume_challenge(&second_code)
        );
        let successes = [&first, &second]
            .into_iter()
            .filter(|result| result.is_ok())
            .count();
        let not_found = [&first, &second]
            .into_iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(SlackPersonalBindingPairingError::ChallengeNotFound)
                )
            })
            .count();

        assert_eq!(successes, 1);
        assert_eq!(not_found, 1);
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_reissues_after_consumed_actor_challenge() {
        let state = state();
        let consumed = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");

        state
            .consume_challenge(&consumed.code)
            .await
            .expect("consume succeeds");
        let reissued = state
            .issue_challenge(challenge())
            .await
            .expect("reissue succeeds");

        assert_ne!(reissued.code, consumed.code);
        assert_eq!(reissued.challenge, challenge());
        assert!(matches!(
            state.get_challenge(&consumed.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
        assert_eq!(
            state
                .get_challenge(&reissued.code)
                .await
                .expect("reissued code remains active"),
            challenge()
        );
    }

    #[tokio::test]
    async fn filesystem_slack_host_state_rejects_expired_pairing_code() {
        let state = state().with_pairing_ttl(Duration::from_millis(1));
        let issued = state
            .issue_challenge(challenge())
            .await
            .expect("issue succeeds");
        tokio::time::sleep(Duration::from_millis(5)).await;

        assert!(matches!(
            state.consume_challenge(&issued.code).await,
            Err(SlackPersonalBindingPairingError::ChallengeNotFound)
        ));
    }

    fn state() -> FilesystemSlackHostState<InMemoryBackend> {
        let root = Arc::new(InMemoryBackend::default());
        let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
            root,
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/tenant-shared").unwrap(),
                VirtualPath::new("/tenants/tenant-alpha/shared").unwrap(),
                MountPermissions::read_write_list_delete(),
            )])
            .unwrap(),
        ));
        FilesystemSlackHostState::new(
            scoped,
            TenantId::new("tenant-alpha").unwrap(),
            user("user:host"),
            AgentId::new("agent:host").unwrap(),
            Some(ProjectId::new("project:host").unwrap()),
        )
    }

    fn binding(user_id: &str) -> RebornUserIdentityBinding {
        RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("slack").unwrap(),
            provider_user_id: RebornIdentityProviderUserId::new("install-alpha:U123").unwrap(),
            user_id: user(user_id),
        }
    }

    async fn read_identity(
        state: &FilesystemSlackHostState<InMemoryBackend>,
        provider: &str,
        provider_user_id: &str,
    ) -> StoredSlackUserIdentity {
        let path =
            FilesystemSlackHostState::<InMemoryBackend>::identity_path(provider, provider_user_id)
                .unwrap();
        state
            .read_record(&path)
            .await
            .unwrap()
            .expect("identity exists")
            .0
    }

    fn challenge() -> SlackPersonalBindingPairingChallenge {
        SlackPersonalBindingPairingChallenge {
            installation_id: installation(),
            slack_user_id: SlackUserId::new("U123"),
        }
    }

    fn installation() -> AdapterInstallationId {
        AdapterInstallationId::new("install-alpha").unwrap()
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }
}
