//! Telegram pairing: IronClaw-issued codes, deep-link consume, identity
//! binding, and blocked-run resume dispatch.
//!
//! Direction is web→telegram (WebGeneratedCode): IronClaw mints a short-lived
//! single-use code presented as `https://t.me/<bot>?start=<CODE>`; the webhook
//! consumes it (`/start <CODE>` or a bare live code) and binds the sending
//! Telegram account to the code's Reborn user. Codes expire; gates don't —
//! the parked `BlockedAuth` run is provider-keyed (`telegram`), so pairing
//! with the n-th rotated code still resumes it via the standard
//! auth-continuation fan-out.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthProductScope, AuthProviderId,
    AuthSurface,
};
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::product_auth::api::auth::RebornAuthContinuationDispatcher;
use crate::telegram::telegram_actor_identity::{
    TELEGRAM_IDENTITY_PROVIDER, telegram_user_identity_provider_user_id,
};
use crate::telegram::telegram_setup::{TelegramSetupError, TelegramSetupService};

pub(crate) const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub(crate) const PAIRING_CODE_LEN: usize = 8;
pub(crate) const PAIRING_TTL_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TelegramPairingRecord {
    pub(crate) code: String,
    pub(crate) tenant_id: TenantId,
    pub(crate) user_id: UserId,
    pub(crate) installation_id: AdapterInstallationId,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) consumed_at: Option<DateTime<Utc>>,
}

