//! Pairing-aware pre-router between verified Telegram ingress and the
//! ProductAdapter runner.
//!
//! Every update reaching this router has already passed the shared-secret
//! header verification in `telegram_serve`. The router decides, per update:
//!
//! - non-actionable updates (no message, non-private chat, no sender, bot
//!   sender, unparseable body) are acked silently — no reply, no forward;
//! - pairing attempts (`/start <CODE>` or a bare code-shaped message) are
//!   consumed against [`TelegramPairingService`] and answered with a static
//!   reply;
//! - bare `/start` and any message from an unpaired sender get a throttled
//!   static onboarding hint (at most one per chat per window);
//! - ordinary messages from paired senders are forwarded to the wrapped
//!   [`crate::telegram_serve::TelegramUpdatesWebhookDispatcher`]
//!   runner so the normal ProductWorkflow path runs.
//!
//! Static replies are best-effort: send failures are logged at debug and
//! swallowed — the webhook must still ack (honest delivery applies to turn
//! replies, not static hints).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use ironclaw_product_adapters::redaction::RedactedString;
use ironclaw_product_adapters::{
    AdapterInstallationId, ProductAdapterError, ProductInboundAck, ProtocolAuthEvidence,
};
use ironclaw_wasm_product_adapters::{
    ImmediateAckWorkflowObserver, RunnerError, WebhookProcessOutcome,
};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::telegram_actor_identity::{
    TELEGRAM_IDENTITY_PROVIDER, telegram_user_identity_provider_user_id,
};
use crate::telegram_bot_api::TelegramBotApi;
use crate::telegram_pairing::{
    PAIRING_CODE_ALPHABET, PAIRING_CODE_LEN, PairingConsumeOutcome, TelegramPairingService,
};
use crate::telegram_serve::TelegramUpdatesWebhookDispatcher;
use crate::telegram_setup::TelegramSetupService;
use ironclaw_channel_host::identity::RebornUserIdentityLookup;

const TRACING_TARGET: &str = "ironclaw::reborn::telegram_updates";
const PRIVATE_CHAT_KIND: &str = "private";
const START_COMMAND: &str = "/start";
const HINT_THROTTLE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Shorter than the hint window: a legitimate user who mistyped gets fresh
/// feedback quickly, while repeated invalid guesses in one chat stop earning
/// replies (the consume itself is never gated — see `handle_pairing_attempt`).
const INVALID_CODE_REPLY_THROTTLE_WINDOW: Duration = Duration::from_secs(30);

const PAIRED_REPLY: &str = "✅ Paired. You can talk to IronClaw right here.";
const ALREADY_PAIRED_SAME_USER_REPLY: &str = "You're already paired — just send me a message.";
const ALREADY_BOUND_TO_OTHER_USER_REPLY: &str =
    "This Telegram account is already paired to another IronClaw user.";
const EXPIRED_OR_UNKNOWN_REPLY: &str = "That code has expired or was already used — get a fresh one from IronClaw → Extensions → Telegram and send it here (or /start <code>).";
const UNPAIRED_HINT_REPLY: &str = "This bot is IronClaw. Pair your account from IronClaw → Extensions → Telegram, then message me here. Already have a pairing code? Just send it in this chat (or /start <code>).";

const IDENTITY_LOOKUP_UNAVAILABLE_REASON: &str = "telegram identity lookup unavailable";

/// Pairing-aware admission router for one resolved Telegram installation.
/// Sits between the verified ingress route and the wrapped native runner.
pub struct TelegramInboundPreRouter {
    pairing: Arc<TelegramPairingService>,
    lookup: Arc<dyn RebornUserIdentityLookup>,
    bot_api: Arc<dyn TelegramBotApi>,
    setup: Arc<TelegramSetupService>,
    installation_id: AdapterInstallationId,
    hint_throttle: Mutex<HashMap<i64, Instant>>,
    hint_throttle_window: Duration,
    invalid_code_reply_throttle: Mutex<HashMap<i64, Instant>>,
    invalid_code_reply_throttle_window: Duration,
    runner: Arc<dyn TelegramUpdatesWebhookDispatcher>,
}

impl TelegramInboundPreRouter {
    pub fn new(
        pairing: Arc<TelegramPairingService>,
        lookup: Arc<dyn RebornUserIdentityLookup>,
        bot_api: Arc<dyn TelegramBotApi>,
        setup: Arc<TelegramSetupService>,
        installation_id: AdapterInstallationId,
        runner: Arc<dyn TelegramUpdatesWebhookDispatcher>,
    ) -> Self {
        Self {
            pairing,
            lookup,
            bot_api,
            setup,
            installation_id,
            hint_throttle: Mutex::new(HashMap::new()),
            hint_throttle_window: HINT_THROTTLE_WINDOW,
            invalid_code_reply_throttle: Mutex::new(HashMap::new()),
            invalid_code_reply_throttle_window: INVALID_CODE_REPLY_THROTTLE_WINDOW,
            runner,
        }
    }

    #[cfg(test)]
    fn with_hint_throttle_window(mut self, window: Duration) -> Self {
        self.hint_throttle_window = window;
        self
    }

