//! Telegram reply-target interpretation and status-message delivery.

use ironclaw_channel_host::delivery_protocol::FinalReplyDeliveryError;

/// Telegram's [`ironclaw_channel_host::delivery_protocol::ChannelDeliveryProtocol`]:
/// `tg:` binding-ref decoding, positive-chat-id DM classification, and the
/// host-authored status messages the delivery machinery posts around the
/// adapter render path (working message, busy hints, blocked-run notices).
/// Status posts ride the policy-scoped Telegram egress as `sendMessage`
/// bodies — the URL's `/bot{telegram_bot_token}` segment is substituted by
/// the mediated egress from the opaque credential handle, exactly like the
/// setup-time Bot API client — and the returned `message_id` handle lets the
/// observer delete its working message (`deleteMessage`) once the reply
/// lands.
#[derive(Debug, Default)]
pub struct TelegramDeliveryProtocol;

const TELEGRAM_SEND_MESSAGE_PATH: &str = "/sendMessage";
const TELEGRAM_DELETE_MESSAGE_PATH: &str = "/deleteMessage";
/// Cap on the provider `description` text kept as a debug diagnostic; the
/// text never rides an error value (mirrors `bot_api`).
const STATUS_DIAGNOSTIC_MAX_CHARS: usize = 160;

/// Bot API envelope for the status-message calls. `result` stays raw JSON:
/// `sendMessage` returns the message object, `deleteMessage` returns `true`.
#[derive(Debug, serde::Deserialize)]
struct TelegramStatusEnvelope {
    ok: bool,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    description: Option<String>,
}

fn status_error(reason: impl Into<String>) -> FinalReplyDeliveryError {
    FinalReplyDeliveryError::StatusMessage {
        reason: reason.into(),
    }
}

/// The origin-form Bot API request the policy-scoped Telegram egress expects:
/// declared host, method path (the egress prepends the credential-placeholder
/// bot segment), JSON body, opaque token handle.
fn telegram_status_request(
    path: &'static str,
    body: Vec<u8>,
) -> Result<ironclaw_product_adapters::EgressRequest, ironclaw_product_adapters::ProductAdapterError>
{
    use ironclaw_product_adapters::{
        DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
        EgressRequest,
    };
    Ok(EgressRequest::new(
        DeclaredEgressHost::new(crate::bot_api::TELEGRAM_API_HOST)?,
        EgressMethod::post(),
        EgressPath::new(path)?,
    )
    .with_header(EgressHeader::new("content-type", "application/json")?)
    .with_body(body)
    .with_credential_handle(Some(EgressCredentialHandle::new(
        crate::egress::TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE,
    )?)))
}

/// Send one status-message Bot API call and return the parsed `result`.
/// Non-2xx statuses and `ok: false` envelopes map to a stable
/// [`FinalReplyDeliveryError::StatusMessage`] reason; the provider's
/// free-text `description` is a bounded debug diagnostic only.
async fn telegram_status_call(
    egress: &dyn ironclaw_product_adapters::ProtocolHttpEgress,
    method_name: &'static str,
    path: &'static str,
    body: serde_json::Value,
) -> Result<serde_json::Value, FinalReplyDeliveryError> {
    let body = serde_json::to_vec(&body).map_err(|error| status_error(error.to_string()))?;
    let response = egress
        .send(telegram_status_request(path, body)?)
        .await
        .map_err(|error| status_error(error.to_string()))?;
    let envelope: Option<TelegramStatusEnvelope> = serde_json::from_slice(response.body()).ok();
    let description: String = envelope
        .as_ref()
        .and_then(|envelope| envelope.description.clone())
        .unwrap_or_default()
        .chars()
        .take(STATUS_DIAGNOSTIC_MAX_CHARS)
        .collect();
    if !(200..300).contains(&response.status()) {
        tracing::debug!(
            status = response.status(),
            method_name,
            description,
            "telegram status message call rejected"
        );
        return Err(status_error(format!(
            "Telegram {method_name} returned HTTP {}",
            response.status()
        )));
    }
    let envelope = envelope
        .ok_or_else(|| status_error(format!("Telegram {method_name} response was not JSON")))?;
    if !envelope.ok {
        tracing::debug!(
            status = response.status(),
            method_name,
            description,
            "telegram status message call returned ok=false"
        );
        return Err(status_error(format!("Telegram {method_name} failed")));
    }
    Ok(envelope.result.unwrap_or(serde_json::Value::Null))
}

