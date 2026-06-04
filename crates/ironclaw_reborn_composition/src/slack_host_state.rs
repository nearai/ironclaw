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
use ironclaw_product_adapters::{
    AdapterInstallationId, DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod,
    EgressPath, EgressRequest, ProtocolHttpEgress,
};
use ironclaw_slack_v2_adapter::SLACK_API_HOST;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::slack_actor_identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};
use crate::slack_personal_binding::{
    RebornUserIdentityBinding, RebornUserIdentityBindingError, RebornUserIdentityBindingStore,
};
use crate::slack_personal_binding_pairing::{
    IssuedSlackPersonalBindingPairingChallenge, SlackPersonalBindingPairingChallenge,
    SlackPersonalBindingPairingChallengeStore, SlackPersonalBindingPairingCode,
    SlackPersonalBindingPairingError, SlackPersonalBindingPairingNotification,
    SlackPersonalBindingPairingNotifier,
};
use crate::slack_serve::SlackUserId;

const SLACK_HOST_STATE_ROOT: &str = "/tenant-shared/slack-personal-binding";
const IDENTITY_ROOT: &str = "/tenant-shared/slack-personal-binding/identities";
const PAIRING_CODE_ROOT: &str = "/tenant-shared/slack-personal-binding/pairing/codes";
const PAIRING_CODE_LEN: usize = 8;
const PAIRING_CODE_RETRIES: usize = 16;
const DEFAULT_PAIRING_TTL: Duration = Duration::from_secs(10 * 60);
const SLACK_CONVERSATIONS_OPEN_PATH: &str = "/api/conversations.open";
const SLACK_POST_MESSAGE_PATH: &str = "/api/chat.postMessage";
const SLACK_API_RESPONSE_LIMIT: usize = 64 * 1024;

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
        Ok(challenge)
    }
}

pub(crate) struct SlackPairingChallengeHttpNotifier {
    egress: Arc<dyn ProtocolHttpEgress>,
    credential_handle: EgressCredentialHandle,
}

impl SlackPairingChallengeHttpNotifier {
    pub(crate) fn new(
        egress: Arc<dyn ProtocolHttpEgress>,
        credential_handle: EgressCredentialHandle,
    ) -> Self {
        Self {
            egress,
            credential_handle,
        }
    }
}

#[async_trait::async_trait]
impl SlackPersonalBindingPairingNotifier for SlackPairingChallengeHttpNotifier {
    async fn send_pairing_challenge(
        &self,
        notification: SlackPersonalBindingPairingNotification,
    ) -> Result<(), SlackPersonalBindingPairingError> {
        let channel = self
            .open_dm_channel(notification.slack_user_id.as_str())
            .await?;
        let body = serde_json::to_vec(&SlackPairingPostMessage {
            channel,
            text: format!(
                "Connect this Slack account to Ironclaw by entering code {} in WebChat.",
                notification.code.as_str()
            ),
            mrkdwn: false,
        })
        .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?;
        let response = self
            .send_slack_request(SLACK_POST_MESSAGE_PATH, body)
            .await?;
        slack_ok_response("Slack pairing DM", response.body())?;
        Ok(())
    }
}