    async fn process_update(
        &self,
        body: &[u8],
        evidence: &ProtocolAuthEvidence,
        observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        let update: TelegramUpdateLite = match serde_json::from_slice(body) {
            Ok(update) => update,
            Err(_) => {
                // Verified but unusable: ack so Telegram does not redeliver a
                // permanently-unparseable update. Never log the body.
                tracing::debug!(
                    target = TRACING_TARGET,
                    "verified telegram update body did not parse; acked without action"
                );
                return Ok(handled_silently_ack());
            }
        };
        let Some(message) = update.message else {
            tracing::debug!(
                target = TRACING_TARGET,
                update_id = update.update_id,
                "telegram update without message; acked without action"
            );
            return Ok(handled_silently_ack());
        };
        if message.chat.kind != PRIVATE_CHAT_KIND {
            return Ok(handled_silently_ack());
        }
        let Some(from) = &message.from else {
            return Ok(handled_silently_ack());
        };
        if from.is_bot {
            return Ok(handled_silently_ack());
        }
        let chat_id = message.chat.id;

        match classify_inbound_text(message.text.as_deref()) {
            InboundTextClass::PairingAttempt(code) => {
                self.handle_pairing_attempt(&code, from.id, chat_id).await;
                Ok(handled_silently_ack())
            }
            InboundTextClass::StartWithoutPayload => {
                // qa-telegram:C6 — a paired sender's bare /start is a static
                // no-op (re-opening the chat must not pitch pairing to an
                // already-paired account); only unpaired senders get the
                // throttled onboarding hint. A pairedness-lookup outage acks
                // silently: neither misleading copy nor a redelivery loop is
                // worth a greeting.
                let provider_user_id = telegram_user_identity_provider_user_id(
                    &self.installation_id,
                    &from.id.to_string(),
                );
                match self
                    .lookup
                    .resolve_user_identity(TELEGRAM_IDENTITY_PROVIDER, &provider_user_id)
                    .await
                {
                    Ok(Some(_)) => {}
                    Ok(None) => self.send_throttled_hint(chat_id).await,
                    Err(error) => {
                        // silent-ok: /start is a greeting — during an
                        // identity-store outage neither the misleading
                        // pairing hint nor a redelivery loop is worth it;
                        // the outage is logged and the update is acked.
                        tracing::debug!(
                            target = TRACING_TARGET,
                            reason = %error,
                            "pairedness lookup failed on /start; acked without a reply"
                        );
                    }
                }
                Ok(handled_silently_ack())
            }
            InboundTextClass::Ordinary => {
                let provider_user_id = telegram_user_identity_provider_user_id(
                    &self.installation_id,
                    &from.id.to_string(),
                );
                match self
                    .lookup
                    .resolve_user_identity(TELEGRAM_IDENTITY_PROVIDER, &provider_user_id)
                    .await
                {
                    Ok(Some(_)) => {
                        self.runner
                            .process_verified_update(body, evidence, observer)
                            .await
                    }
                    Ok(None) => {
                        self.send_throttled_hint(chat_id).await;
                        Ok(handled_silently_ack())
                    }
                    Err(error) => {
                        // Transient identity-store outage: surface a
                        // retryable error (503) so Telegram redelivers once
                        // the store is back instead of silently dropping a
                        // possibly-paired sender's message.
                        tracing::debug!(
                            target = TRACING_TARGET,
                            reason = %error,
                            "telegram identity lookup failed; asking transport to retry"
                        );
                        Err(RunnerError::Adapter(
                            ProductAdapterError::WorkflowTransient {
                                reason: RedactedString::new(IDENTITY_LOOKUP_UNAVAILABLE_REASON),
                            },
                        ))
                    }
                }
            }
        }
    }

    async fn handle_pairing_attempt(&self, code: &str, telegram_user_id: i64, chat_id: i64) {
        let reply = match self
            .pairing
            .consume(code, &telegram_user_id.to_string(), chat_id)
            .await
        {
            Ok(PairingConsumeOutcome::Paired { .. }) => PAIRED_REPLY,
            Ok(PairingConsumeOutcome::AlreadyPairedSameUser { .. }) => {
                ALREADY_PAIRED_SAME_USER_REPLY
            }
            Ok(PairingConsumeOutcome::AlreadyBoundToOtherUser) => ALREADY_BOUND_TO_OTHER_USER_REPLY,
            Ok(PairingConsumeOutcome::ExpiredOrUnknown) => {
                // Failed attempts are rate-limited per chat (contract §pairing):
                // the guess was still checked above — only the REPLY is
                // suppressed, so a valid code (e.g. the deep-link retry right
                // after a typo) always consumes and always confirms.
                if !throttle_allows(
                    &self.invalid_code_reply_throttle,
                    self.invalid_code_reply_throttle_window,
                    chat_id,
                )
                .await
                {
                    return;
                }
                EXPIRED_OR_UNKNOWN_REPLY
            }
            Err(error) => {
                // Infra fault (store/continuation): ack without a reply — no
                // copy fits, and a code re-send retries cleanly.
                tracing::debug!(
                    target = TRACING_TARGET,
                    reason = %error,
                    "telegram pairing consume failed; update acked without reply"
                );
                return;
            }
        };
        self.send_static_reply(chat_id, reply).await;
    }

