use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
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

use crate::setup::TelegramSetupService;
use crate::state::FilesystemTelegramHostState;
use crate::telegram_actor_identity::{
    TELEGRAM_IDENTITY_PROVIDER, telegram_user_identity_provider_user_id,
};
use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;

use super::code::{mint_pairing_code, pairing_issue};
use super::{
    PAIRING_TTL_MINUTES, PairingCode, PairingConsumeOutcome, PairingIssue, TelegramBindingError,
    TelegramDmTarget, TelegramPairingError, TelegramPairingRecord, TelegramPairingStatus,
};

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
                if let Some(chat_id) = self
                    .state
                    .pending_pairing_completion_chat(&installation_id, caller)
                    .await?
                {
                    self.finish_pending_pairing_completion(&installation_id, caller, chat_id)
                        .await?;
                }
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
                .filter(|record| {
                    record.is_live(Utc::now())
                        && setup
                            .installation_id()
                            .is_ok_and(|current| current == record.installation_id)
                })
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
        authenticated_installation_id: &AdapterInstallationId,
        raw_code: &str,
        telegram_user_id: &str,
        chat_id: i64,
    ) -> Result<PairingConsumeOutcome, TelegramPairingError> {
        let Ok(code) = PairingCode::parse(raw_code) else {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        };
        let Some(record) = self.state.pairing_for_code(&code).await? else {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        };
        if &record.installation_id != authenticated_installation_id {
            return Ok(PairingConsumeOutcome::ExpiredOrUnknown);
        }
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
        // Persist the continuation work before exposing the DM target as
        // connected. Status polling retries this outbox entry after a failed
        // dispatch or process restart; the user never needs to resend a
        // consumed code to unstrand the blocked run.
        self.state
            .persist_pairing_completion(&record.installation_id, &record.user_id, chat_id)
            .await?;
        self.finish_pending_pairing_completion(&record.installation_id, &record.user_id, chat_id)
            .await
    }

    async fn finish_pending_pairing_completion(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
        chat_id: i64,
    ) -> Result<(), TelegramPairingError> {
        self.dispatch_pairing_completion(user_id).await?;
        self.state
            .upsert_dm_target(
                installation_id,
                TelegramDmTarget {
                    user_id: user_id.clone(),
                    chat_id,
                },
            )
            .await?;
        self.state
            .finish_pairing_completion(installation_id, user_id, chat_id)
            .await
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
        let mut installations = BTreeSet::new();
        let mut actor_cleanups = Vec::new();
        for removed_binding in &removed {
            let (installation, telegram_user_id) = removed_binding
                .provider_user_id
                .split_once(':')
                .ok_or_else(|| TelegramPairingError::StoreUnavailable {
                    reason: "stored telegram binding identity is malformed".to_string(),
                })?;
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
            let binding_epoch = removed_binding
                .epoch
                .clone()
                .map(ExternalActorBindingEpoch::new)
                .transpose()
                .map_err(|error| TelegramPairingError::StoreUnavailable {
                    reason: format!("stored telegram binding epoch invalid: {error}"),
                })?;
            installations.insert(installation.to_string());
            actor_cleanups.push((installation_id, actor_ref, binding_epoch));
        }
        if let Some(setup) = self.setup.current_setup().await? {
            installations.insert(
                setup
                    .installation_id()
                    .map_err(TelegramPairingError::from)?
                    .as_str()
                    .to_string(),
            );
        }
        // Remove delivery authority before cleanup that can fail. Bindings are
        // already inactive, and deleting every derived DM target makes the
        // user observably disconnected while durable actor-cleanup metadata is
        // retained for retry.
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
        for (installation_id, actor_ref, binding_epoch) in actor_cleanups {
            self.conversation_actor_pairings
                .unpair_external_actor_if_owned_by(
                    &self.tenant_id,
                    &adapter_kind,
                    &installation_id,
                    &actor_ref,
                    &ExpectedExternalActorOwner {
                        user_id: caller.clone(),
                        binding_epoch,
                    },
                )
                .await
                .map_err(|error| TelegramPairingError::StoreUnavailable {
                    reason: error.to_string(),
                })?;
        }
        self.state
            .finalize_unbound_telegram_users_for_user(
                caller,
                &removed
                    .iter()
                    .map(|binding| binding.provider_user_id.clone())
                    .collect::<Vec<_>>(),
            )
            .await
            .map_err(|error| TelegramPairingError::StoreUnavailable {
                reason: error.to_string(),
            })?;
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
            tracing::debug!(
                target: "ironclaw::reborn::telegram",
                error = %error,
                "telegram pairing status lookup failed"
            );
            ironclaw_product_workflow::AccountConnectionStatusError::new(
                "telegram pairing status unavailable",
            )
        })?;
        Ok(status.connected)
    }
}