impl SlackPairingChallengeHttpNotifier {
    async fn open_dm_channel(
        &self,
        slack_user_id: &str,
    ) -> Result<String, SlackPersonalBindingPairingError> {
        let body = serde_json::to_vec(&SlackConversationsOpenRequest {
            users: slack_user_id.to_string(),
        })
        .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?;
        let response = self
            .send_slack_request(SLACK_CONVERSATIONS_OPEN_PATH, body)
            .await?;
        let opened: SlackConversationsOpenResponse =
            slack_json_response("Slack conversations.open", response.body())?;
        if !opened.ok {
            return Err(SlackPersonalBindingPairingError::Backend(format!(
                "Slack rejected conversations.open ({})",
                opened.error.unwrap_or_else(|| "unknown_error".into())
            )));
        }
        opened
            .channel
            .map(|channel| channel.id)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                SlackPersonalBindingPairingError::Backend(
                    "Slack conversations.open response did not include a channel id".into(),
                )
            })
    }

    async fn send_slack_request(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Result<ironclaw_product_adapters::EgressResponse, SlackPersonalBindingPairingError> {
        let response = self
            .egress
            .send(slack_api_request(
                path,
                body,
                self.credential_handle.clone(),
            ))
            .await
            .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?;
        if !(200..300).contains(&response.status()) {
            return Err(SlackPersonalBindingPairingError::Backend(format!(
                "Slack API request {path} failed with HTTP {}",
                response.status()
            )));
        }
        Ok(response)
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
    fn binding(&self) -> RebornUserIdentityBinding {
        RebornUserIdentityBinding {
            provider: crate::slack_personal_binding::RebornIdentityProviderId::new(
                self.provider.clone(),
            )
            .expect("valid provider"),
            provider_user_id: crate::slack_personal_binding::RebornIdentityProviderUserId::new(
                self.provider_user_id.clone(),
            )
            .expect("valid provider user id"),
            user_id: UserId::new(self.user_id.clone()).expect("valid user id"),
        }
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

#[derive(Debug, Serialize)]
struct SlackConversationsOpenRequest {
    users: String,
}

#[derive(Debug, Deserialize)]
struct SlackConversationsOpenResponse {
    ok: bool,
    error: Option<String>,
    channel: Option<SlackConversationsOpenChannel>,
}

#[derive(Debug, Deserialize)]
struct SlackConversationsOpenChannel {
    id: String,
}

#[derive(Debug, Serialize)]
struct SlackPairingPostMessage {
    channel: String,
    text: String,
    mrkdwn: bool,
}

#[derive(Debug, Deserialize)]
struct SlackApiResponse {
    ok: bool,
    error: Option<String>,
}

fn slack_api_request(
    path: &'static str,
    body: Vec<u8>,
    credential_handle: EgressCredentialHandle,
) -> EgressRequest {
    let host = DeclaredEgressHost::new(SLACK_API_HOST).expect("static Slack host valid");
    let method = EgressMethod::post();
    let path = EgressPath::new(path).expect("static Slack API path valid");
    let content_type = EgressHeader::new("content-type", "application/json")
        .expect("static content-type header valid");
    EgressRequest::new(host, method, path)
        .with_header(content_type)
        .with_body(body)
        .with_credential_handle(Some(credential_handle))
}

fn slack_json_response<T>(
    label: &'static str,
    body: &[u8],
) -> Result<T, SlackPersonalBindingPairingError>
where
    T: DeserializeOwned,
{
    if body.len() > SLACK_API_RESPONSE_LIMIT {
        return Err(SlackPersonalBindingPairingError::Backend(format!(
            "{label} response exceeded body limit"
        )));
    }
    serde_json::from_slice(body).map_err(|error| {
        SlackPersonalBindingPairingError::Backend(format!(
            "{label} response was invalid JSON: {error}"
        ))
    })
}

fn slack_ok_response(
    label: &'static str,
    body: &[u8],
) -> Result<(), SlackPersonalBindingPairingError> {
    let response: SlackApiResponse = slack_json_response(label, body)?;
    if response.ok {
        Ok(())
    } else {
        Err(SlackPersonalBindingPairingError::Backend(format!(
            "Slack rejected {label} ({})",
            response.error.unwrap_or_else(|| "unknown_error".into())
        )))
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
        assert_eq!(stored.binding(), binding);
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

    #[tokio::test]
    async fn slack_pairing_notifier_posts_code_to_slack_user() {
        let egress = Arc::new(RecordingEgress::default());
        let notifier = SlackPairingChallengeHttpNotifier::new(
            egress.clone(),
            EgressCredentialHandle::new("slack_bot_token").unwrap(),
        );

        notifier
            .send_pairing_challenge(SlackPersonalBindingPairingNotification {
                installation_id: installation(),
                slack_user_id: SlackUserId::new("U123"),
                code: SlackPersonalBindingPairingCode::new("ABCD1234").unwrap(),
            })
            .await
            .expect("notification succeeds");

        let calls = egress.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].path().as_str(), SLACK_CONVERSATIONS_OPEN_PATH);
        let open_body: serde_json::Value = serde_json::from_slice(calls[0].body()).unwrap();
        assert_eq!(open_body["users"], "U123");
        assert_eq!(calls[1].path().as_str(), SLACK_POST_MESSAGE_PATH);
        let post_body: serde_json::Value = serde_json::from_slice(calls[1].body()).unwrap();
        assert_eq!(post_body["channel"], "D123");
        assert!(post_body["text"].as_str().unwrap().contains("ABCD1234"));
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

    #[derive(Default)]
    struct RecordingEgress {
        calls: Mutex<Vec<EgressRequest>>,
    }

    impl RecordingEgress {
        fn calls(&self) -> Vec<EgressRequest> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl ProtocolHttpEgress for RecordingEgress {
        async fn send(
            &self,
            request: EgressRequest,
        ) -> Result<
            ironclaw_product_adapters::EgressResponse,
            ironclaw_product_adapters::ProtocolHttpEgressError,
        > {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            let response = match self.calls().last().map(|request| request.path().as_str()) {
                Some(SLACK_CONVERSATIONS_OPEN_PATH) => {
                    br#"{"ok":true,"channel":{"id":"D123"}}"#.to_vec()
                }
                _ => br#"{"ok":true}"#.to_vec(),
            };
            Ok(ironclaw_product_adapters::EgressResponse::new(
                200, response,
            ))
        }
    }
}