    async fn send_throttled_hint(&self, chat_id: i64) {
        if !self.hint_allowed(chat_id).await {
            return;
        }
        self.send_static_reply(chat_id, UNPAIRED_HINT_REPLY).await;
    }

    /// At most one hint per chat per window. Entries older than the window
    /// are pruned on every check so the map stays bounded by active chats.
    async fn hint_allowed(&self, chat_id: i64) -> bool {
        throttle_allows(&self.hint_throttle, self.hint_throttle_window, chat_id).await
    }

    async fn send_static_reply(&self, chat_id: i64, text: &str) {
        let bot_token = match self.setup.bot_token().await {
            Ok(Some(token)) => token,
            Ok(None) => {
                tracing::debug!(
                    target = TRACING_TARGET,
                    "telegram bot token not configured; static reply skipped"
                );
                return;
            }
            Err(error) => {
                tracing::debug!(
                    target = TRACING_TARGET,
                    reason = %error,
                    "telegram bot token unavailable; static reply skipped"
                );
                return;
            }
        };
        if let Err(error) = self.bot_api.send_message(&bot_token, chat_id, text).await {
            tracing::debug!(
                target = TRACING_TARGET,
                reason = %error,
                "telegram static reply send failed; webhook still acked"
            );
        }
    }
}

impl std::fmt::Debug for TelegramInboundPreRouter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramInboundPreRouter")
            .field("installation_id", &self.installation_id)
            .finish_non_exhaustive()
    }
}

impl TelegramUpdatesWebhookDispatcher for TelegramInboundPreRouter {
    fn verify_webhook_auth(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ProtocolAuthEvidence, RunnerError> {
        self.runner.verify_webhook_auth(headers, body)
    }

    fn process_verified_update<'a>(
        &'a self,
        body: &'a [u8],
        evidence: &'a ProtocolAuthEvidence,
        observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>> {
        Box::pin(self.process_update(body, evidence, observer))
    }

    fn drain_immediate_ack_tasks<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        self.runner.drain_immediate_ack_tasks()
    }
}

/// Handled without forwarding: the webhook acks 200 and the workflow never
/// sees the update.
/// Shared per-chat reply throttle: at most one reply per chat per window.
/// Entries older than the window are pruned on every check so each map stays
/// bounded by recently active chats.
async fn throttle_allows(
    throttle: &Mutex<HashMap<i64, Instant>>,
    window: Duration,
    chat_id: i64,
) -> bool {
    let now = Instant::now();
    let mut throttle = throttle.lock().await;
    throttle.retain(|_, sent_at| now.duration_since(*sent_at) < window);
    if throttle.contains_key(&chat_id) {
        return false;
    }
    throttle.insert(chat_id, now);
    true
}

