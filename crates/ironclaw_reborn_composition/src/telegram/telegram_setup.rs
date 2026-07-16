//! Durable Telegram installation setup and secret boundary.
//!
//! One bot per deployment, operator-managed at runtime. This module owns the
//! only place WebUI-submitted Telegram secrets are written to the shared
//! `SecretStore` and the only place runtime code resolves those handles back
//! to material. The save pipeline is fail-closed: token validation (`getMe`)
//! and webhook registration (`setWebhook`) both succeed before anything is
//! persisted, and a failed post-save activation restores the previous record
//! (mirroring the Slack setup rollback contract).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_common::hashing::sha256_hex;
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStoreError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::telegram::telegram_bot_api::{TelegramBotApi, TelegramBotApiError, TelegramBotIdentity};

const TELEGRAM_BOT_TOKEN_HANDLE_PREFIX: &str = "telegram_bot_token";
const TELEGRAM_WEBHOOK_SECRET_HANDLE_PREFIX: &str = "telegram_webhook_secret";
const INSTALLATION_HANDLE_HASH_LEN: usize = 24;

/// The route every deployment registers with Telegram (`setWebhook`). Pinned
/// to the unified-extension-runtime path so registrations survive the port.
pub(crate) const TELEGRAM_UPDATES_ROUTE_PATH: &str = "/webhooks/extensions/telegram/updates";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TelegramInstallationSetup {
    pub(crate) bot_id: i64,
    pub(crate) bot_username: String,
    pub(crate) webhook_url: String,
    pub(crate) bot_token_handle: SecretHandle,
    pub(crate) webhook_secret_handle: SecretHandle,
    pub(crate) revision: u64,
    pub(crate) updated_at: DateTime<Utc>,
}

