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

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ironclaw_auth::{
    AuthContinuationEvent, AuthContinuationRef, AuthFlowId, AuthProductScope, AuthProviderId,
    AuthSurface,
};
use ironclaw_conversations::{
    AdapterKind, ConversationActorPairingService, ExpectedExternalActorOwner,
    ExternalActorBindingEpoch, ExternalActorRef,
};
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::state::FilesystemTelegramHostState;
use crate::telegram_actor_identity::{
    TELEGRAM_IDENTITY_PROVIDER, telegram_user_identity_provider_user_id,
};
use crate::telegram_setup::{TelegramSetupError, TelegramSetupService};
use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;

pub const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const PAIRING_CODE_LEN: usize = 8;
pub const PAIRING_TTL_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramPairingRecord {
    pub code: String,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub installation_id: AdapterInstallationId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

impl TelegramPairingRecord {
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.consumed_at.is_none() && self.expires_at > now
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelegramPairingError {
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
pub enum TelegramBindingError {
    #[error("telegram binding store unavailable: {reason}")]
    StoreUnavailable { reason: String },
    #[error("this telegram account is already paired to another user")]
    AlreadyBoundToOtherUser,
}

/// A binding removed by the concrete host state's user-scoped unbind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedTelegramBinding {
    pub provider_user_id: String,
    /// `None` only when the stored record was unreadable at removal time; the
    /// conditional pairing cleanup then fails safe (owner-changed no-op).
    pub epoch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramDmTarget {
    pub user_id: UserId,
    pub chat_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairingIssue {
    pub code: String,
    pub deep_link: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TelegramPairingStatus {
    pub connected: bool,
    pub pending: Option<PairingIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingConsumeOutcome {
    Paired { user_id: UserId },
    AlreadyPairedSameUser { user_id: UserId },
    AlreadyBoundToOtherUser,
    ExpiredOrUnknown,
}

pub struct TelegramPairingService {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    setup: Arc<TelegramSetupService>,
    state: Arc<FilesystemTelegramHostState>,
    continuation: Arc<dyn RebornAuthContinuationDispatcher>,
    /// Conversation-actor pairing cleanup on unpair (Slack disconnect
    /// parity): without it a re-paired chat resurrects its old thread and
    /// any run parked there.
    conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
}

impl std::fmt::Debug for TelegramPairingService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramPairingService").finish()
    }
}

impl TelegramPairingService {
    // arch-exempt: too_many_args, mirrors the slack binder shape until the telegram host mounts bundle owns the aggregation, plan #6116
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        setup: Arc<TelegramSetupService>,
        state: Arc<FilesystemTelegramHostState>,
        continuation: Arc<dyn RebornAuthContinuationDispatcher>,
        conversation_actor_pairings: Arc<dyn ConversationActorPairingService>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            setup,
            state,
            continuation,
            conversation_actor_pairings,
        }
    }

    /// Mint (or rotate) the caller's pairing code. Fails closed when the
    /// admin has not configured the bot — no code is ever minted first.
    pub async fn issue_or_rotate(
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
        self.state.upsert_pending_pairing(record.clone()).await?;
        Ok(pairing_issue(&record, &setup.bot_username))
    }

    pub async fn status_for(
        &self,
        caller: &UserId,
    ) -> Result<TelegramPairingStatus, TelegramPairingError> {
        let setup = self.setup.current_setup().await?;
        let connected = match &setup {
            Some(setup) => {
                let installation_id = setup
                    .installation_id()
                    .map_err(TelegramPairingError::from)?;
                self.state
                    .dm_target_for_user(&installation_id, caller)
                    .await?
                    .is_some()
            }
            None => false,
        };
        let pending = match (&setup, connected) {
            (Some(setup), false) => self
                .state
                .live_pairing_for_user(caller)
                .await?
                .filter(|record| record.is_live(Utc::now()))
                .map(|record| pairing_issue(&record, &setup.bot_username)),
            _ => None,
        };
        Ok(TelegramPairingStatus { connected, pending })
    }

    /// Consume a code arriving over the verified webhook from a private chat.
    ///
    /// Ordering is claim-first: the code is atomically consumed (single
    /// winner) BEFORE any identity/target side effect, so two concurrent
    /// consumers of one code can never both bind. Completion (DM target +
    /// continuation dispatch) is idempotently repairable: a sender already
    /// bound to the code's user re-runs the completion effects — including on
    /// an already-consumed code — so a consume that failed after the claim is
    /// recovered by re-sending a code instead of stranding the blocked run.
    pub async fn consume(
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
        let Some(record) = self.state.pairing_for_code(&code).await? else {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        };
        let provider_user_id =
            telegram_user_identity_provider_user_id(&record.installation_id, telegram_user_id);
        match self.state.bound_user_for(&provider_user_id).await {
            Ok(Some(existing)) if existing == record.user_id => {
                // Repair path: burn the code if it is still live (whoever
                // wins — the sender is already bound), then re-run completion.
                let _already_burned = self.state.claim_pairing(&code).await?;
                return self
                    .complete_pairing(&record, chat_id)
                    .await
                    .map(|()| PairingConsumeOutcome::AlreadyPairedSameUser { user_id: existing });
            }
            Ok(Some(_other)) => {
                if !record.is_live(Utc::now()) {
                    return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
                }
                // Refusal keeps the live code intact for its owner.
                return Ok(PairingConsumeOutcome::AlreadyBoundToOtherUser);
            }
            Ok(None) => {}
            Err(error) => {
                return Err(TelegramPairingError::StoreUnavailable {
                    reason: error.to_string(),
                });
            }
        }
        // Single-consumer claim BEFORE identity/target writes: exactly one
        // concurrent consumer of a live code proceeds past this point.
        let Some(record) = self.state.claim_pairing(&code).await? else {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        };
        match self
            .state
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
        self.complete_pairing(&record, chat_id).await?;
        Ok(PairingConsumeOutcome::Paired {
            user_id: record.user_id,
        })
    }

    /// The idempotent completion tail shared by first-time pairing and the
    /// repair path: record the DM delivery target and dispatch the blocked-run
    /// continuation.
    async fn complete_pairing(
        &self,
        record: &TelegramPairingRecord,
        chat_id: i64,
    ) -> Result<(), TelegramPairingError> {
        self.state
            .upsert_dm_target(
                &record.installation_id,
                TelegramDmTarget {
                    user_id: record.user_id.clone(),
                    chat_id,
                },
            )
            .await?;
        self.dispatch_pairing_completion(&record.user_id).await
    }

    /// Unpair the caller: bindings + DM targets removed, pending code
    /// invalidated. Only this user is affected; history is retained.
    ///
    /// Deliberately independent of the current bot setup: an admin clearing
    /// the deployment must not orphan a user's durable bindings — those would
    /// silently resurrect the connection when the same bot is reconfigured
    /// even though the user disconnected. Bindings are removed across every
    /// installation, and DM targets are derived from the removed provider ids
    /// plus the current setup (when one exists).
    pub async fn unpair(&self, caller: &UserId) -> Result<(), TelegramPairingError> {
        self.state.invalidate_for_user(caller).await?;
        let removed = self
            .state
            .unbind_telegram_users_for_user(caller, None)
            .await
            .map_err(|error| TelegramPairingError::StoreUnavailable {
                reason: error.to_string(),
            })?;
        // Conversation-actor pairing cleanup (Slack disconnect parity,
        // 2026-07-17): the workflow paired this chat's external actor to the
        // caller at inbound; leaving that pairing behind re-attaches a
        // re-paired user to their old thread — and any run parked on it.
        let adapter_kind = AdapterKind::new(crate::telegram_actor_identity::TELEGRAM_V2_ADAPTER_ID)
            .map_err(|error| TelegramPairingError::StoreUnavailable {
                reason: format!("telegram adapter kind invalid: {error}"),
            })?;
        for removed_binding in &removed {
            let Some((installation, telegram_user_id)) =
                removed_binding.provider_user_id.split_once(':')
            else {
                continue;
            };
            let installation_id = ironclaw_conversations::AdapterInstallationId::new(installation)
                .map_err(|error| TelegramPairingError::StoreUnavailable {
                    reason: format!("stored telegram binding installation invalid: {error}"),
                })?;
            let actor_ref = ExternalActorRef::new(
                ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND,
                telegram_user_id,
            )
            .map_err(|error| TelegramPairingError::StoreUnavailable {
                reason: format!("stored telegram binding actor invalid: {error}"),
            })?;
            self.conversation_actor_pairings
                .unpair_external_actor_if_owned_by(
                    &self.tenant_id,
                    &adapter_kind,
                    &installation_id,
                    &actor_ref,
                    &ExpectedExternalActorOwner {
                        user_id: caller.clone(),
                        binding_epoch: removed_binding
                            .epoch
                            .clone()
                            .map(ExternalActorBindingEpoch::new)
                            .transpose()
                            .map_err(|error| TelegramPairingError::StoreUnavailable {
                                reason: format!("stored telegram binding epoch invalid: {error}"),
                            })?,
                    },
                )
                .await
                .map_err(|error| TelegramPairingError::StoreUnavailable {
                    reason: error.to_string(),
                })?;
        }
        let mut installations: BTreeSet<String> = removed
            .iter()
            .filter_map(|binding| binding.provider_user_id.split_once(':'))
            .map(|(installation, _)| installation.to_string())
            .collect();
        if let Some(setup) = self.setup.current_setup().await? {
            installations.insert(
                setup
                    .installation_id()
                    .map_err(TelegramPairingError::from)?
                    .as_str()
                    .to_string(),
            );
        }
        for installation in installations {
            let installation_id = AdapterInstallationId::new(installation).map_err(|error| {
                TelegramPairingError::StoreUnavailable {
                    reason: format!("stored telegram binding installation invalid: {error}"),
                }
            })?;
            self.state
                .delete_dm_target_for_user(&installation_id, caller)
                .await?;
        }
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

/// The extension lifecycle's narrow connection-status probe. Composition
/// connects the pairing service to Telegram's declared account-setup entry so
/// activation can gate on the caller's pairing state without holding the full
/// pairing surface.
#[async_trait]
impl ironclaw_product_workflow::AccountConnectionStatusSource for TelegramPairingService {
    async fn connected(
        &self,
        user_id: &UserId,
    ) -> Result<bool, ironclaw_product_workflow::AccountConnectionStatusError> {
        let status = self.status_for(user_id).await.map_err(|error| {
            ironclaw_product_workflow::AccountConnectionStatusError::new(error.to_string())
        })?;
        Ok(status.connected)
    }
}

/// Recording fake for the conversation-actor pairing port, shared by this
/// module's and `telegram_dispatch`'s tests.
#[cfg(test)]
pub(crate) mod pairing_test_support {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_conversations::{
        AdapterInstallationId, AdapterKind, ConditionalUnpairOutcome,
        ConversationActorPairingService, ExpectedExternalActorOwner, ExternalActorBindingEpoch,
        ExternalActorRef, InboundTurnError,
    };
    use ironclaw_host_api::{TenantId, UserId};

    #[derive(Default)]
    pub(crate) struct RecordingActorPairings {
        pub(crate) conditional_unpairs: Mutex<Vec<(String, String, String)>>,
    }

    impl RecordingActorPairings {
        pub(crate) fn shared() -> Arc<Self> {
            Arc::new(Self::default())
        }
    }

    #[async_trait]
    impl ConversationActorPairingService for RecordingActorPairings {
        async fn pair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
        ) -> Result<(), InboundTurnError> {
            Ok(())
        }

        async fn pair_external_actor_with_epoch(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
            _binding_epoch: ExternalActorBindingEpoch,
        ) -> Result<(), InboundTurnError> {
            Ok(())
        }

        async fn unpair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
        ) -> Result<(), InboundTurnError> {
            Ok(())
        }

        async fn unpair_external_actor_if_owned_by(
            &self,
            _tenant_id: &TenantId,
            _adapter_kind: &AdapterKind,
            adapter_installation_id: &AdapterInstallationId,
            external_actor_ref: &ExternalActorRef,
            expected: &ExpectedExternalActorOwner,
        ) -> Result<ConditionalUnpairOutcome, InboundTurnError> {
            self.conditional_unpairs
                .lock()
                .expect("recording lock")
                .push((
                    adapter_installation_id.as_str().to_string(),
                    external_actor_ref.id().to_string(),
                    expected.user_id.as_str().to_string(),
                ));
            Ok(ConditionalUnpairOutcome::Unpaired)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use ironclaw_auth::AuthProductError;
    use ironclaw_secrets::InMemorySecretStore;
    use secrecy::SecretString;

    use super::*;
    use crate::state::FilesystemTelegramHostState;
    use crate::telegram_bot_api::{TelegramBotApi, TelegramBotApiError, TelegramBotIdentity};
    use crate::telegram_setup::{TelegramInstallationSetupUpdate, TelegramSetupService};
    use crate::test_support::{fault_injected_telegram_state, telegram_state};

    #[derive(Debug, Default)]
    struct RecordingDispatcher {
        events: StdMutex<Vec<AuthContinuationEvent>>,
        fail_remaining: std::sync::atomic::AtomicUsize,
    }

    impl RecordingDispatcher {
        fn failing_once() -> Self {
            Self {
                events: StdMutex::new(Vec::new()),
                fail_remaining: std::sync::atomic::AtomicUsize::new(1),
            }
        }
    }

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RecordingDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            if self
                .fail_remaining
                .fetch_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |remaining| remaining.checked_sub(1),
                )
                .is_ok()
            {
                return Err(AuthProductError::BackendUnavailable);
            }
            self.events.lock().expect("lock").push(event);
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
        state: Arc<FilesystemTelegramHostState>,
        setup: Arc<TelegramSetupService>,
        actor_pairings: Arc<super::pairing_test_support::RecordingActorPairings>,
    }

    async fn fixture(configured: bool) -> Fixture {
        fixture_with_state(
            configured,
            telegram_state(),
            Arc::new(RecordingDispatcher::default()),
        )
        .await
    }

    async fn fixture_with_state(
        configured: bool,
        state: Arc<FilesystemTelegramHostState>,
        dispatcher: Arc<RecordingDispatcher>,
    ) -> Fixture {
        let tenant_id = TenantId::new("tenant-a").expect("tenant");
        let agent_id = AgentId::new("agent-a").expect("agent");
        let setup = Arc::new(TelegramSetupService::new(
            tenant_id.clone(),
            agent_id.clone(),
            None,
            UserId::new("operator").expect("user"),
            Arc::clone(&state),
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
        let actor_pairings = super::pairing_test_support::RecordingActorPairings::shared();
        let service = TelegramPairingService::new(
            tenant_id,
            agent_id,
            None,
            Arc::clone(&setup),
            Arc::clone(&state),
            Arc::clone(&dispatcher) as Arc<dyn RebornAuthContinuationDispatcher>,
            Arc::clone(&actor_pairings)
                as Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
        );
        Fixture {
            service,
            dispatcher,
            state,
            setup,
            actor_pairings,
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

    /// Two concurrent consumers of the same live code, from different
    /// Telegram accounts, both read the record before either claims it (the
    /// barrier pins that interleaving). Exactly one may bind: the claim is
    /// single-consumer and happens before any identity/target side effect.
    #[tokio::test]
    async fn concurrent_consume_of_one_code_binds_exactly_one_winner() {
        let (state, filesystem) = fault_injected_telegram_state();
        let fixture =
            fixture_with_state(true, state, Arc::new(RecordingDispatcher::default())).await;
        let ben = user("ben");
        let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        filesystem.hold_next_reads_at(2, Arc::new(tokio::sync::Barrier::new(2)));

        let (first, second) = tokio::join!(
            fixture.service.consume(&issue.code, "tg-attacker", 111),
            fixture.service.consume(&issue.code, "tg-victim", 222),
        );
        let outcomes = [first.expect("consume"), second.expect("consume")];

        let paired = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PairingConsumeOutcome::Paired { .. }))
            .count();
        let refused = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, PairingConsumeOutcome::ExpiredOrUnknown))
            .count();
        assert_eq!(paired, 1, "exactly one concurrent consumer may pair");
        assert_eq!(refused, 1, "the claim loser is refused");
        let installation_id = fixture
            .setup
            .current_setup()
            .await
            .expect("setup read")
            .expect("configured")
            .installation_id()
            .expect("installation id");
        let mut bound_count = 0;
        for telegram_user_id in ["tg-attacker", "tg-victim"] {
            let provider_user_id =
                telegram_user_identity_provider_user_id(&installation_id, telegram_user_id);
            if fixture
                .state
                .bound_user_for(&provider_user_id)
                .await
                .expect("binding read")
                .is_some()
            {
                bound_count += 1;
            }
        }
        assert_eq!(bound_count, 1, "the loser must not leave a binding behind");
        assert_eq!(
            fixture.dispatcher.events.lock().expect("lock").len(),
            1,
            "exactly one continuation dispatch"
        );
    }

    /// A continuation dispatch that fails after the code was claimed must not
    /// strand the blocked run: re-sending the (already consumed) code from the
    /// now-bound account repairs completion — DM target upserted and the
    /// continuation dispatched.
    #[tokio::test]
    async fn resend_after_failed_continuation_dispatch_repairs_completion() {
        let fixture = fixture_with_state(
            true,
            telegram_state(),
            Arc::new(RecordingDispatcher::failing_once()),
        )
        .await;
        let ben = user("ben");
        let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");

        let error = fixture
            .service
            .consume(&issue.code, "tg-100", 555)
            .await
            .expect_err("first consume surfaces the dispatch failure");
        assert!(matches!(
            error,
            TelegramPairingError::ContinuationDispatch { .. }
        ));
        assert!(
            fixture.dispatcher.events.lock().expect("lock").is_empty(),
            "failed dispatch recorded no continuation"
        );

        let outcome = fixture
            .service
            .consume(&issue.code, "tg-100", 555)
            .await
            .expect("resend repairs");
        assert_eq!(
            outcome,
            PairingConsumeOutcome::AlreadyPairedSameUser {
                user_id: ben.clone()
            }
        );
        let events = fixture.dispatcher.events.lock().expect("lock").clone();
        assert_eq!(events.len(), 1, "repair re-dispatches the continuation");
        assert_eq!(events[0].scope.resource.user_id, ben);
        let status = fixture.service.status_for(&ben).await.expect("status");
        assert!(status.connected, "DM target present after repair");
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
        let unpairs = fixture
            .actor_pairings
            .conditional_unpairs
            .lock()
            .expect("recording lock")
            .clone();
        assert_eq!(
            unpairs.len(),
            1,
            "unpair clears the conversation-actor pairing (Slack disconnect parity) — \
             leaving it re-attaches a re-paired chat to its old thread"
        );
        assert!(
            unpairs[0].0.starts_with("tg-bot-"),
            "cleanup targets the stored installation: {unpairs:?}"
        );
        drop(fixture.setup);
    }

    /// Unpair must not depend on the current bot setup: after an admin clears
    /// the deployment, a user's disconnect still removes their durable
    /// binding — reconfiguring the same bot must not silently resurrect the
    /// connection they explicitly severed.
    #[tokio::test]
    async fn unpair_after_admin_cleared_setup_still_removes_the_binding() {
        let fixture = fixture(true).await;
        let ben = user("ben");
        let issue = fixture.service.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .service
            .consume(&issue.code, "tg-100", 1)
            .await
            .expect("pair");

        fixture.setup.clear().await.expect("admin clears setup");
        fixture.service.unpair(&ben).await.expect("unpair");
        let provider_user_id = telegram_user_identity_provider_user_id(
            &AdapterInstallationId::new("tg-bot-777").expect("installation"),
            "tg-100",
        );
        assert!(
            fixture
                .state
                .bound_user_for(&provider_user_id)
                .await
                .expect("binding read")
                .is_none(),
            "unpair without a current setup must still remove the binding"
        );

        // Reconfigure the same bot: the disconnected user must NOT come back
        // paired, and their old Telegram account is unbound.
        fixture
            .setup
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("123:abc".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("same bot reconfigures");
        let status = fixture.service.status_for(&ben).await.expect("status");
        assert!(
            !status.connected,
            "clear-setup → unpair → reconfigure must not resurrect the pairing"
        );
    }
}
