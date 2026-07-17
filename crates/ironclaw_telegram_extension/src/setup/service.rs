use std::sync::Arc;

use chrono::Utc;
use ironclaw_common::hashing::sha256_hex;
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStoreError};
use secrecy::{ExposeSecret, SecretString};

use crate::bot_api::{HostEgressTelegramBotApi, TelegramBotIdentity};
use crate::state::FilesystemTelegramHostState;

use super::{
    TELEGRAM_UPDATES_ROUTE_PATH, TelegramInstallationSetup, TelegramInstallationSetupStatus,
    TelegramInstallationSetupUpdate, TelegramSetupError,
};

const TELEGRAM_BOT_TOKEN_HANDLE_PREFIX: &str = "telegram_bot_token";
const TELEGRAM_WEBHOOK_SECRET_HANDLE_PREFIX: &str = "telegram_webhook_secret";
const INSTALLATION_HANDLE_HASH_LEN: usize = 24;

#[derive(Clone)]
pub struct TelegramSetupService {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    operator_user_id: UserId,
    pub(super) state: Arc<FilesystemTelegramHostState>,
    pub(super) secret_store: Arc<dyn SecretStore>,
    pub(super) bot_api: Arc<HostEgressTelegramBotApi>,
    public_base_url: Option<String>,
}

impl TelegramSetupService {
    // arch-exempt: too_many_args, mirrors SlackSetupService::new (+ bot api port and public base URL) until the host runtime config bundle aggregates these, plan #6116
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        operator_user_id: UserId,
        state: Arc<FilesystemTelegramHostState>,
        secret_store: Arc<dyn SecretStore>,
        bot_api: Arc<HostEgressTelegramBotApi>,
        public_base_url: Option<String>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            operator_user_id,
            state,
            secret_store,
            bot_api,
            public_base_url,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn operator_user_id(&self) -> &UserId {
        &self.operator_user_id
    }

    pub(crate) fn bot_api(&self) -> Arc<HostEgressTelegramBotApi> {
        Arc::clone(&self.bot_api)
    }

    #[cfg(test)]
    pub(crate) fn state_for_test(&self) -> Arc<FilesystemTelegramHostState> {
        Arc::clone(&self.state)
    }

    pub async fn current_setup(
        &self,
    ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
        self.state.get_telegram_installation_setup().await
    }