impl TelegramInstallationSetup {
    /// Installation identity is the bot: rotating the same bot's token keeps
    /// pairings; pointing at a different bot re-scopes them by design.
    pub(crate) fn installation_id(&self) -> Result<AdapterInstallationId, TelegramSetupError> {
        AdapterInstallationId::new(format!("tg-bot-{}", self.bot_id)).map_err(|error| {
            TelegramSetupError::InvalidField {
                field: "bot_id",
                reason: error.to_string(),
            }
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TelegramInstallationSetupUpdate {
    /// New bot token; `None`/blank means "keep the existing token".
    pub(crate) bot_token: Option<SecretString>,
    /// Explicit public webhook URL override; `None` derives it from the
    /// deployment public base URL.
    pub(crate) webhook_url_override: Option<String>,
}

/// Redacted, serialize-only status projection for the admin UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TelegramInstallationSetupStatus {
    pub(crate) configured: bool,
    pub(crate) bot_username: Option<String>,
    pub(crate) bot_token_configured: bool,
    pub(crate) webhook_url: Option<String>,
    pub(crate) revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum TelegramSetupError {
    #[error("invalid telegram setup field {field}: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("missing telegram setup field {field}")]
    MissingField { field: &'static str },
    #[error("telegram setup store unavailable")]
    StoreUnavailable,
    #[error("telegram secret store unavailable: {reason}")]
    SecretStoreUnavailable { reason: &'static str },
    #[error(
        "no public base URL is configured; set a webhook URL override or configure the deployment public origin"
    )]
    PublicUrlMissing,
    #[error("telegram bot api call failed: {reason}")]
    BotApi { reason: String },
}

impl From<TelegramBotApiError> for TelegramSetupError {
    fn from(error: TelegramBotApiError) -> Self {
        TelegramSetupError::BotApi {
            reason: error.to_string(),
        }
    }
}

#[async_trait]
pub(crate) trait TelegramInstallationSetupStore: Send + Sync + std::fmt::Debug {
    async fn get_telegram_installation_setup(
        &self,
    ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError>;

    async fn put_telegram_installation_setup(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<(), TelegramSetupError>;

    async fn delete_telegram_installation_setup(&self) -> Result<(), TelegramSetupError>;
}

#[derive(Clone)]
pub(crate) struct TelegramSetupService {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    operator_user_id: UserId,
    store: Arc<dyn TelegramInstallationSetupStore>,
    secret_store: Arc<dyn SecretStore>,
    bot_api: Arc<dyn TelegramBotApi>,
    public_base_url: Option<String>,
    save_lock: Arc<Mutex<()>>,
}

impl TelegramSetupService {
    #[allow(clippy::too_many_arguments)]
    // arch-exempt: too_many_args, mirrors SlackSetupService::new plus the two
    // telegram-only inputs (bot api port + public base URL); folds into the
    // telegram host runtime config bundle at the #6116 port, plan follows the
    // slack cleanup.
    pub(crate) fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        operator_user_id: UserId,
        store: Arc<dyn TelegramInstallationSetupStore>,
        secret_store: Arc<dyn SecretStore>,
        bot_api: Arc<dyn TelegramBotApi>,
        public_base_url: Option<String>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            operator_user_id,
            store,
            secret_store,
            bot_api,
            public_base_url,
            save_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    // Scope-parity accessors mirroring `SlackSetupService` (used there by the
    // dynamic provisioner Debug impls); Telegram's host wiring passes the host
    // config scope directly, so these stay for the #6116 fold's shared shape.
    #[allow(dead_code)]
    pub(crate) fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    #[allow(dead_code)]
    pub(crate) fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }

    pub(crate) fn operator_user_id(&self) -> &UserId {
        &self.operator_user_id
    }

    pub(crate) fn bot_api(&self) -> Arc<dyn TelegramBotApi> {
        Arc::clone(&self.bot_api)
    }

    pub(crate) async fn current_setup(
        &self,
    ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
        self.store.get_telegram_installation_setup().await
    }

    pub(crate) async fn status(
        &self,
    ) -> Result<TelegramInstallationSetupStatus, TelegramSetupError> {
        let Some(setup) = self.current_setup().await? else {
            return Ok(TelegramInstallationSetupStatus {
                configured: false,
                bot_username: None,
                bot_token_configured: false,
                webhook_url: None,
                revision: None,
            });
        };
        let scope = self.secret_scope();
        let bot_token_configured = self
            .secret_store
            .metadata(&scope, &setup.bot_token_handle)
            .await
            .map_err(map_secret_error)?
            .is_some();
        let webhook_secret_configured = self
            .secret_store
            .metadata(&scope, &setup.webhook_secret_handle)
            .await
            .map_err(map_secret_error)?
            .is_some();
        Ok(TelegramInstallationSetupStatus {
            configured: bot_token_configured && webhook_secret_configured,
            bot_username: Some(setup.bot_username),
            bot_token_configured,
            webhook_url: Some(setup.webhook_url),
            revision: Some(setup.revision),
        })
    }

    /// Full save pipeline (fail-closed, nothing persisted on failure):
    /// resolve the effective token → `getMe` → derive the webhook URL →
    /// mint a fresh webhook secret → `setWebhook` → persist secrets under
    /// revision-suffixed handles → persist the record.
    pub(crate) async fn save_with_previous(
        &self,
        update: TelegramInstallationSetupUpdate,
    ) -> Result<(Option<TelegramInstallationSetup>, TelegramInstallationSetup), TelegramSetupError>
    {
        let _save_guard = self.save_lock.lock().await;
        let previous = self.current_setup().await?;
        let revision = previous
            .as_ref()
            .map(|setup| setup.revision.saturating_add(1))
            .unwrap_or(1);

        let bot_token = match normalize_secret(update.bot_token) {
            Some(token) => token,
            None => match previous.as_ref() {
                Some(previous_setup) => {
                    self.secret_material(&previous_setup.bot_token_handle)
                        .await?
                }
                None => return Err(TelegramSetupError::MissingField { field: "bot_token" }),
            },
        };

        let identity = self.bot_api.get_me(&bot_token).await?;
        let webhook_url = self.effective_webhook_url(update.webhook_url_override)?;
        let webhook_secret = mint_webhook_secret();
        self.bot_api
            .set_webhook(&bot_token, &webhook_url, &webhook_secret)
            .await?;

        let record = self.build_record(&identity, webhook_url, revision)?;
        self.put_secret(record.bot_token_handle.clone(), bot_token)
            .await?;
        self.put_secret(record.webhook_secret_handle.clone(), webhook_secret)
            .await?;
        self.store.put_telegram_installation_setup(&record).await?;
        Ok((previous, record))
    }

    /// Restore the previous record when post-save extension activation fails,
    /// so store state and runtime never split-brain (Slack rollback contract).
    pub(crate) async fn rollback_failed_activation_save(
        &self,
        saved: &TelegramInstallationSetup,
        previous: Option<&TelegramInstallationSetup>,
    ) -> Result<(), TelegramSetupError> {
        let _save_guard = self.save_lock.lock().await;
        let current = self.current_setup().await?;
        if current.as_ref() != Some(saved) {
            return Ok(());
        }
        match previous {
            Some(previous_setup) => {
                self.store
                    .put_telegram_installation_setup(previous_setup)
                    .await
            }
            None => self.store.delete_telegram_installation_setup().await,
        }
    }

    /// Clear the setup: best-effort `deleteWebhook`, then remove the durable
    /// record. Pairing records and history are deliberately retained — an
    /// unconfigured deployment simply fails closed at ingress.
    pub(crate) async fn clear(&self) -> Result<(), TelegramSetupError> {
        let _save_guard = self.save_lock.lock().await;
        if let Some(setup) = self.current_setup().await? {
            match self.secret_material(&setup.bot_token_handle).await {
                Ok(material) => {
                    let token = material;
                    if let Err(error) = self.bot_api.delete_webhook(&token).await {
                        tracing::debug!(
                            reason = %error,
                            "telegram deleteWebhook failed during clear; proceeding"
                        );
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        reason = %error,
                        "telegram bot token unavailable during clear; skipping deleteWebhook"
                    );
                }
            }
        }
        self.store.delete_telegram_installation_setup().await
    }

    /// Resolve the current bot token material (ingress/egress wiring).
    pub(crate) async fn bot_token(&self) -> Result<Option<SecretString>, TelegramSetupError> {
        let Some(setup) = self.current_setup().await? else {
            return Ok(None);
        };
        Ok(Some(self.secret_material(&setup.bot_token_handle).await?))
    }

    /// Resolve the current webhook shared secret (ingress verification).
    pub(crate) async fn webhook_secret(&self) -> Result<Option<SecretString>, TelegramSetupError> {
        let Some(setup) = self.current_setup().await? else {
            return Ok(None);
        };
        Ok(Some(
            self.secret_material(&setup.webhook_secret_handle).await?,
        ))
    }

    fn effective_webhook_url(
        &self,
        webhook_url_override: Option<String>,
    ) -> Result<String, TelegramSetupError> {
        if let Some(explicit) = normalize_string(webhook_url_override) {
            if !explicit.starts_with("https://") {
                return Err(TelegramSetupError::InvalidField {
                    field: "webhook_url",
                    reason: "webhook URL must be https".to_string(),
                });
            }
            return Ok(explicit);
        }
        let base = self
            .public_base_url
            .as_deref()
            .map(str::trim)
            .filter(|base| !base.is_empty())
            .ok_or(TelegramSetupError::PublicUrlMissing)?;
        if !base.starts_with("https://") {
            return Err(TelegramSetupError::PublicUrlMissing);
        }
        Ok(format!(
            "{}{TELEGRAM_UPDATES_ROUTE_PATH}",
            base.trim_end_matches('/')
        ))
    }

    fn build_record(
        &self,
        identity: &TelegramBotIdentity,
        webhook_url: String,
        revision: u64,
    ) -> Result<TelegramInstallationSetup, TelegramSetupError> {
        let installation_key = format!("tg-bot-{}", identity.id);
        Ok(TelegramInstallationSetup {
            bot_id: identity.id,
            bot_username: identity.username.clone(),
            webhook_url,
            bot_token_handle: bot_token_handle(&self.tenant_id, &installation_key, revision)?,
            webhook_secret_handle: webhook_secret_handle(
                &self.tenant_id,
                &installation_key,
                revision,
            )?,
            revision,
            updated_at: Utc::now(),
        })
    }

    async fn put_secret(
        &self,
        handle: SecretHandle,
        value: SecretString,
    ) -> Result<(), TelegramSetupError> {
        self.secret_store
            .put(
                self.secret_scope(),
                handle,
                SecretMaterial::from(value.expose_secret().to_string()),
                None,
            )
            .await
            .map_err(map_secret_error)?;
        Ok(())
    }

    async fn secret_material(
        &self,
        handle: &SecretHandle,
    ) -> Result<SecretMaterial, TelegramSetupError> {
        let scope = self.secret_scope();
        let lease = self
            .secret_store
            .lease_once(&scope, handle)
            .await
            .map_err(map_secret_error)?;
        self.secret_store
            .consume(&scope, lease.id)
            .await
            .map_err(map_secret_error)
    }

    fn secret_scope(&self) -> ResourceScope {
        ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.operator_user_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            project_id: self.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}

fn mint_webhook_secret() -> SecretString {
    let random: [u8; 32] = rand::random();
    SecretString::from(sha256_hex(&random))
}

fn normalize_secret(value: Option<SecretString>) -> Option<SecretString> {
    let secret = value?;
    let trimmed = secret.expose_secret().trim();
    (!trimmed.is_empty()).then(|| SecretString::from(trimmed.to_string()))
}

fn normalize_string(value: Option<String>) -> Option<String> {
    let s = value?;
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn bot_token_handle(
    tenant_id: &TenantId,
    installation_key: &str,
    revision: u64,
) -> Result<SecretHandle, TelegramSetupError> {
    secret_handle_for_installation(
        TELEGRAM_BOT_TOKEN_HANDLE_PREFIX,
        tenant_id,
        installation_key,
        revision,
    )
    .map_err(|reason| TelegramSetupError::InvalidField {
        field: "bot_token",
        reason: reason.to_string(),
    })
}

fn webhook_secret_handle(
    tenant_id: &TenantId,
    installation_key: &str,
    revision: u64,
) -> Result<SecretHandle, TelegramSetupError> {
    secret_handle_for_installation(
        TELEGRAM_WEBHOOK_SECRET_HANDLE_PREFIX,
        tenant_id,
        installation_key,
        revision,
    )
    .map_err(|reason| TelegramSetupError::InvalidField {
        field: "webhook_secret",
        reason: reason.to_string(),
    })
}

fn secret_handle_for_installation(
    prefix: &str,
    tenant_id: &TenantId,
    installation_key: &str,
    revision: u64,
) -> Result<SecretHandle, ironclaw_host_api::HostApiError> {
    let digest = sha256_hex(&secret_handle_key_material(tenant_id, installation_key));
    SecretHandle::new(format!(
        "{prefix}_{}_v{revision}",
        &digest[..INSTALLATION_HANDLE_HASH_LEN]
    ))
}

fn secret_handle_key_material(tenant_id: &TenantId, installation_key: &str) -> Vec<u8> {
    let mut key = b"telegram-installation-secret:v1".to_vec();
    append_length_prefixed(&mut key, tenant_id.as_str().as_bytes());
    append_length_prefixed(&mut key, installation_key.as_bytes());
    key
}

fn append_length_prefixed(key: &mut Vec<u8>, value: &[u8]) {
    key.extend_from_slice(&(value.len() as u64).to_be_bytes());
    key.extend_from_slice(value);
}

fn map_secret_error(error: SecretStoreError) -> TelegramSetupError {
    TelegramSetupError::SecretStoreUnavailable {
        reason: error.stable_reason(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use ironclaw_secrets::InMemorySecretStore;

    use super::*;

    #[derive(Debug, Default)]
    struct InMemorySetupStore {
        record: StdMutex<Option<TelegramInstallationSetup>>,
    }

    #[async_trait]
    impl TelegramInstallationSetupStore for InMemorySetupStore {
        async fn get_telegram_installation_setup(
            &self,
        ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
            Ok(self.record.lock().expect("lock").clone())
        }

        async fn put_telegram_installation_setup(
            &self,
            setup: &TelegramInstallationSetup,
        ) -> Result<(), TelegramSetupError> {
            *self.record.lock().expect("lock") = Some(setup.clone());
            Ok(())
        }

        async fn delete_telegram_installation_setup(&self) -> Result<(), TelegramSetupError> {
            *self.record.lock().expect("lock") = None;
            Ok(())
        }
    }

    #[derive(Debug, Clone)]
    enum BotApiCall {
        GetMe,
        SetWebhook { url: String },
        DeleteWebhook,
        SendMessage { _chat_id: i64 },
    }

    #[derive(Debug)]
    struct RecordingBotApi {
        calls: StdMutex<Vec<BotApiCall>>,
        get_me: Result<TelegramBotIdentity, TelegramBotApiError>,
        set_webhook: Result<(), TelegramBotApiError>,
    }

    impl RecordingBotApi {
        fn ok() -> Self {
            Self {
                calls: StdMutex::new(Vec::new()),
                get_me: Ok(TelegramBotIdentity {
                    id: 4242,
                    username: "ironclaw_qa_bot".to_string(),
                }),
                set_webhook: Ok(()),
            }
        }

        fn calls(&self) -> Vec<BotApiCall> {
            self.calls.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl TelegramBotApi for RecordingBotApi {
        async fn get_me(
            &self,
            _bot_token: &SecretString,
        ) -> Result<TelegramBotIdentity, TelegramBotApiError> {
            self.calls.lock().expect("lock").push(BotApiCall::GetMe);
            self.get_me.clone()
        }

        async fn set_webhook(
            &self,
            _bot_token: &SecretString,
            url: &str,
            _secret_token: &SecretString,
        ) -> Result<(), TelegramBotApiError> {
            self.calls
                .lock()
                .expect("lock")
                .push(BotApiCall::SetWebhook {
                    url: url.to_string(),
                });
            self.set_webhook.clone()
        }

        async fn delete_webhook(
            &self,
            _bot_token: &SecretString,
        ) -> Result<(), TelegramBotApiError> {
            self.calls
                .lock()
                .expect("lock")
                .push(BotApiCall::DeleteWebhook);
            Ok(())
        }

        async fn send_message(
            &self,
            _bot_token: &SecretString,
            chat_id: i64,
            _text: &str,
        ) -> Result<(), TelegramBotApiError> {
            self.calls
                .lock()
                .expect("lock")
                .push(BotApiCall::SendMessage { _chat_id: chat_id });
            Ok(())
        }
    }

    fn service_with(
        store: Arc<InMemorySetupStore>,
        bot_api: Arc<RecordingBotApi>,
        public_base_url: Option<&str>,
    ) -> TelegramSetupService {
        TelegramSetupService::new(
            TenantId::new("tenant-a").expect("tenant"),
            AgentId::new("agent-a").expect("agent"),
            None,
            UserId::new("operator").expect("user"),
            store,
            Arc::new(InMemorySecretStore::new()),
            bot_api,
            public_base_url.map(str::to_string),
        )
    }

    fn update_with_token(token: &str) -> TelegramInstallationSetupUpdate {
        TelegramInstallationSetupUpdate {
            bot_token: Some(SecretString::from(token.to_string())),
            webhook_url_override: None,
        }
    }

    #[tokio::test]
    async fn save_happy_path_validates_registers_and_persists() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi::ok());
        let service = service_with(
            Arc::clone(&store),
            Arc::clone(&bot_api),
            Some("https://ironclaw.example"),
        );

        let (previous, saved) = service
            .save_with_previous(update_with_token("123:abc"))
            .await
            .expect("save succeeds");
        assert!(previous.is_none());
        assert_eq!(saved.bot_id, 4242);
        assert_eq!(saved.bot_username, "ironclaw_qa_bot");
        assert_eq!(
            saved.webhook_url,
            "https://ironclaw.example/webhooks/extensions/telegram/updates"
        );
        assert_eq!(saved.revision, 1);
        let calls = bot_api.calls();
        assert!(matches!(calls[0], BotApiCall::GetMe));
        match &calls[1] {
            BotApiCall::SetWebhook { url } => assert_eq!(
                url, "https://ironclaw.example/webhooks/extensions/telegram/updates",
                "setWebhook must register the derived public updates URL"
            ),
            other => panic!("expected SetWebhook as the second bot api call, got {other:?}"),
        }
        let token = service.bot_token().await.expect("token resolves");
        assert_eq!(
            token.expect("token present").expose_secret(),
            "123:abc",
            "bot token round-trips through the secret store"
        );
        assert!(
            service
                .webhook_secret()
                .await
                .expect("secret resolves")
                .is_some()
        );
        let status = service.status().await.expect("status");
        assert!(status.configured && status.bot_token_configured);
        assert_eq!(status.bot_username.as_deref(), Some("ironclaw_qa_bot"));
    }

    #[tokio::test]
    async fn invalid_token_persists_nothing() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi {
            calls: StdMutex::new(Vec::new()),
            get_me: Err(TelegramBotApiError::Rejected {
                description: "Unauthorized".to_string(),
            }),
            set_webhook: Ok(()),
        });
        let service = service_with(
            Arc::clone(&store),
            bot_api,
            Some("https://ironclaw.example"),
        );
        let error = service
            .save_with_previous(update_with_token("bad"))
            .await
            .expect_err("save fails closed");
        assert!(matches!(error, TelegramSetupError::BotApi { .. }));
        assert!(service.current_setup().await.expect("read").is_none());
    }

    #[tokio::test]
    async fn set_webhook_failure_persists_nothing() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi {
            calls: StdMutex::new(Vec::new()),
            get_me: Ok(TelegramBotIdentity {
                id: 1,
                username: "b".to_string(),
            }),
            set_webhook: Err(TelegramBotApiError::Rejected {
                description: "bad webhook".to_string(),
            }),
        });
        let service = service_with(
            Arc::clone(&store),
            bot_api,
            Some("https://ironclaw.example"),
        );
        let error = service
            .save_with_previous(update_with_token("123:abc"))
            .await
            .expect_err("save fails closed");
        assert!(matches!(error, TelegramSetupError::BotApi { .. }));
        assert!(service.current_setup().await.expect("read").is_none());
    }

    #[tokio::test]
    async fn missing_public_base_url_fails_before_any_bot_api_call_after_validation() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi::ok());
        let service = service_with(Arc::clone(&store), Arc::clone(&bot_api), None);
        let error = service
            .save_with_previous(update_with_token("123:abc"))
            .await
            .expect_err("save fails closed");
        assert!(matches!(error, TelegramSetupError::PublicUrlMissing));
        assert!(
            !bot_api
                .calls()
                .iter()
                .any(|call| matches!(call, BotApiCall::SetWebhook { .. })),
            "webhook must not be registered without a public URL"
        );
        assert!(service.current_setup().await.expect("read").is_none());
    }

    #[tokio::test]
    async fn rotation_bumps_revision_and_keeps_installation_identity() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi::ok());
        let service = service_with(
            Arc::clone(&store),
            bot_api,
            Some("https://ironclaw.example"),
        );
        let (_, first) = service
            .save_with_previous(update_with_token("123:abc"))
            .await
            .expect("first save");
        let (previous, second) = service
            .save_with_previous(update_with_token("123:rotated"))
            .await
            .expect("second save");
        assert_eq!(previous.as_ref(), Some(&first));
        assert_eq!(second.revision, 2);
        assert_ne!(second.webhook_secret_handle, first.webhook_secret_handle);
        assert_eq!(
            second.installation_id().expect("id"),
            first.installation_id().expect("id"),
            "same bot keeps the installation identity"
        );
    }

    #[tokio::test]
    async fn blank_token_keeps_existing_material() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi::ok());
        let service = service_with(
            Arc::clone(&store),
            bot_api,
            Some("https://ironclaw.example"),
        );
        service
            .save_with_previous(update_with_token("123:abc"))
            .await
            .expect("first save");
        service
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("   ".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("blank token save reuses existing");
        let token = service.bot_token().await.expect("token").expect("present");
        assert_eq!(token.expose_secret(), "123:abc");
    }