#[async_trait::async_trait]
impl ironclaw_channel_host::delivery_protocol::ChannelDeliveryProtocol
    for TelegramDeliveryProtocol
{
    fn run_notification_projection_prefix(&self) -> &'static str {
        "telegram"
    }

    fn conversation_id_from_reply_target_binding_ref(
        &self,
        target: &ironclaw_turns::ReplyTargetBindingRef,
    ) -> Option<(String, Option<String>)> {
        // The Telegram adapter renders straight from the
        // `tg:<chat_id>:<topic|_>:<reply|_>` binding ref; Telegram has no
        // space/team dimension.
        let parsed = ironclaw_telegram_v2_adapter::parse_reply_target(target).ok()?;
        Some((parsed.chat_id.to_string(), None))
    }

    fn reply_target_is_personal_dm(&self, target: &ironclaw_turns::ReplyTargetBindingRef) -> bool {
        // Telegram private chats have positive chat ids (groups/supergroups/
        // channels are negative), and the host only stores DM targets from
        // private-chat pairing.
        ironclaw_telegram_v2_adapter::parse_reply_target(target)
            .map(|parsed| parsed.chat_id > 0)
            .unwrap_or(false)
    }

    fn posted_message_from_render_response(
        &self,
        path: &str,
        request_body: &[u8],
        response_body: &[u8],
    ) -> Option<ironclaw_channel_host::delivery_protocol::PostedChannelMessage> {
        if path != TELEGRAM_SEND_MESSAGE_PATH {
            return None;
        }
        let request: serde_json::Value = serde_json::from_slice(request_body).ok()?;
        let response: TelegramStatusEnvelope = serde_json::from_slice(response_body).ok()?;
        if !response.ok {
            return None;
        }
        let message_id = response.result?.get("message_id")?.as_i64()?;
        let conversation_id = request.get("chat_id")?.as_i64()?.to_string();
        Some(
            ironclaw_channel_host::delivery_protocol::PostedChannelMessage {
                conversation_id,
                message_ref: message_id.to_string(),
            },
        )
    }

    fn connect_nudge_message(&self) -> &'static str {
        // Unreachable in practice (the pairing-aware pre-router intercepts
        // unpaired senders before the workflow), kept consistent with the
        // pre-router's static hint.
        "This bot is IronClaw. Pair your account from IronClaw → Extensions → Telegram, then message me here. Already have a pairing code? Just send it in this chat (or /start <code>)."
    }

    fn is_direct_message_conversation(&self, conversation_id: &str) -> bool {
        conversation_id
            .parse::<i64>()
            .is_ok_and(|chat_id| chat_id > 0)
    }

    async fn post_status_message(
        &self,
        egress: &dyn ironclaw_product_adapters::ProtocolHttpEgress,
        conversation: &ironclaw_product_adapters::ExternalConversationRef,
        text: &str,
    ) -> Result<
        ironclaw_channel_host::delivery_protocol::PostedChannelMessage,
        ironclaw_channel_host::delivery_protocol::FinalReplyDeliveryError,
    > {
        // Fail closed before any request is built: a conversation id this
        // channel cannot address (Telegram chat ids are integers) is a
        // foreign or malformed ref, never a network problem.
        let chat_id: i64 = conversation
            .conversation_id()
            .parse()
            .map_err(|_| status_error("telegram status message target is not a numeric chat id"))?;
        let result = telegram_status_call(
            egress,
            "sendMessage",
            TELEGRAM_SEND_MESSAGE_PATH,
            serde_json::json!({ "chat_id": chat_id, "text": text }),
        )
        .await?;
        let message_id = result
            .get("message_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| status_error("Telegram sendMessage result missing message_id"))?;
        Ok(
            ironclaw_channel_host::delivery_protocol::PostedChannelMessage {
                conversation_id: chat_id.to_string(),
                message_ref: message_id.to_string(),
            },
        )
    }

    async fn delete_status_message(
        &self,
        egress: &dyn ironclaw_product_adapters::ProtocolHttpEgress,
        message: &ironclaw_channel_host::delivery_protocol::PostedChannelMessage,
    ) -> Result<(), ironclaw_channel_host::delivery_protocol::FinalReplyDeliveryError> {
        let chat_id: i64 = message.conversation_id.parse().map_err(|_| {
            status_error("telegram posted-message handle has a non-numeric chat id")
        })?;
        let message_id: i64 = message.message_ref.parse().map_err(|_| {
            status_error("telegram posted-message handle has a non-numeric message id")
        })?;
        let result = telegram_status_call(
            egress,
            "deleteMessage",
            TELEGRAM_DELETE_MESSAGE_PATH,
            serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
        )
        .await?;
        // Deletion evidence is the provider's `result: true` — an ok envelope
        // carrying `false` (or no result) did NOT delete anything and must
        // not report success.
        if result != serde_json::Value::Bool(true) {
            return Err(status_error(
                "Telegram deleteMessage did not confirm deletion",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod telegram_delivery_protocol_tests {
    use std::sync::Arc;

    use ironclaw_product_adapters::{
        EgressRequest, EgressResponse, ExternalConversationRef, FakeProtocolHttpEgress,
        ProtocolHttpEgress, ProtocolHttpEgressError,
    };

    use super::TelegramDeliveryProtocol;
    use crate::bot_api::TELEGRAM_API_HOST;
    use crate::egress::TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE;
    use ironclaw_channel_host::delivery_protocol::{
        ChannelDeliveryProtocol, FinalReplyDeliveryError, PostedChannelMessage,
    };

    /// Egress that panics if the protocol touches the network — for the arms
    /// that must fail closed before any request is built.
    #[derive(Debug)]
    struct PanicEgress;

    #[async_trait::async_trait]
    impl ProtocolHttpEgress for PanicEgress {
        async fn send(
            &self,
            _request: EgressRequest,
        ) -> Result<EgressResponse, ProtocolHttpEgressError> {
            panic!("telegram status messages must not reach egress for this input");
        }
    }

    fn telegram_recording_egress() -> Arc<FakeProtocolHttpEgress> {
        let egress = FakeProtocolHttpEgress::new(vec![TELEGRAM_API_HOST.to_string()]);
        egress.allow_credential_handle(TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE);
        Arc::new(egress)
    }

    fn dm_conversation() -> ExternalConversationRef {
        ExternalConversationRef::new(None, "555", None, None).expect("conversation")
    }

    /// The wired happy path: one policy-scoped `sendMessage` egress carrying
    /// the bot-token credential handle (the egress substitutes the braced
    /// `/bot{telegram_bot_token}` path placeholder), the DM chat id, and the
    /// plain status text; the response's `message_id` becomes the posted
    /// handle so the working message can later be deleted.
    #[tokio::test]
    async fn post_status_message_sends_send_message_through_policy_egress() {
        let egress = telegram_recording_egress();
        egress.program_response(
            TELEGRAM_API_HOST,
            Ok(EgressResponse::new(
                200,
                br#"{"ok":true,"result":{"message_id":42}}"#.to_vec(),
            )),
        );

        let posted = TelegramDeliveryProtocol
            .post_status_message(
                egress.as_ref(),
                &dm_conversation(),
                "Ironclaw is thinking...",
            )
            .await
            .expect("wired status message posts");

        assert_eq!(
            posted,
            PostedChannelMessage {
                conversation_id: "555".to_string(),
                message_ref: "42".to_string(),
            }
        );
        let calls = egress.calls();
        assert_eq!(calls.len(), 1, "exactly one sendMessage egress");
        assert_eq!(calls[0].path, "/sendMessage");
        assert_eq!(
            calls[0].credential_handle.as_deref(),
            Some(TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE),
            "the bot token travels as an opaque handle, never material"
        );
        assert!(
            calls[0]
                .headers
                .iter()
                .any(|header| header.name().eq_ignore_ascii_case("content-type")
                    && header.value() == "application/json"),
            "sendMessage body is JSON"
        );
        let body: serde_json::Value =
            serde_json::from_slice(&calls[0].body).expect("sendMessage body is JSON");
        assert_eq!(body["chat_id"], 555, "chat id from the conversation ref");
        assert_eq!(body["text"], "Ironclaw is thinking...");
    }

    #[tokio::test]
    async fn post_status_message_maps_http_rejection_to_status_message_error() {
        let egress = telegram_recording_egress();
        egress.program_response(
            TELEGRAM_API_HOST,
            Ok(EgressResponse::new(
                403,
                br#"{"ok":false,"description":"Forbidden: bot was blocked by the user"}"#.to_vec(),
            )),
        );

        let error = TelegramDeliveryProtocol
            .post_status_message(egress.as_ref(), &dm_conversation(), "working…")
            .await
            .expect_err("non-2xx sendMessage fails honestly");

        match &error {
            FinalReplyDeliveryError::StatusMessage { reason } => {
                assert!(
                    reason.contains("403"),
                    "reason carries the stable HTTP status, got: {reason}"
                );
                assert!(
                    !reason.contains("blocked by the user"),
                    "provider description text must never ride the error, got: {reason}"
                );
            }
            other => panic!("expected StatusMessage error, got: {other:?}"),
        }
        assert_eq!(egress.calls().len(), 1, "the rejection came from egress");
    }

    #[tokio::test]
    async fn post_status_message_maps_not_ok_envelope_to_status_message_error() {
        let egress = telegram_recording_egress();
        egress.program_response(
            TELEGRAM_API_HOST,
            Ok(EgressResponse::new(
                200,
                br#"{"ok":false,"description":"weird 200-with-error"}"#.to_vec(),
            )),
        );

        let error = TelegramDeliveryProtocol
            .post_status_message(egress.as_ref(), &dm_conversation(), "working…")
            .await
            .expect_err("ok:false envelope fails honestly");
        assert!(matches!(
            error,
            FinalReplyDeliveryError::StatusMessage { .. }
        ));
    }

    /// A conversation ref this channel cannot address (non-numeric chat id)
    /// fails closed before any egress request is built.
    #[tokio::test]
    async fn post_status_message_rejects_non_numeric_chat_id_without_egress() {
        let error = TelegramDeliveryProtocol
            .post_status_message(
                &PanicEgress,
                &ExternalConversationRef::new(None, "not-a-chat-id", None, None)
                    .expect("conversation"),
                "working…",
            )
            .await
            .expect_err("non-numeric chat ids cannot be addressed");
        assert!(matches!(
            error,
            FinalReplyDeliveryError::StatusMessage { .. }
        ));
    }

    /// Round-trip: the handle returned by `post_status_message` addresses the
    /// same message via `deleteMessage` (working-message cleanup).
    #[tokio::test]
    async fn delete_status_message_sends_delete_message_for_posted_handle() {
        let egress = telegram_recording_egress();
        egress.program_response(
            TELEGRAM_API_HOST,
            Ok(EgressResponse::new(
                200,
                br#"{"ok":true,"result":{"message_id":42}}"#.to_vec(),
            )),
        );
        egress.program_response(
            TELEGRAM_API_HOST,
            Ok(EgressResponse::new(
                200,
                br#"{"ok":true,"result":true}"#.to_vec(),
            )),
        );

        let posted = TelegramDeliveryProtocol
            .post_status_message(
                egress.as_ref(),
                &dm_conversation(),
                "Ironclaw is thinking...",
            )
            .await
            .expect("post succeeds");
        TelegramDeliveryProtocol
            .delete_status_message(egress.as_ref(), &posted)
            .await
            .expect("delete succeeds");

        let calls = egress.calls();
        assert_eq!(calls.len(), 2, "post then delete");
        assert_eq!(calls[1].path, "/deleteMessage");
        assert_eq!(
            calls[1].credential_handle.as_deref(),
            Some(TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE)
        );
        let body: serde_json::Value =
            serde_json::from_slice(&calls[1].body).expect("deleteMessage body is JSON");
        assert_eq!(body["chat_id"], 555);
        assert_eq!(body["message_id"], 42);
    }

    /// Deletion evidence is `result: true` — an ok envelope whose result is
    /// `false` (or missing) did not delete anything and must surface as a
    /// failure, never optimistic success.
    #[tokio::test]
    async fn delete_status_message_requires_result_true() {
        for stale_body in [
            br#"{"ok":true,"result":false}"#.to_vec(),
            br#"{"ok":true}"#.to_vec(),
        ] {
            let egress = telegram_recording_egress();
            egress.program_response(TELEGRAM_API_HOST, Ok(EgressResponse::new(200, stale_body)));
            let posted = ironclaw_channel_host::delivery_protocol::PostedChannelMessage {
                conversation_id: "555".to_string(),
                message_ref: "42".to_string(),
            };
            let error = TelegramDeliveryProtocol
                .delete_status_message(egress.as_ref(), &posted)
                .await
                .expect_err("unconfirmed deletion must fail");
            assert!(
                error.to_string().contains("did not confirm deletion"),
                "stable evidence-shaped reason, got: {error}"
            );
        }
    }

    #[tokio::test]
    async fn delete_status_message_maps_rejection_to_status_message_error() {
        let egress = telegram_recording_egress();
        egress.program_response(
            TELEGRAM_API_HOST,
            Ok(EgressResponse::new(
                400,
                br#"{"ok":false,"description":"message to delete not found"}"#.to_vec(),
            )),
        );

        let error = TelegramDeliveryProtocol
            .delete_status_message(
                egress.as_ref(),
                &PostedChannelMessage {
                    conversation_id: "555".to_string(),
                    message_ref: "42".to_string(),
                },
            )
            .await
            .expect_err("rejected deleteMessage surfaces honestly");
        assert!(matches!(
            error,
            FinalReplyDeliveryError::StatusMessage { .. }
        ));
    }

    #[test]
    fn telegram_refs_classify_dm_and_conversation() {
        let protocol = TelegramDeliveryProtocol;
        assert!(protocol.is_direct_message_conversation("555"));
        assert!(!protocol.is_direct_message_conversation("-100123"));
        assert!(!protocol.is_direct_message_conversation("not-a-chat-id"));
    }

    #[test]
    fn rendered_send_message_response_produces_telegram_delivery_evidence() {
        let protocol = TelegramDeliveryProtocol;
        assert_eq!(protocol.run_notification_projection_prefix(), "telegram");
        assert_eq!(
            protocol.posted_message_from_render_response(
                "/sendMessage",
                br#"{"chat_id":555,"text":"hello"}"#,
                br#"{"ok":true,"result":{"message_id":42}}"#,
            ),
            Some(PostedChannelMessage {
                conversation_id: "555".to_string(),
                message_ref: "42".to_string(),
            })
        );
        assert!(
            protocol
                .posted_message_from_render_response(
                    "/sendMessage",
                    br#"{"chat_id":555}"#,
                    br#"{"ok":true,"result":true}"#,
                )
                .is_none(),
            "a provider response without a message id is not delivery evidence"
        );
    }
}