fn handled_silently_ack() -> WebhookProcessOutcome {
    WebhookProcessOutcome::Acknowledged {
        ack: ProductInboundAck::NoOp,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum InboundTextClass {
    /// `/start <CODE>` or a bare code-shaped message; the candidate goes to
    /// `TelegramPairingService::consume`, which owns format validation.
    PairingAttempt(String),
    /// Bare `/start` — the deep-link command without a payload.
    StartWithoutPayload,
    /// Anything else (including media messages without text).
    Ordinary,
}

fn classify_inbound_text(text: Option<&str>) -> InboundTextClass {
    let Some(text) = text else {
        return InboundTextClass::Ordinary;
    };
    let tokens: Vec<&str> = text.split_whitespace().collect();
    match tokens.as_slice() {
        [START_COMMAND] => InboundTextClass::StartWithoutPayload,
        [START_COMMAND, code] => InboundTextClass::PairingAttempt((*code).to_string()),
        [single] if is_bare_pairing_code(single) => {
            InboundTextClass::PairingAttempt((*single).to_string())
        }
        _ => InboundTextClass::Ordinary,
    }
}

fn is_bare_pairing_code(token: &str) -> bool {
    token.len() == PAIRING_CODE_LEN
        && token
            .to_ascii_uppercase()
            .bytes()
            .all(|byte| PAIRING_CODE_ALPHABET.contains(&byte))
}

/// Minimal update projection for admission routing. Deliberately tolerant:
/// unknown fields are ignored, and the full parse (attachments, entities,
/// group triggers) stays owned by `ironclaw_telegram_v2_adapter` on the
/// forwarded path.
#[derive(Debug, Deserialize)]
struct TelegramUpdateLite {
    #[serde(default)]
    update_id: i64,
    #[serde(default)]
    message: Option<MessageLite>,
}

#[derive(Debug, Deserialize)]
struct MessageLite {
    chat: ChatLite,
    #[serde(default)]
    from: Option<FromLite>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatLite {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct FromLite {
    id: i64,
    #[serde(default)]
    is_bot: bool,
}

/// Shared in-memory fakes for the Telegram serve/dispatch unit tests. Lives
/// here so `telegram_serve`'s route tests reuse the same fixtures instead of
/// standing up a second copy.
#[cfg(test)]
pub mod test_fixtures {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_auth::AuthContinuationEvent;
    use ironclaw_auth::AuthProductError;
    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_product_adapters::{
        ProductAdapterError, ProductInboundAck, ProductInboundEnvelope,
        ProjectionSubscriptionRequest,
    };
    use ironclaw_secrets::InMemorySecretStore;
    use secrecy::SecretString;

    use super::*;
    use crate::telegram_bot_api::{TelegramBotApiError, TelegramBotIdentity};
    use crate::telegram_pairing::{
        TelegramBindingError, TelegramDmTarget, TelegramDmTargetStore, TelegramPairingError,
        TelegramPairingRecord, TelegramPairingStore, TelegramUserBindingStore,
    };
    use crate::telegram_setup::{
        TelegramInstallationSetup, TelegramInstallationSetupStore, TelegramInstallationSetupUpdate,
        TelegramSetupError,
    };
    use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;
    use ironclaw_channel_host::identity::RebornUserIdentityLookupError;

    pub const FIXTURE_BOT_ID: i64 = 4242;
    pub const FIXTURE_BOT_USERNAME: &str = "ironclaw_qa_bot";

    /// Records `sendMessage` calls; setup-time calls (`getMe`, `setWebhook`,
    /// `deleteWebhook`) succeed with a fixed bot identity unless a test swaps
    /// it via [`RecordingBotApi::set_bot_identity`] (bot-swap scenarios).
    #[derive(Debug)]
    pub struct RecordingBotApi {
        sends: StdMutex<Vec<(i64, String)>>,
        fail_sends: AtomicBool,
        identity: StdMutex<TelegramBotIdentity>,
    }

    impl Default for RecordingBotApi {
        fn default() -> Self {
            Self {
                sends: StdMutex::new(Vec::new()),
                fail_sends: AtomicBool::new(false),
                identity: StdMutex::new(TelegramBotIdentity {
                    id: FIXTURE_BOT_ID,
                    username: FIXTURE_BOT_USERNAME.to_string(),
                }),
            }
        }
    }

    impl RecordingBotApi {
        pub fn sends(&self) -> Vec<(i64, String)> {
            self.sends.lock().expect("lock").clone()
        }

        pub fn fail_sends(&self) {
            self.fail_sends.store(true, Ordering::SeqCst);
        }

        /// Point `getMe` at a different bot so the next setup save models a
        /// bot swap (new installation id).
        pub fn set_bot_identity(&self, id: i64, username: &str) {
            *self.identity.lock().expect("lock") = TelegramBotIdentity {
                id,
                username: username.to_string(),
            };
        }
    }

    #[async_trait]
    impl TelegramBotApi for RecordingBotApi {
        async fn get_me(
            &self,
            _bot_token: &SecretString,
        ) -> Result<TelegramBotIdentity, TelegramBotApiError> {
            Ok(self.identity.lock().expect("lock").clone())
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
            chat_id: i64,
            text: &str,
        ) -> Result<(), TelegramBotApiError> {
            self.sends
                .lock()
                .expect("lock")
                .push((chat_id, text.to_string()));
            if self.fail_sends.load(Ordering::SeqCst) {
                return Err(TelegramBotApiError::Rejected {
                    kind: crate::telegram_bot_api::TelegramBotApiRejection::Forbidden,
                });
            }
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    pub struct InMemorySetupStore {
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

    #[derive(Debug, Default)]
    pub struct InMemoryPairingStore {
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

        async fn pairing_for_code(
            &self,
            code: &str,
        ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
            Ok(self
                .records
                .lock()
                .expect("lock")
                .iter()
                .find(|record| record.code.eq_ignore_ascii_case(code))
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

        async fn claim_pairing(
            &self,
            code: &str,
        ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
            let mut records = self.records.lock().expect("lock");
            let now = Utc::now();
            for record in records.iter_mut() {
                if record.code.eq_ignore_ascii_case(code) {
                    if !record.is_live(now) {
                        return Ok(None);
                    }
                    record.consumed_at = Some(now);
                    return Ok(Some(record.clone()));
                }
            }
            Ok(None)
        }

        async fn invalidate_for_user(&self, user_id: &UserId) -> Result<(), TelegramPairingError> {
            let mut records = self.records.lock().expect("lock");
            records.retain(|record| &record.user_id != user_id);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    pub struct InMemoryBindingStore {
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
            installation: Option<&AdapterInstallationId>,
        ) -> Result<Vec<crate::telegram_pairing::RemovedTelegramBinding>, TelegramBindingError>
        {
            let mut bindings = self.bindings.lock().expect("lock");
            let removed: Vec<crate::telegram_pairing::RemovedTelegramBinding> = bindings
                .iter()
                .filter(|(key, (bound, _))| {
                    bound == user_id
                        && installation.is_none_or(|installation| {
                            crate::telegram_actor_identity::provider_user_id_in_installation(
                                key,
                                installation,
                            )
                        })
                })
                .map(
                    |(key, (_, epoch))| crate::telegram_pairing::RemovedTelegramBinding {
                        provider_user_id: key.clone(),
                        epoch: Some(epoch.clone()),
                    },
                )
                .collect();
            for binding in &removed {
                bindings.remove(&binding.provider_user_id);
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
    pub struct InMemoryDmTargetStore {
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
    pub struct RecordingContinuationDispatcher {
        events: StdMutex<Vec<AuthContinuationEvent>>,
    }

    #[async_trait]
    impl RebornAuthContinuationDispatcher for RecordingContinuationDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            self.events.lock().expect("lock").push(event);
            Ok(())
        }
    }

    /// Identity lookup fake keyed by `(provider, provider_user_id)`. `fail()`
    /// switches every read into a backend error.
    #[derive(Debug, Default)]
    pub struct FakeIdentityLookup {
        bindings: StdMutex<HashMap<(String, String), UserId>>,
        fail: AtomicBool,
    }

    impl FakeIdentityLookup {
        pub fn bind(&self, provider: &str, provider_user_id: &str, user: &str) {
            self.bindings.lock().expect("lock").insert(
                (provider.to_string(), provider_user_id.to_string()),
                UserId::new(user).expect("valid user id"),
            );
        }

        pub fn fail(&self) {
            self.fail.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl RebornUserIdentityLookup for FakeIdentityLookup {
        async fn resolve_user_identity(
            &self,
            provider: &str,
            provider_user_id: &str,
        ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(RebornUserIdentityLookupError::Backend(
                    "test lookup outage".to_string(),
                ));
            }
            Ok(self
                .bindings
                .lock()
                .expect("lock")
                .get(&(provider.to_string(), provider_user_id.to_string()))
                .cloned())
        }

        async fn user_has_provider_binding(
            &self,
            provider: &str,
            user_id: &UserId,
        ) -> Result<bool, RebornUserIdentityLookupError> {
            Ok(self
                .bindings
                .lock()
                .expect("lock")
                .iter()
                .any(|((bound_provider, _), bound)| bound_provider == provider && bound == user_id))
        }
    }

    /// `ProductWorkflow` fake counting durable submissions.
    pub struct CountingWorkflow {
        submitted: Arc<AtomicUsize>,
    }

    impl CountingWorkflow {
        pub fn new(submitted: Arc<AtomicUsize>) -> Self {
            Self { submitted }
        }
    }

    #[async_trait]
    impl ironclaw_product_adapters::ProductWorkflow for CountingWorkflow {
        async fn submit_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            self.submitted.fetch_add(1, Ordering::SeqCst);
            Ok(ProductInboundAck::NoOp)
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(ProductAdapterError::Internal {
                detail: ironclaw_product_adapters::redaction::RedactedString::new(
                    "test stub: resolve_projection_subscription not supported",
                ),
            })
        }
    }

    fn tenant_id() -> TenantId {
        TenantId::new("tenant-a").expect("valid tenant")
    }

    fn agent_id() -> AgentId {
        AgentId::new("agent-a").expect("valid agent")
    }

    pub fn unconfigured_setup_service(bot_api: Arc<RecordingBotApi>) -> Arc<TelegramSetupService> {
        Arc::new(TelegramSetupService::new(
            tenant_id(),
            agent_id(),
            None,
            UserId::new("operator").expect("valid user"),
            Arc::new(InMemorySetupStore::default()),
            Arc::new(InMemorySecretStore::new()),
            bot_api,
            Some("https://ironclaw.example".to_string()),
        ))
    }

    /// A saved deployment bot (`tg-bot-4242`) with token + webhook secret in
    /// the in-memory secret store.
    pub async fn configured_setup_service(
        bot_api: Arc<RecordingBotApi>,
    ) -> Arc<TelegramSetupService> {
        let setup = unconfigured_setup_service(bot_api);
        setup
            .save_with_previous(TelegramInstallationSetupUpdate {
                bot_token: Some(SecretString::from("123:abc".to_string())),
                webhook_url_override: None,
            })
            .await
            .expect("test setup saves");
        setup
    }

    pub fn pairing_service_with(setup: Arc<TelegramSetupService>) -> Arc<TelegramPairingService> {
        Arc::new(TelegramPairingService::new(
            tenant_id(),
            agent_id(),
            None,
            setup,
            Arc::new(InMemoryPairingStore::default()),
            Arc::new(InMemoryBindingStore::default()),
            Arc::new(InMemoryDmTargetStore::default()),
            Arc::new(RecordingContinuationDispatcher::default()),
            crate::telegram_pairing::pairing_test_support::RecordingActorPairings::shared(),
        ))
    }

    pub fn fixture_installation_id() -> AdapterInstallationId {
        AdapterInstallationId::new(format!("tg-bot-{FIXTURE_BOT_ID}"))
            .expect("valid installation id")
    }

    /// A private-chat update body; `text: None` models a media-only message.
    pub fn private_text_update_body(from_id: i64, chat_id: i64, text: Option<&str>) -> Vec<u8> {
        let mut message = serde_json::json!({
            "message_id": 100,
            "date": 1,
            "chat": {"id": chat_id, "type": "private"},
            "from": {"id": from_id, "is_bot": false, "first_name": "Test"},
        });
        if let Some(text) = text {
            message["text"] = serde_json::Value::String(text.to_string());
        }
        serde_json::to_vec(&serde_json::json!({
            "update_id": 7,
            "message": message,
        }))
        .expect("test body serializes")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_product_adapters::auth::mark_shared_secret_header_verified;

    use super::test_fixtures::{
        FakeIdentityLookup, RecordingBotApi, configured_setup_service, fixture_installation_id,
        pairing_service_with, private_text_update_body,
    };
    use super::*;
    use crate::telegram_serve::TELEGRAM_SECRET_TOKEN_HEADER;

    struct FakeForwardRunner {
        calls: Arc<AtomicUsize>,
    }

    impl TelegramUpdatesWebhookDispatcher for FakeForwardRunner {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            Ok(verified_evidence())
        }

        fn process_verified_update<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
            _observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch) })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    fn verified_evidence() -> ProtocolAuthEvidence {
        mark_shared_secret_header_verified(TELEGRAM_SECRET_TOKEN_HEADER, "tg-bot-4242")
    }

    struct Fixture {
        pre_router: TelegramInboundPreRouter,
        pairing: Arc<TelegramPairingService>,
        lookup: Arc<FakeIdentityLookup>,
        bot_api: Arc<RecordingBotApi>,
        runner_calls: Arc<AtomicUsize>,
    }

    impl Fixture {
        async fn process(&self, body: &[u8]) -> Result<WebhookProcessOutcome, RunnerError> {
            let evidence = verified_evidence();
            self.pre_router
                .process_verified_update(body, &evidence, None)
                .await
        }

        fn paired_provider_user_id(&self, from_id: i64) -> String {
            telegram_user_identity_provider_user_id(
                &fixture_installation_id(),
                &from_id.to_string(),
            )
        }

        fn bind_paired(&self, from_id: i64, user: &str) {
            self.lookup.bind(
                TELEGRAM_IDENTITY_PROVIDER,
                &self.paired_provider_user_id(from_id),
                user,
            );
        }
    }

    async fn fixture() -> Fixture {
        fixture_with_window(HINT_THROTTLE_WINDOW).await
    }

    async fn fixture_with_window(window: Duration) -> Fixture {
        let bot_api = Arc::new(RecordingBotApi::default());
        let setup = configured_setup_service(Arc::clone(&bot_api)).await;
        let pairing = pairing_service_with(Arc::clone(&setup));
        let lookup = Arc::new(FakeIdentityLookup::default());
        let runner_calls = Arc::new(AtomicUsize::new(0));
        let runner = Arc::new(FakeForwardRunner {
            calls: Arc::clone(&runner_calls),
        });
        let pre_router = TelegramInboundPreRouter::new(
            Arc::clone(&pairing),
            lookup.clone() as Arc<dyn RebornUserIdentityLookup>,
            bot_api.clone() as Arc<dyn TelegramBotApi>,
            setup,
            fixture_installation_id(),
            runner,
        )
        .with_hint_throttle_window(window);
        Fixture {
            pre_router,
            pairing,
            lookup,
            bot_api,
            runner_calls,
        }
    }

    fn user(name: &str) -> ironclaw_host_api::UserId {
        ironclaw_host_api::UserId::new(name).expect("valid user")
    }

    fn assert_silently_handled(outcome: &WebhookProcessOutcome) {
        assert!(
            matches!(
                outcome,
                WebhookProcessOutcome::Acknowledged {
                    ack: ProductInboundAck::NoOp
                }
            ),
            "expected silent ack, got: {outcome:?}"
        );
    }

    // Row (a): non-actionable updates are acked with no reply and no forward.
    #[tokio::test]
    async fn non_actionable_updates_are_acked_silently() {
        let fixture = fixture().await;
        let group_body = serde_json::to_vec(&serde_json::json!({
            "update_id": 2,
            "message": {
                "chat": {"id": -100, "type": "supergroup"},
                "from": {"id": 42, "is_bot": false},
                "text": "hello"
            }
        }))
        .expect("body");
        let bot_sender_body = serde_json::to_vec(&serde_json::json!({
            "update_id": 3,
            "message": {
                "chat": {"id": 555, "type": "private"},
                "from": {"id": 42, "is_bot": true},
                "text": "hello"
            }
        }))
        .expect("body");
        let no_from_body = serde_json::to_vec(&serde_json::json!({
            "update_id": 4,
            "message": {"chat": {"id": 555, "type": "private"}, "text": "hello"}
        }))
        .expect("body");
        let cases: Vec<Vec<u8>> = vec![
            br#"{"update_id":1}"#.to_vec(),
            group_body,
            bot_sender_body,
            no_from_body,
            b"not json at all".to_vec(),
        ];

        for body in cases {
            let outcome = fixture.process(&body).await.expect("acked");
            assert_silently_handled(&outcome);
        }
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.bot_api.sends().is_empty());
    }

    // Row (b): `/start <CODE>` with a live code pairs and replies.
    #[tokio::test]
    async fn start_with_live_code_pairs_and_replies() {
        let fixture = fixture().await;
        let ben = user("ben");
        let issue = fixture
            .pairing
            .issue_or_rotate(&ben)
            .await
            .expect("code issues");

        let body = private_text_update_body(42, 555, Some(&format!("/start {}", issue.code)));
        let outcome = fixture.process(&body).await.expect("acked");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture.bot_api.sends(),
            vec![(555, PAIRED_REPLY.to_string())]
        );
        let status = fixture
            .pairing
            .status_for(&ben)
            .await
            .expect("status reads");
        assert!(status.connected, "consume must actually bind the account");
    }

    // Row (b): a bare code (any case) is a pairing attempt too.
    #[tokio::test]
    async fn bare_lowercase_code_pairs() {
        let fixture = fixture().await;
        let ben = user("ben");
        let issue = fixture
            .pairing
            .issue_or_rotate(&ben)
            .await
            .expect("code issues");

        let body = private_text_update_body(42, 555, Some(&issue.code.to_ascii_lowercase()));
        fixture.process(&body).await.expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![(555, PAIRED_REPLY.to_string())]
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
    }

    // Row (b): every consume outcome maps to its static reply.
    #[tokio::test]
    async fn consume_outcome_replies_are_mapped() {
        let fixture = fixture().await;
        let ben = user("ben");
        let illia = user("illia");

        // Pair ben's telegram account 42.
        let first = fixture.pairing.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", first.code)),
            ))
            .await
            .expect("acked");

        // Same user re-pairs the same account with a fresh code.
        let second = fixture.pairing.issue_or_rotate(&ben).await.expect("issue");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", second.code)),
            ))
            .await
            .expect("acked");

        // Another user's code against the already-bound telegram account.
        let illia_issue = fixture
            .pairing
            .issue_or_rotate(&illia)
            .await
            .expect("issue");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", illia_issue.code)),
            ))
            .await
            .expect("acked");

        // Unknown / malformed code payload.
        fixture
            .process(&private_text_update_body(42, 555, Some("/start NOPE1234")))
            .await
            .expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, PAIRED_REPLY.to_string()),
                (555, ALREADY_PAIRED_SAME_USER_REPLY.to_string()),
                (555, ALREADY_BOUND_TO_OTHER_USER_REPLY.to_string()),
                (555, EXPIRED_OR_UNKNOWN_REPLY.to_string()),
            ]
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
    }

    /// Invalid pairing guesses are uniformly answered but the failure reply
    /// is rate-limited per chat, and the limiter never gates a valid consume
    /// — a typo followed by the real code (the deep-link retry shape) still
    /// pairs immediately. Automates the reply-throttle half of
    /// qa-telegram:P13; guess feasibility itself is bounded by the code
    /// space, TTL, and the per-installation ingress limiter.
    #[tokio::test]
    async fn invalid_code_replies_are_throttled_per_chat_without_gating_valid_consume() {
        let fixture = fixture().await;
        let ben = user("ben");

        // Two invalid code-shaped guesses in the same chat: one reply only.
        for _ in 0..2 {
            fixture
                .process(&private_text_update_body(42, 555, Some("AAAAAAAA")))
                .await
                .expect("acked");
        }
        // A different chat gets its own (single) failure reply.
        fixture
            .process(&private_text_update_body(43, 777, Some("AAAAAAAA")))
            .await
            .expect("acked");
        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, EXPIRED_OR_UNKNOWN_REPLY.to_string()),
                (777, EXPIRED_OR_UNKNOWN_REPLY.to_string()),
            ],
            "the second same-chat invalid guess must be answered with silence"
        );

        // The throttle must not gate valid consumption: the real code sent
        // from the throttled chat still pairs and still confirms.
        let issue = fixture
            .pairing
            .issue_or_rotate(&ben)
            .await
            .expect("code issues");
        fixture
            .process(&private_text_update_body(
                42,
                555,
                Some(&format!("/start {}", issue.code)),
            ))
            .await
            .expect("acked");
        let status = fixture.pairing.status_for(&ben).await.expect("status");
        assert!(status.connected, "valid consume must bypass the throttle");
        assert_eq!(
            fixture.bot_api.sends().last(),
            Some(&(555, PAIRED_REPLY.to_string())),
            "the success confirmation is never throttled"
        );
    }

    /// qa-telegram:C6 — a PAIRED sender's bare `/start` is a static no-op:
    /// no turn, no reply (re-opening the chat must not pitch pairing to an
    /// already-paired account). Unpaired `/start` keeps the throttled hint
    /// (pinned above).
    #[tokio::test]
    async fn paired_start_without_payload_is_a_silent_no_op() {
        let fixture = fixture().await;
        fixture.bind_paired(42, "ben");

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0, "no turn");
        assert_eq!(
            fixture.bot_api.sends(),
            Vec::<(i64, String)>::new(),
            "no reply of any kind to a paired /start"
        );
    }

    /// A pairedness-lookup outage on `/start` acks silently rather than
    /// pitching the pairing hint to a possibly-paired sender or asking
    /// Telegram to redeliver a greeting.
    #[tokio::test]
    async fn start_without_payload_acks_silently_when_lookup_is_down() {
        let fixture = fixture().await;
        fixture.lookup.fail();

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.bot_api.sends(), Vec::<(i64, String)>::new());
    }

    // Row (c): bare `/start` and unpaired ordinary text hint, throttled per
    // chat within the window.
    #[tokio::test]
    async fn unpaired_hints_are_throttled_per_chat() {
        let fixture = fixture().await;

        fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");
        // Second trigger inside the window (ordinary unpaired text this time)
        // must be suppressed for the same chat.
        fixture
            .process(&private_text_update_body(42, 555, Some("hello there")))
            .await
            .expect("acked");
        // A different chat gets its own hint.
        fixture
            .process(&private_text_update_body(43, 777, Some("/start")))
            .await
            .expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, UNPAIRED_HINT_REPLY.to_string()),
                (777, UNPAIRED_HINT_REPLY.to_string()),
            ]
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
    }

    // Throttle pruning: entries older than the window are dropped, so the
    // next hint is allowed again.
    #[tokio::test]
    async fn hint_throttle_prunes_entries_older_than_window() {
        let fixture = fixture_with_window(Duration::ZERO).await;

        fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");
        fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked");

        assert_eq!(
            fixture.bot_api.sends(),
            vec![
                (555, UNPAIRED_HINT_REPLY.to_string()),
                (555, UNPAIRED_HINT_REPLY.to_string()),
            ],
            "a zero window prunes the previous entry, so both hints send"
        );
    }

    // Row (d): a paired sender's ordinary text forwards to the runner
    // exactly once, with no static reply.
    #[tokio::test]
    async fn paired_ordinary_text_forwards_to_runner_exactly_once() {
        let fixture = fixture().await;
        fixture.bind_paired(42, "ben");

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("hello ironclaw")))
            .await
            .expect("forwarded");

        assert!(matches!(
            outcome,
            WebhookProcessOutcome::AcceptedForAsyncDispatch
        ));
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 1);
        assert!(fixture.bot_api.sends().is_empty());
    }

    // Row (d)/(c): media messages without text follow the pairing split.
    #[tokio::test]
    async fn textless_message_follows_pairing_split() {
        let fixture = fixture().await;
        fixture.bind_paired(42, "ben");

        fixture
            .process(&private_text_update_body(42, 555, None))
            .await
            .expect("forwarded");
        assert_eq!(
            fixture.runner_calls.load(Ordering::SeqCst),
            1,
            "paired media-only message forwards"
        );

        fixture
            .process(&private_text_update_body(99, 888, None))
            .await
            .expect("acked");
        assert_eq!(
            fixture.bot_api.sends(),
            vec![(888, UNPAIRED_HINT_REPLY.to_string())],
            "unpaired media-only message hints"
        );
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 1);
    }

    // Static-reply failures are swallowed: the webhook still acks.
    #[tokio::test]
    async fn send_failures_are_swallowed() {
        let fixture = fixture().await;
        fixture.bot_api.fail_sends();

        let outcome = fixture
            .process(&private_text_update_body(42, 555, Some("/start")))
            .await
            .expect("acked despite send failure");

        assert_silently_handled(&outcome);
        assert_eq!(fixture.bot_api.sends().len(), 1, "send was attempted");
    }

    // An identity-store outage must not silently drop a possibly-paired
    // sender's message: surface a retryable error so the transport retries.
    #[tokio::test]
    async fn lookup_outage_maps_to_retryable_error() {
        let fixture = fixture().await;
        fixture.lookup.fail();

        let error = fixture
            .process(&private_text_update_body(42, 555, Some("hello there")))
            .await
            .expect_err("lookup outage is not an ack");

        match &error {
            RunnerError::Adapter(adapter_error) => {
                assert!(adapter_error.is_retryable(), "must ask for redelivery");
            }
            other => panic!("expected retryable adapter error, got: {other:?}"),
        }
        assert_eq!(fixture.runner_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.bot_api.sends().is_empty());
    }

    #[test]
    fn classify_inbound_text_covers_admission_rows() {
        assert_eq!(
            classify_inbound_text(Some("/start")),
            InboundTextClass::StartWithoutPayload
        );
        assert_eq!(
            classify_inbound_text(Some("  /start  ABCDEFGH ")),
            InboundTextClass::PairingAttempt("ABCDEFGH".to_string())
        );
        assert_eq!(
            classify_inbound_text(Some("abcdefgh")),
            InboundTextClass::PairingAttempt("abcdefgh".to_string())
        );
        // O, 0, 1 and I are not in the pairing alphabet: not code-shaped.
        assert_eq!(
            classify_inbound_text(Some("NOPE1234")),
            InboundTextClass::Ordinary
        );
        assert_eq!(
            classify_inbound_text(Some("hello there")),
            InboundTextClass::Ordinary
        );
        assert_eq!(
            classify_inbound_text(Some("/start CODE EXTRA")),
            InboundTextClass::Ordinary
        );
        assert_eq!(classify_inbound_text(None), InboundTextClass::Ordinary);
        // Multi-byte text must not panic the length checks.
        assert_eq!(
            classify_inbound_text(Some("héllö wörld ✅")),
            InboundTextClass::Ordinary
        );
    }
}