    pub async fn status(&self) -> Result<TelegramInstallationSetupStatus, TelegramSetupError> {
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
    pub async fn save_with_previous(
        &self,
        update: TelegramInstallationSetupUpdate,
    ) -> Result<(Option<TelegramInstallationSetup>, TelegramInstallationSetup), TelegramSetupError>
    {
        self.recover_pending_setup_rollback().await?;
        let previous = self.current_setup().await?;
        let revision = self
            .state
            .next_telegram_installation_setup_revision()
            .await?;

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
        // Two processes can legitimately calculate the same next revision
        // before either wins the setup-record CAS. Keep their secret handles
        // disjoint so the loser can clean up only its own material.
        let save_attempt = mint_save_attempt_id();
        let record = self.build_record(&identity, webhook_url, revision, &save_attempt)?;
        self.bot_api
            .set_webhook(&bot_token, &record.webhook_url, &webhook_secret)
            .await?;

        // From here Telegram already points at the new registration; a local
        // persistence failure must compensate the provider side (restore the
        // previous registration, or delete the fresh one) so the durable
        // record and the remote webhook cannot diverge.
        match self
            .persist_saved_record(record, previous.as_ref(), &bot_token, &webhook_secret)
            .await
        {
            Ok(record) => Ok((previous, record)),
            Err(error) => {
                self.reconcile_remote_webhook_to_current(&bot_token, identity.id)
                    .await;
                Err(error)
            }
        }
    }

    /// The local persistence tail of a save: secrets under revision-suffixed
    /// handles, then the durable record. Each failure cleans up the secrets
    /// already written for this revision (best-effort) before surfacing.
    async fn persist_saved_record(
        &self,
        record: TelegramInstallationSetup,
        previous: Option<&TelegramInstallationSetup>,
        bot_token: &SecretString,
        webhook_secret: &SecretString,
    ) -> Result<TelegramInstallationSetup, TelegramSetupError> {
        self.put_secret(record.bot_token_handle.clone(), bot_token.clone())
            .await?;
        if let Err(error) = self
            .put_secret(record.webhook_secret_handle.clone(), webhook_secret.clone())
            .await
        {
            self.best_effort_delete_secret(&record.bot_token_handle)
                .await;
            return Err(error);
        }
        match self
            .state
            .replace_telegram_installation_setup_if_current(previous, Some(&record))
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                self.best_effort_delete_secret(&record.bot_token_handle)
                    .await;
                self.best_effort_delete_secret(&record.webhook_secret_handle)
                    .await;
                return Err(TelegramSetupError::ConcurrentUpdate);
            }
            Err(error) => {
                self.best_effort_delete_secret(&record.bot_token_handle)
                    .await;
                self.best_effort_delete_secret(&record.webhook_secret_handle)
                    .await;
                return Err(error);
            }
        }
        Ok(record)
    }

    /// Best-effort provider-side compensation once `setWebhook` has already
    /// mutated Telegram but the local save (or its post-save activation)
    /// failed: a same-bot save restores the previous registration so the
    /// restored record keeps verifying webhooks; a different (or first) bot's
    /// fresh registration is deleted so Telegram stops delivering to a
    /// deployment that never persisted it. Compensation failures are logged
    /// and never mask the original error — the admin sees the save failure
    /// and retries.
    /// Restore the previous record when post-save extension activation fails,
    /// so store state and runtime never split-brain (Slack rollback contract).
    /// The provider side rolls back too: Telegram is still registered with
    /// the SAVED revision's URL/secret, so without compensation the restored
    /// record would reject every subsequent webhook.
    /// Clear the setup through a durable cleanup saga. Pairing records and
    /// history are deliberately retained; provider and secret cleanup must be
    /// confirmed before the setup tombstone is finalized.
    pub async fn clear(&self) -> Result<(), TelegramSetupError> {
        self.recover_pending_setup_rollback().await?;
        let Some(setup) = self.state.telegram_installation_setup_for_cleanup().await? else {
            return Ok(());
        };
        if !self
            .state
            .begin_telegram_installation_setup_cleanup(&setup)
            .await?
        {
            return Err(TelegramSetupError::ConcurrentUpdate);
        }

        // The durable Clearing record above retains both handles until every
        // external cleanup step is confirmed. Any failure is retryable by a
        // later clear call, including after process restart.
        let token = self.secret_material(&setup.bot_token_handle).await?;
        self.bot_api.delete_webhook(&token).await?;
        self.delete_secret(&setup.webhook_secret_handle).await?;
        self.delete_secret(&setup.bot_token_handle).await?;
        if !self
            .state
            .finish_telegram_installation_setup_cleanup(&setup)
            .await?
        {
            return Err(TelegramSetupError::ConcurrentUpdate);
        }
        Ok(())
    }

    /// Resolve the current bot token material (ingress/egress wiring).
    pub async fn bot_token(&self) -> Result<Option<SecretString>, TelegramSetupError> {
        let Some(setup) = self.current_setup().await? else {
            return Ok(None);
        };
        Ok(Some(self.secret_material(&setup.bot_token_handle).await?))
    }

    /// Resolve the current webhook shared secret (ingress verification).
    pub async fn webhook_secret(&self) -> Result<Option<SecretString>, TelegramSetupError> {
        let Some(setup) = self.current_setup().await? else {
            return Ok(None);
        };
        Ok(Some(
            self.secret_material(&setup.webhook_secret_handle).await?,
        ))
    }

    /// Resolve secret material from the caller's already-captured setup
    /// snapshot. Revision builders use this instead of re-reading `current` so
    /// authentication evidence cannot be assembled from two setup revisions.
    pub(crate) async fn webhook_secret_for_setup(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<SecretString, TelegramSetupError> {
        self.secret_material(&setup.webhook_secret_handle).await
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
        save_attempt: &str,
    ) -> Result<TelegramInstallationSetup, TelegramSetupError> {
        let installation_key = format!("tg-bot-{}", identity.id);
        Ok(TelegramInstallationSetup {
            bot_id: identity.id,
            bot_username: identity.username.clone(),
            webhook_url,
            bot_token_handle: bot_token_handle(
                &self.tenant_id,
                &installation_key,
                revision,
                save_attempt,
            )?,
            webhook_secret_handle: webhook_secret_handle(
                &self.tenant_id,
                &installation_key,
                revision,
                save_attempt,
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

    pub(super) async fn delete_secret(
        &self,
        handle: &SecretHandle,
    ) -> Result<(), TelegramSetupError> {
        self.secret_store
            .delete(&self.secret_scope(), handle)
            .await
            .map(|_| ())
            .map_err(map_secret_error)
    }

    pub(super) async fn secret_material(
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

    pub(super) fn secret_scope(&self) -> ResourceScope {
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

fn mint_save_attempt_id() -> String {
    let random: [u8; 16] = rand::random();
    sha256_hex(&random).chars().take(16).collect()
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
    save_attempt: &str,
) -> Result<SecretHandle, TelegramSetupError> {
    secret_handle_for_installation(
        TELEGRAM_BOT_TOKEN_HANDLE_PREFIX,
        tenant_id,
        installation_key,
        revision,
        save_attempt,
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
    save_attempt: &str,
) -> Result<SecretHandle, TelegramSetupError> {
    secret_handle_for_installation(
        TELEGRAM_WEBHOOK_SECRET_HANDLE_PREFIX,
        tenant_id,
        installation_key,
        revision,
        save_attempt,
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
    save_attempt: &str,
) -> Result<SecretHandle, ironclaw_host_api::HostApiError> {
    let digest = sha256_hex(&secret_handle_key_material(
        tenant_id,
        installation_key,
        save_attempt,
    ));
    let digest_prefix: String = digest.chars().take(INSTALLATION_HANDLE_HASH_LEN).collect();
    SecretHandle::new(format!("{prefix}_{digest_prefix}_v{revision}"))
}

fn secret_handle_key_material(
    tenant_id: &TenantId,
    installation_key: &str,
    save_attempt: &str,
) -> Vec<u8> {
    let mut key = b"telegram-installation-secret:v1".to_vec();
    append_length_prefixed(&mut key, tenant_id.as_str().as_bytes());
    append_length_prefixed(&mut key, installation_key.as_bytes());
    append_length_prefixed(&mut key, save_attempt.as_bytes());
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