impl TelegramPairingRecord {
    pub(crate) fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.consumed_at.is_none() && self.expires_at > now
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum TelegramPairingError {
    #[error("telegram pairing store unavailable: {reason}")]
    StoreUnavailable { reason: String },
    #[error("telegram is not configured by an administrator yet")]
    NotConfigured,
    #[error("telegram setup unavailable: {reason}")]
    Setup { reason: String },
    #[error("pairing continuation dispatch failed: {reason}")]
    ContinuationDispatch { reason: String },
}

impl From<TelegramSetupError> for TelegramPairingError {
    fn from(error: TelegramSetupError) -> Self {
        TelegramPairingError::Setup {
            reason: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum TelegramBindingError {
    #[error("telegram binding store unavailable: {reason}")]
    StoreUnavailable { reason: String },
    #[error("this telegram account is already paired to another user")]
    AlreadyBoundToOtherUser,
}

/// Pending pairing codes: one live code per user per installation.
#[async_trait]
pub(crate) trait TelegramPairingStore: Send + Sync + std::fmt::Debug {
    /// Insert the caller's pending code, replacing (rotating) any live one.
    async fn upsert_pending_pairing(
        &self,
        record: TelegramPairingRecord,
    ) -> Result<(), TelegramPairingError>;

    /// Live (unexpired, unconsumed) record for an uppercased code.
    async fn live_pairing_for_code(
        &self,
        code: &str,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError>;

    async fn live_pairing_for_user(
        &self,
        user_id: &UserId,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError>;

    async fn mark_consumed(&self, code: &str) -> Result<(), TelegramPairingError>;

    async fn invalidate_for_user(&self, user_id: &UserId) -> Result<(), TelegramPairingError>;
}

/// Write/delete side of the telegram identity bindings (reads go through
/// [`crate::slack::slack_actor_identity::RebornUserIdentityLookup`]).
#[async_trait]
pub(crate) trait TelegramUserBindingStore: Send + Sync + std::fmt::Debug {
    /// Bind `{installation}:{telegram_user_id}` → user. Rebinding the same
    /// pair is idempotent; a different user yields `AlreadyBoundToOtherUser`.
    async fn bind_telegram_user(
        &self,
        provider_user_id: &str,
        user_id: &UserId,
        epoch: &str,
    ) -> Result<(), TelegramBindingError>;

    /// Remove every binding for `user_id` under the installation prefix.
    /// Returns the removed provider user ids.
    async fn unbind_telegram_users_for_user(
        &self,
        user_id: &UserId,
        installation_prefix: &str,
    ) -> Result<Vec<String>, TelegramBindingError>;

    /// The bound user for a provider id, if any (conflict checks + consume).
    async fn bound_user_for(
        &self,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, TelegramBindingError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TelegramDmTarget {
    pub(crate) user_id: UserId,
    pub(crate) chat_id: i64,
}

/// Paired users' DM chat ids — the outbound delivery targets.
#[async_trait]
pub(crate) trait TelegramDmTargetStore: Send + Sync + std::fmt::Debug {
    async fn upsert_dm_target(
        &self,
        installation_id: &AdapterInstallationId,
        target: TelegramDmTarget,
    ) -> Result<(), TelegramPairingError>;

    async fn dm_target_for_user(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<Option<TelegramDmTarget>, TelegramPairingError>;

    async fn delete_dm_target_for_user(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<(), TelegramPairingError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PairingIssue {
    pub(crate) code: String,
    pub(crate) deep_link: String,
    pub(crate) expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TelegramPairingStatus {
    pub(crate) connected: bool,
    pub(crate) pending: Option<PairingIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PairingConsumeOutcome {
    Paired { user_id: UserId },
    AlreadyPairedSameUser { user_id: UserId },
    AlreadyBoundToOtherUser,
    ExpiredOrUnknown,
}

pub(crate) struct TelegramPairingService {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    setup: Arc<TelegramSetupService>,
    pairing_store: Arc<dyn TelegramPairingStore>,
    binding_store: Arc<dyn TelegramUserBindingStore>,
    dm_target_store: Arc<dyn TelegramDmTargetStore>,
    continuation: Arc<dyn RebornAuthContinuationDispatcher>,
}

impl std::fmt::Debug for TelegramPairingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramPairingService").finish()
    }
}

impl TelegramPairingService {
    #[allow(clippy::too_many_arguments)]
    // arch-exempt: too_many_args, mirrors the slack binder construction shape;
    // folds into the telegram host mounts bundle, cleanup rides the #6116 port.
    pub(crate) fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        setup: Arc<TelegramSetupService>,
        pairing_store: Arc<dyn TelegramPairingStore>,
        binding_store: Arc<dyn TelegramUserBindingStore>,
        dm_target_store: Arc<dyn TelegramDmTargetStore>,
        continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            setup,
            pairing_store,
            binding_store,
            dm_target_store,
            continuation,
        }
    }

    /// Mint (or rotate) the caller's pairing code. Fails closed when the
    /// admin has not configured the bot — no code is ever minted first.
    pub(crate) async fn issue_or_rotate(
        &self,
        caller: &UserId,
    ) -> Result<PairingIssue, TelegramPairingError> {
        let setup = self
            .setup
            .current_setup()
            .await?
            .ok_or(TelegramPairingError::NotConfigured)?;
        let installation_id = setup
            .installation_id()
            .map_err(TelegramPairingError::from)?;
        let now = Utc::now();
        let record = TelegramPairingRecord {
            code: mint_pairing_code(),
            tenant_id: self.tenant_id.clone(),
            user_id: caller.clone(),
            installation_id,
            created_at: now,
            expires_at: now + Duration::minutes(PAIRING_TTL_MINUTES),
            consumed_at: None,
        };
        self.pairing_store
            .upsert_pending_pairing(record.clone())
            .await?;
        Ok(pairing_issue(&record, &setup.bot_username))
    }

    pub(crate) async fn status_for(
        &self,
        caller: &UserId,
    ) -> Result<TelegramPairingStatus, TelegramPairingError> {
        let setup = self.setup.current_setup().await?;
        let connected = match &setup {
            Some(setup) => {
                let installation_id = setup
                    .installation_id()
                    .map_err(TelegramPairingError::from)?;
                self.dm_target_store
                    .dm_target_for_user(&installation_id, caller)
                    .await?
                    .is_some()
            }
            None => false,
        };
        let pending = match (&setup, connected) {
            (Some(setup), false) => self
                .pairing_store
                .live_pairing_for_user(caller)
                .await?
                .filter(|record| record.is_live(Utc::now()))
                .map(|record| pairing_issue(&record, &setup.bot_username)),
            _ => None,
        };
        Ok(TelegramPairingStatus { connected, pending })
    }

    /// Consume a code arriving over the verified webhook from a private chat.
    pub(crate) async fn consume(
        &self,
        raw_code: &str,
        telegram_user_id: &str,
        chat_id: i64,
    ) -> Result<PairingConsumeOutcome, TelegramPairingError> {
        let code = raw_code.trim().to_ascii_uppercase();
        if code.len() != PAIRING_CODE_LEN
            || !code
                .bytes()
                .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte))
        {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        }
        let Some(record) = self.pairing_store.live_pairing_for_code(&code).await? else {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        };
        if !record.is_live(Utc::now()) {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        }
        let provider_user_id =
            telegram_user_identity_provider_user_id(&record.installation_id, telegram_user_id);
        match self.binding_store.bound_user_for(&provider_user_id).await {
            Ok(Some(existing)) if existing != record.user_id => {
                return Ok(PairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
            Ok(Some(existing)) => {
                self.pairing_store.mark_consumed(&code).await?;
                return Ok(PairingConsumeOutcome::AlreadyPairedSameUser { user_id: existing });
            }
            Ok(None) => {}
            Err(error) => {
                return Err(TelegramPairingError::StoreUnavailable {
                    reason: error.to_string(),
                });
            }
        }
        match self
            .binding_store
            .bind_telegram_user(&provider_user_id, &record.user_id, &code)
            .await
        {
            Ok(()) => {}
            Err(TelegramBindingError::AlreadyBoundToOtherUser) => {
                return Ok(PairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
            Err(error) => {
                return Err(TelegramPairingError::StoreUnavailable {
                    reason: error.to_string(),
                });
            }
        }
        self.dm_target_store
            .upsert_dm_target(
                &record.installation_id,
                TelegramDmTarget {
                    user_id: record.user_id.clone(),
                    chat_id,
                },
            )
            .await?;
        self.pairing_store.mark_consumed(&code).await?;
        self.dispatch_pairing_completion(&record.user_id).await?;
        Ok(PairingConsumeOutcome::Paired {
            user_id: record.user_id,
        })
    }

    /// Unpair the caller: bindings + DM target removed, pending code
    /// invalidated. Only this user is affected; history is retained.
    pub(crate) async fn unpair(&self, caller: &UserId) -> Result<(), TelegramPairingError> {
        self.pairing_store.invalidate_for_user(caller).await?;
        let Some(setup) = self.setup.current_setup().await? else {
            return Ok(());
        };
        let installation_id = setup
            .installation_id()
            .map_err(TelegramPairingError::from)?;
        let prefix = format!("{}:", installation_id.as_str());
        self.binding_store
            .unbind_telegram_users_for_user(caller, &prefix)
            .await
            .map_err(|error| TelegramPairingError::StoreUnavailable {
                reason: error.to_string(),
            })?;
        self.dm_target_store
            .delete_dm_target_for_user(&installation_id, caller)
            .await?;
        Ok(())
    }

    /// Emit the standard auth-continuation completion so the
    /// `BlockedAuthResumeFanout` resumes every run parked on provider
    /// `telegram` for this user. `SetupOnly` deliberately: the resumed run
    /// re-runs `extension_activate` and re-checks pairedness itself.
    async fn dispatch_pairing_completion(
        &self,
        user_id: &UserId,
    ) -> Result<(), TelegramPairingError> {
        let provider = AuthProviderId::new(TELEGRAM_IDENTITY_PROVIDER).map_err(|error| {
            TelegramPairingError::ContinuationDispatch {
                reason: error.to_string(),
            }
        })?;
        let event = AuthContinuationEvent {
            flow_id: AuthFlowId::new(),
            scope: AuthProductScope::new(
                ResourceScope {
                    tenant_id: self.tenant_id.clone(),
                    user_id: user_id.clone(),
                    agent_id: Some(self.agent_id.clone()),
                    project_id: self.project_id.clone(),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                AuthSurface::Callback,
            ),
            continuation: AuthContinuationRef::SetupOnly,
            provider,
            credential_account_id: None,
            emitted_at: Utc::now(),
        };
        self.continuation
            .dispatch_auth_continuation(event)
            .await
            .map_err(|error| TelegramPairingError::ContinuationDispatch {
                reason: error.to_string(),
            })
    }
}

fn pairing_issue(record: &TelegramPairingRecord, bot_username: &str) -> PairingIssue {
    PairingIssue {
        code: record.code.clone(),
        deep_link: format!("https://t.me/{bot_username}?start={}", record.code),
        expires_at: record.expires_at,
    }
}

fn mint_pairing_code() -> String {
    (0..PAIRING_CODE_LEN)
        .map(|_| {
            let index = rand::random_range(0..PAIRING_CODE_ALPHABET.len());
            PAIRING_CODE_ALPHABET[index] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    use ironclaw_auth::AuthProductError;
    use ironclaw_secrets::InMemorySecretStore;
    use secrecy::SecretString;

    use super::*;
    use crate::telegram::telegram_bot_api::{
        TelegramBotApi, TelegramBotApiError, TelegramBotIdentity,
    };
    use crate::telegram::telegram_setup::{
        TelegramInstallationSetup, TelegramInstallationSetupStore, TelegramInstallationSetupUpdate,
        TelegramSetupService,
    };

    #[derive(Debug, Default)]
    struct InMemoryPairingStore {
        records: StdMutex<Vec<TelegramPairingRecord>>,
    }

    #[async_trait]
    impl TelegramPairingStore for InMemoryPairingStore {
        async fn upsert_pending_pairing(
            &self,
            record: TelegramPairingRecord,
        ) -> Result<(), TelegramPairingError> {
            let mut records = self.records.lock().expect("lock");
            records.retain(|existing| {
                existing.user_id != record.user_id || existing.consumed_at.is_some()
            });
            records.push(record);
            Ok(())
        }

        async fn live_pairing_for_code(
            &self,
            code: &str,
        ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
            let now = Utc::now();
            Ok(self
                .records
                .lock()
                .expect("lock")
                .iter()
                .find(|record| record.code.eq_ignore_ascii_case(code) && record.is_live(now))
                .cloned())
        }

        async fn live_pairing_for_user(
            &self,
            user_id: &UserId,
        ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
            let now = Utc::now();
            Ok(self
                .records
                .lock()
                .expect("lock")
                .iter()
                .find(|record| &record.user_id == user_id && record.is_live(now))
                .cloned())
        }

        async fn mark_consumed(&self, code: &str) -> Result<(), TelegramPairingError> {
            let mut records = self.records.lock().expect("lock");
            for record in records.iter_mut() {
                if record.code.eq_ignore_ascii_case(code) {
                    record.consumed_at = Some(Utc::now());
                }
            }
            Ok(())
        }

        async fn invalidate_for_user(&self, user_id: &UserId) -> Result<(), TelegramPairingError> {
            let mut records = self.records.lock().expect("lock");
            records.retain(|record| &record.user_id != user_id);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct InMemoryBindingStore {
        bindings: StdMutex<HashMap<String, (UserId, String)>>,
    }

    #[async_trait]
    impl TelegramUserBindingStore for InMemoryBindingStore {
        async fn bind_telegram_user(
            &self,
            provider_user_id: &str,
            user_id: &UserId,
            epoch: &str,
        ) -> Result<(), TelegramBindingError> {
            let mut bindings = self.bindings.lock().expect("lock");
            if let Some((existing, _)) = bindings.get(provider_user_id)
                && existing != user_id
            {
                return Err(TelegramBindingError::AlreadyBoundToOtherUser);
            }
            bindings.insert(
                provider_user_id.to_string(),
                (user_id.clone(), epoch.to_string()),
            );
            Ok(())
        }

        async fn unbind_telegram_users_for_user(
            &self,
            user_id: &UserId,
            installation_prefix: &str,
        ) -> Result<Vec<String>, TelegramBindingError> {
            let mut bindings = self.bindings.lock().expect("lock");
            let removed: Vec<String> = bindings
                .iter()
                .filter(|(key, (bound, _))| {
                    bound == user_id && key.starts_with(installation_prefix)
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in &removed {
                bindings.remove(key);
            }
            Ok(removed)
        }

        async fn bound_user_for(
            &self,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, TelegramBindingError> {
            Ok(self
                .bindings
                .lock()
                .expect("lock")
                .get(provider_user_id)
                .map(|(user, _)| user.clone()))
        }
    }

    #[derive(Debug, Default)]
    struct InMemoryDmTargetStore {
        targets: StdMutex<HashMap<(String, String), TelegramDmTarget>>,
    }

    #[async_trait]
    impl TelegramDmTargetStore for InMemoryDmTargetStore {
        async fn upsert_dm_target(
            &self,
            installation_id: &AdapterInstallationId,
            target: TelegramDmTarget,
        ) -> Result<(), TelegramPairingError> {
            self.targets.lock().expect("lock").insert(
                (
                    installation_id.as_str().to_string(),
                    target.user_id.as_str().to_string(),
                ),
                target,
            );
            Ok(())
        }

        async fn dm_target_for_user(
            &self,
            installation_id: &AdapterInstallationId,
            user_id: &UserId,
        ) -> Result<Option<TelegramDmTarget>, TelegramPairingError> {
            Ok(self
                .targets
                .lock()
                .expect("lock")
                .get(&(
                    installation_id.as_str().to_string(),
                    user_id.as_str().to_string(),
                ))
                .cloned())
        }

        async fn delete_dm_target_for_user(
            &self,
            installation_id: &AdapterInstallationId,
            user_id: &UserId,
        ) -> Result<(), TelegramPairingError> {
            self.targets.lock().expect("lock").remove(&(
                installation_id.as_str().to_string(),
                user_id.as_str().to_string(),
            ));
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct RecordingDispatcher {
        events: StdMutex<Vec<AuthContinuationEvent>>,
    }

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RecordingDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            self.events.lock().expect("lock").push(event);
            Ok(())
        }
    }

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

    #[derive(Debug)]
    struct OkBotApi;

    #[async_trait]
    impl TelegramBotApi for OkBotApi {
        async fn get_me(
            &self,
            _bot_token: &SecretString,
        ) -> Result<TelegramBotIdentity, TelegramBotApiError> {
            Ok(TelegramBotIdentity {
                id: 777,
                username: "ironclaw_qa_bot".to_string(),
            })
        }

        async fn set_webhook(
            &self,
            _bot_token: &SecretString,
            _url: &str,
            _secret_token: &SecretString,
        ) -> Result<(), TelegramBotApiError> {
            Ok(())
        }

        async fn delete_webhook(
            &self,
            _bot_token: &SecretString,
        ) -> Result<(), TelegramBotApiError> {
            Ok(())
        }

        async fn send_message(
            &self,
            _bot_token: &SecretString,
            _chat_id: i64,
            _text: &str,
        ) -> Result<(), TelegramBotApiError> {
            Ok(())
        }
    }

    struct Fixture {
        service: TelegramPairingService,
        dispatcher: Arc<RecordingDispatcher>,
        setup: Arc<TelegramSetupService>,
    }

    async fn fixture(configured: bool) -> Fixture {
        let tenant_id = TenantId::new("tenant-a").expect("tenant");
        let agent_id = AgentId::new("agent-a").expect("agent");
        let setup = Arc::new(TelegramSetupService::new(
            tenant_id.clone(),
            agent_id.clone(),
            None,
            UserId::new("operator").expect("user"),
            Arc::new(InMemorySetupStore::default()),
            Arc::new(InMemorySecretStore::new()),
            Arc::new(OkBotApi),
            Some("https://ironclaw.example".to_string()),
        ));
        if configured {
            setup
                .save_with_previous(TelegramInstallationSetupUpdate {
                    bot_token: Some(SecretString::from("123:abc".to_string())),
                    webhook_url_override: None,
                })
                .await
                .expect("setup saves");
        }
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let service = TelegramPairingService::new(
            tenant_id,
            agent_id,
            None,
            Arc::clone(&setup),
            Arc::new(InMemoryPairingStore::default()),
            Arc::new(InMemoryBindingStore::default()),
            Arc::new(InMemoryDmTargetStore::default()),
            Arc::clone(&dispatcher) as Arc<dyn RebornAuthContinuationDispatcher>,
        );
        Fixture {
            service,
            dispatcher,
            setup,
        }
    }

    fn user(name: &str) -> UserId {
        UserId::new(name).expect("user")
    }

    #[tokio::test]
    async fn issue_mints_code_with_deep_link_and_ttl() {
        let fixture = fixture(true).await;
        let issue = fixture
            .service
            .issue_or_rotate(&user("ben"))
            .await
            .expect("issue");
        assert_eq!(issue.code.len(), PAIRING_CODE_LEN);
        assert!(
            issue
                .code
                .bytes()
                .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte))
        );
        assert_eq!(
            issue.deep_link,
            format!("https://t.me/ironclaw_qa_bot?start={}", issue.code)
        );
        assert!(issue.expires_at > Utc::now());
    }

    #[tokio::test]
    async fn issue_fails_closed_when_unconfigured() {
        let fixture = fixture(false).await;
        let error = fixture
            .service
            .issue_or_rotate(&user("ben"))
            .await
            .expect_err("no code without admin setup");
        assert_eq!(error, TelegramPairingError::NotConfigured);
    }

    #[tokio::test]
    async fn reissue_rotates_and_kills_the_old_code() {
        let fixture = fixture(true).await;
        let first = fixture
            .service
            .issue_or_rotate(&user("ben"))
            .await
            .expect("first");
        let second = fixture
            .service
            .issue_or_rotate(&user("ben"))
            .await
            .expect("second");
        assert_ne!(first.code, second.code);
        let outcome = fixture
            .service
            .consume(&first.code, "tg-1", 100)
            .await
            .expect("consume old");
        assert_eq!(outcome, PairingConsumeOutcome::ExpiredOrUnknown);
        let outcome = fixture
            .service
            .consume(&second.code, "tg-1", 100)
            .await
            .expect("consume new");
        assert!(matches!(outcome, PairingConsumeOutcome::Paired { .. }));
    }

    #[tokio::test]
    async fn consume_happy_path_binds_targets_and_dispatches() {
        let fixture = fixture(true).await;
        let ben = user("ben");
        let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        let outcome = fixture
            .service
            .consume(&issue.code.to_ascii_lowercase(), "tg-100", 555)
            .await
            .expect("consume");
        assert_eq!(
            outcome,
            PairingConsumeOutcome::Paired {
                user_id: ben.clone()
            }
        );

        let status = fixture.service.status_for(&ben).await.expect("status");
        assert!(status.connected);
        assert!(status.pending.is_none());

        let events = fixture.dispatcher.events.lock().expect("lock").clone();
        assert_eq!(events.len(), 1, "exactly one continuation dispatch");
        assert_eq!(events[0].provider.as_str(), "telegram");
        assert!(matches!(
            events[0].continuation,
            AuthContinuationRef::SetupOnly
        ));
        assert_eq!(events[0].scope.resource.user_id, ben);

        let replay = fixture
            .service
            .consume(&issue.code, "tg-other", 556)
            .await
            .expect("replay");
        assert_eq!(
            replay,
            PairingConsumeOutcome::ExpiredOrUnknown,
            "single-use"
        );
    }

    #[tokio::test]
    async fn consume_unknown_or_malformed_never_dispatches() {
        let fixture = fixture(true).await;
        for code in ["NOPE1234", "short", "!!!!!!!!"] {
            let outcome = fixture
                .service
                .consume(code, "tg-1", 1)
                .await
                .expect("consume");
            assert_eq!(outcome, PairingConsumeOutcome::ExpiredOrUnknown);
        }
        assert!(fixture.dispatcher.events.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn telegram_account_bound_to_other_user_is_refused() {
        let fixture = fixture(true).await;
        let ben = user("ben");
        let illia = user("illia");
        let ben_issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .service
            .consume(&ben_issue.code, "tg-shared", 1)
            .await
            .expect("ben pairs");
        let illia_issue = fixture
            .service
            .issue_or_rotate(&illia)
            .await
            .expect("issue");
        let outcome = fixture
            .service
            .consume(&illia_issue.code, "tg-shared", 2)
            .await
            .expect("consume");
        assert_eq!(outcome, PairingConsumeOutcome::AlreadyBoundToOtherUser);
        let ben_status = fixture.service.status_for(&ben).await.expect("status");
        assert!(ben_status.connected, "original binding intact");
    }

    #[tokio::test]
    async fn same_user_re_pair_is_idempotent() {
        let fixture = fixture(true).await;
        let ben = user("ben");
        let first = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .service
            .consume(&first.code, "tg-100", 1)
            .await
            .expect("pair");
        let second = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        let outcome = fixture
            .service
            .consume(&second.code, "tg-100", 1)
            .await
            .expect("re-pair");
        assert_eq!(
            outcome,
            PairingConsumeOutcome::AlreadyPairedSameUser { user_id: ben }
        );
    }

    #[tokio::test]
    async fn unpair_removes_binding_target_and_pending_code() {
        let fixture = fixture(true).await;
        let ben = user("ben");
        let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .service
            .consume(&issue.code, "tg-100", 1)
            .await
            .expect("pair");
        fixture.service.unpair(&ben).await.expect("unpair");
        let status = fixture.service.status_for(&ben).await.expect("status");
        assert!(!status.connected);
        let fresh = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        let outcome = fixture
            .service
            .consume(&fresh.code, "tg-100", 1)
            .await
            .expect("re-pair after unpair");
        assert!(matches!(outcome, PairingConsumeOutcome::Paired { .. }));
        drop(fixture.setup);
    }
}