    #[tokio::test]
    async fn clear_deletes_webhook_and_record() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi::ok());
        let service = service_with(
            Arc::clone(&store),
            Arc::clone(&bot_api),
            Some("https://ironclaw.example"),
        );
        service
            .save_with_previous(update_with_token("123:abc"))
            .await
            .expect("save");
        service.clear().await.expect("clear succeeds");
        assert!(service.current_setup().await.expect("read").is_none());
        assert!(
            bot_api
                .calls()
                .iter()
                .any(|call| matches!(call, BotApiCall::DeleteWebhook)),
            "clear must attempt deleteWebhook"
        );
    }

    #[tokio::test]
    async fn rollback_restores_previous_record() {
        let store = Arc::new(InMemorySetupStore::default());
        let bot_api = Arc::new(RecordingBotApi::ok());
        let service = service_with(
            Arc::clone(&store),
            bot_api,
            Some("https://ironclaw.example"),
        );
        let (_, first) = service
            .save_with_previous(update_with_token("123:abc"))
            .await
            .expect("first save");
        let (previous, second) = service
            .save_with_previous(update_with_token("123:rotated"))
            .await
            .expect("second save");
        service
            .rollback_failed_activation_save(&second, previous.as_ref())
            .await
            .expect("rollback");
        assert_eq!(service.current_setup().await.expect("read"), Some(first));
    }
}
