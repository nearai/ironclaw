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
//!   [`crate::ingress::TelegramUpdatesWebhookDispatcher`]
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

use crate::bot_api::HostEgressTelegramBotApi;
use crate::ingress::TelegramUpdatesWebhookDispatcher;
use crate::pairing::{
    PAIRING_CODE_ALPHABET, PAIRING_CODE_LEN, PairingConsumeOutcome, TelegramPairingService,
};
use crate::setup::TelegramSetupService;
use crate::telegram_actor_identity::{
    TELEGRAM_IDENTITY_PROVIDER, telegram_user_identity_provider_user_id,
};
use ironclaw_channel_host::identity::RebornUserIdentityLookup;

const TRACING_TARGET: &str = "ironclaw::reborn::telegram_updates";
const PRIVATE_CHAT_KIND: &str = "private";
const START_COMMAND: &str = "/start";
// Anti-amplification guard, not a UX pacing choice: the hint is an
// auto-reply to UNAUTHENTICATED senders, so an unthrottled version turns any
// inbound flood into an equal outbound flood (Telegram bot quota + spam
// flags). 30s bounds the loop (at most one hint per 30s window per chat —
// i.e. ~2/minute on average) while a real user never
// waits long enough to conclude the bot is dead — the prior 10-minute window
// read as total silence in live testing (2026-07-17).
const HINT_THROTTLE_WINDOW: Duration = Duration::from_secs(30);
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
    bot_api: Arc<HostEgressTelegramBotApi>,
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
        bot_api: Arc<HostEgressTelegramBotApi>,
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
            .consume(
                &self.installation_id,
                code,
                &telegram_user_id.to_string(),
                chat_id,
            )
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
        if !self.send_static_reply(chat_id, UNPAIRED_HINT_REPLY).await {
            // The hint never reached the user — release the throttle slot so
            // the next message retries instead of inheriting up to a full
            // window of silence from one failed send (live-observed
            // 2026-07-17 as "DMing the disconnected bot gets no reply").
            self.hint_throttle.lock().await.remove(&chat_id);
        }
    }

    /// At most one hint per chat per window. Entries older than the window
    /// are pruned on every check so the map stays bounded by active chats.
    async fn hint_allowed(&self, chat_id: i64) -> bool {
        throttle_allows(&self.hint_throttle, self.hint_throttle_window, chat_id).await
    }

    /// Returns `true` when the reply reached the Bot API successfully;
    /// best-effort — failures are logged at debug and the webhook still acks.
    async fn send_static_reply(&self, chat_id: i64, text: &str) -> bool {
        let bot_token = match self.setup.bot_token().await {
            Ok(Some(token)) => token,
            Ok(None) => {
                tracing::debug!(
                    target = TRACING_TARGET,
                    "telegram bot token not configured; static reply skipped"
                );
                return false;
            }
            Err(error) => {
                tracing::debug!(
                    target = TRACING_TARGET,
                    reason = %error,
                    "telegram bot token unavailable; static reply skipped"
                );
                return false;
            }
        };
        if let Err(error) = self.bot_api.send_message(&bot_token, chat_id, text).await {
            tracing::debug!(
                target = TRACING_TARGET,
                reason = %error,
                "telegram static reply send failed; webhook still acked"
            );
            return false;
        }
        true
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
#[path = "dispatch_tests.rs"]
mod dispatch_tests;
#[cfg(test)]
pub(crate) use dispatch_tests::test_fixtures;
