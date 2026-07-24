//! The Slack [`ChannelAdapter`] (generic ingress cutover P4; delivery
//! coordinator cutover P5).
//!
//! `inbound` parses one HOST-VERIFIED Slack Events API request into a
//! normalized outcome (signature verification lives in the host's generic
//! recipe verifier; this adapter never sees signing secrets). `deliver`
//! renders one coordinator envelope to Slack mrkdwn, splits oversized text,
//! posts each message via `chat.postMessage` over restricted egress (the
//! host injects the bot token by declared handle), and maps vendor errors to
//! structured per-part outcomes — the adapter has no store and cannot mark
//! anything delivered.

use async_trait::async_trait;
use ironclaw_host_api::product_adapter::{
    AdapterInstallationId, ChannelAdapter, ChannelError, DeliveryReport, ExternalConversationRef,
    ImmediateResponse, InboundOutcome, OutboundEnvelope, OutboundPart, PartDeliveryOutcome,
    TargetCandidate, TargetQuery, VerifiedInbound, render_channel_auth_prompt,
};
use ironclaw_host_api::{
    NetworkMethod, RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, SecretHandle,
};
use serde::Deserialize;

use crate::delivery::{SlackDeliveryFailureKind, slack_error_kind};
use crate::mrkdwn::{render_slack_mrkdwn, slack_text_chunks};
use crate::payload::{
    SLACK_API_HOST, SlackInboundEvent, SlackPayloadParseError, normalize_slack_event,
};

/// The administrator-configuration handle carrying the bot token (manifest data; the
/// host injects the secret at egress time).
const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";

/// Stateless Slack channel adapter: pure protocol parsing for the generic
/// ingress router.
#[derive(Debug, Default, Clone, Copy)]
pub struct SlackChannelAdapter;

#[async_trait]
impl ChannelAdapter for SlackChannelAdapter {
    fn inbound(&self, request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        let installation_id =
            AdapterInstallationId::new(request.installation_id).map_err(|error| {
                ChannelError::Parse {
                    reason: format!("invalid installation id: {error}"),
                }
            })?;
        match normalize_slack_event(request.body, &installation_id).map_err(parse_error)? {
            SlackInboundEvent::UrlVerification { challenge } => {
                Ok(InboundOutcome::Respond(ImmediateResponse {
                    status: 200,
                    content_type: Some("text/plain".to_string()),
                    body: challenge.into_bytes(),
                }))
            }
            SlackInboundEvent::Ignore => Ok(InboundOutcome::Ignore),
            SlackInboundEvent::Message(message) => Ok(InboundOutcome::Messages(vec![*message])),
        }
    }

    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        if envelope.parts.is_empty() {
            return Err(ChannelError::Render {
                reason: "outbound envelope carries no parts".to_string(),
            });
        }
        let credential =
            SecretHandle::new(SLACK_BOT_TOKEN_HANDLE).map_err(|error| ChannelError::Render {
                reason: format!("invalid bot token handle: {error}"),
            })?;
        let channel = envelope.target.conversation.conversation_id().to_string();
        // Reply threading: an explicit anchor wins; otherwise thread on the
        // conversation's topic (the inbound thread the reply belongs to).
        let thread_ts = envelope
            .target
            .thread_anchor
            .clone()
            .or_else(|| envelope.target.conversation.topic_id().map(str::to_string));

        let mut parts = Vec::new();
        'parts: for part in &envelope.parts {
            match part {
                OutboundPart::Text(markdown) => {
                    let rendered = render_slack_mrkdwn(markdown);
                    for chunk in slack_text_chunks(&rendered) {
                        let outcome = post_slack_chunk(
                            egress,
                            &credential,
                            &channel,
                            thread_ts.as_deref(),
                            &chunk,
                        )
                        .await;
                        let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                        parts.push(outcome);
                        if !sent {
                            // The report describes exactly what the vendor
                            // accepted; the coordinator owns retry semantics
                            // (a partial multipart is terminal there).
                            break 'parts;
                        }
                    }
                }
                OutboundPart::AuthPrompt {
                    view,
                    direct_message,
                } => {
                    let markdown = render_channel_auth_prompt(view, *direct_message);
                    let rendered = render_slack_mrkdwn(&markdown);
                    for chunk in slack_text_chunks(&rendered) {
                        let outcome = post_slack_chunk(
                            egress,
                            &credential,
                            &channel,
                            thread_ts.as_deref(),
                            &chunk,
                        )
                        .await;
                        let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                        parts.push(outcome);
                        if !sent {
                            break 'parts;
                        }
                    }
                }
                OutboundPart::Retract { vendor_message_ref } => {
                    let outcome =
                        delete_slack_message(egress, &credential, &channel, vendor_message_ref)
                            .await;
                    let sent = matches!(outcome, PartDeliveryOutcome::Sent { .. });
                    parts.push(outcome);
                    if !sent {
                        break 'parts;
                    }
                }
            }
        }
        Ok(DeliveryReport { parts })
    }

    /// Target listing. The `im:<slack_user_id>` query provisions (or reuses)
    /// the 1:1 DM conversation with that user via `conversations.open` — the
    /// vendor mechanics half of personal-DM target provisioning.
    async fn list_targets(
        &self,
        query: TargetQuery,
        egress: &dyn RestrictedEgress,
    ) -> Result<Vec<TargetCandidate>, ChannelError> {
        let Some(slack_user_id) = query
            .query
            .as_deref()
            .and_then(|value| value.strip_prefix("im:"))
            .filter(|value| !value.is_empty())
        else {
            return Err(ChannelError::Unsupported);
        };
        let credential = SecretHandle::new(SLACK_BOT_TOKEN_HANDLE).map_err(|error| {
            ChannelError::VendorWiring {
                reason: format!("invalid bot token handle: {error}"),
            }
        })?;
        let body = serde_json::to_vec(&serde_json::json!({ "users": slack_user_id })).map_err(
            |error| ChannelError::VendorWiring {
                reason: format!("conversations.open body did not serialize: {error}"),
            },
        )?;
        let response = egress
            .send(RestrictedEgressRequest {
                method: NetworkMethod::Post,
                url: format!("https://{SLACK_API_HOST}/api/conversations.open"),
                headers: vec![(
                    "content-type".to_string(),
                    "application/json; charset=utf-8".to_string(),
                )],
                body: Some(body),
                credential: Some(credential),
                body_credentials: Vec::new(),
            })
            .await
            .map_err(|error| ChannelError::VendorWiring {
                reason: format!("conversations.open egress failed: {error}"),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(ChannelError::VendorWiring {
                reason: format!("slack web api returned status {}", response.status),
            });
        }
        let parsed: SlackConversationsOpenResponse = serde_json::from_slice(&response.body)
            .map_err(|error| ChannelError::VendorWiring {
                reason: format!("conversations.open response was not valid JSON: {error}"),
            })?;
        if !parsed.ok {
            return Err(ChannelError::VendorWiring {
                reason: format!(
                    "slack rejected conversations.open ({})",
                    parsed.error.unwrap_or_else(|| "unknown_error".to_string())
                ),
            });
        }
        let channel_id = parsed
            .channel
            .map(|channel| channel.id)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ChannelError::VendorWiring {
                reason: "conversations.open response missing channel id".to_string(),
            })?;
        let conversation =
            ExternalConversationRef::new(None, &channel_id, None, None).map_err(|error| {
                ChannelError::VendorWiring {
                    reason: format!("conversations.open returned an invalid channel id: {error}"),
                }
            })?;
        Ok(vec![TargetCandidate {
            conversation,
            display_name: "Direct message".to_string(),
        }])
    }
}

#[derive(Debug, Deserialize)]
struct SlackConversationsOpenResponse {
    ok: bool,
    error: Option<String>,
    channel: Option<SlackOpenedConversation>,
}

#[derive(Debug, Deserialize)]
struct SlackOpenedConversation {
    id: String,
}

#[derive(Debug, Deserialize)]
struct SlackChatPostMessageResponse {
    ok: bool,
    error: Option<String>,
    ts: Option<String>,
}

async fn post_slack_chunk(
    egress: &dyn RestrictedEgress,
    credential: &SecretHandle,
    channel: &str,
    thread_ts: Option<&str>,
    text: &str,
) -> PartDeliveryOutcome {
    let mut body = serde_json::json!({ "channel": channel, "text": text });
    if let Some(thread_ts) = thread_ts {
        body["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
    }
    let body = match serde_json::to_vec(&body) {
        Ok(body) => body,
        Err(error) => {
            return PartDeliveryOutcome::Permanent {
                reason: format!("chat.postMessage body did not serialize: {error}"),
            };
        }
    };
    let response = egress
        .send(RestrictedEgressRequest {
            method: NetworkMethod::Post,
            url: format!("https://{SLACK_API_HOST}/api/chat.postMessage"),
            headers: vec![(
                "content-type".to_string(),
                "application/json; charset=utf-8".to_string(),
            )],
            body: Some(body),
            credential: Some(credential.clone()),
            body_credentials: Vec::new(),
        })
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => return part_outcome_for_egress_error(&error),
    };
    if !(200..300).contains(&response.status) {
        return part_outcome_for_kind(
            SlackDeliveryFailureKind::from_http_status(response.status),
            format!("slack web api returned status {}", response.status),
        );
    }
    let parsed: SlackChatPostMessageResponse = match serde_json::from_slice(&response.body) {
        Ok(parsed) => parsed,
        // A truncated body from a proxy/LB timeout is transient infra.
        Err(error) => {
            return PartDeliveryOutcome::Retryable {
                reason: format!("chat.postMessage response was not valid JSON: {error}"),
            };
        }
    };
    if parsed.ok {
        return PartDeliveryOutcome::Sent {
            vendor_message_ref: parsed.ts,
        };
    }
    let error = parsed.error.unwrap_or_else(|| "unknown_error".to_string());
    part_outcome_for_kind(
        slack_error_kind(&error),
        format!("slack rejected chat.postMessage ({error})"),
    )
}

/// Retract an earlier post (`chat.delete`). The `vendor_message_ref` is the
/// `ts` a previous `Sent` outcome returned; the channel comes from the
/// envelope's target conversation.
async fn delete_slack_message(
    egress: &dyn RestrictedEgress,
    credential: &SecretHandle,
    channel: &str,
    ts: &str,
) -> PartDeliveryOutcome {
    let body = match serde_json::to_vec(&serde_json::json!({ "channel": channel, "ts": ts })) {
        Ok(body) => body,
        Err(error) => {
            return PartDeliveryOutcome::Permanent {
                reason: format!("chat.delete body did not serialize: {error}"),
            };
        }
    };
    let response = egress
        .send(RestrictedEgressRequest {
            method: NetworkMethod::Post,
            url: format!("https://{SLACK_API_HOST}/api/chat.delete"),
            headers: vec![(
                "content-type".to_string(),
                "application/json; charset=utf-8".to_string(),
            )],
            body: Some(body),
            credential: Some(credential.clone()),
            body_credentials: Vec::new(),
        })
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => return part_outcome_for_egress_error(&error),
    };
    if !(200..300).contains(&response.status) {
        return part_outcome_for_kind(
            SlackDeliveryFailureKind::from_http_status(response.status),
            format!("slack web api returned status {}", response.status),
        );
    }
    let parsed: SlackChatPostMessageResponse = match serde_json::from_slice(&response.body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return PartDeliveryOutcome::Retryable {
                reason: format!("chat.delete response was not valid JSON: {error}"),
            };
        }
    };
    if parsed.ok {
        return PartDeliveryOutcome::Sent {
            vendor_message_ref: None,
        };
    }
    let error = parsed.error.unwrap_or_else(|| "unknown_error".to_string());
    part_outcome_for_kind(
        slack_error_kind(&error),
        format!("slack rejected chat.delete ({error})"),
    )
}

fn part_outcome_for_egress_error(error: &RestrictedEgressError) -> PartDeliveryOutcome {
    match error {
        RestrictedEgressError::Transport { .. } => PartDeliveryOutcome::Retryable {
            reason: error.to_string(),
        },
        RestrictedEgressError::AuthRequired { .. }
        | RestrictedEgressError::UndeclaredCredential { .. } => PartDeliveryOutcome::Unauthorized {
            reason: error.to_string(),
        },
        RestrictedEgressError::UndeclaredHost { .. }
        | RestrictedEgressError::UndeclaredMethod
        | RestrictedEgressError::HostOwnedHeader { .. }
        | RestrictedEgressError::PolicyDenied
        | RestrictedEgressError::ResponseTooLarge => PartDeliveryOutcome::Permanent {
            reason: error.to_string(),
        },
    }
}

fn part_outcome_for_kind(kind: SlackDeliveryFailureKind, reason: String) -> PartDeliveryOutcome {
    match kind {
        SlackDeliveryFailureKind::Retryable => PartDeliveryOutcome::Retryable { reason },
        SlackDeliveryFailureKind::Unauthorized => PartDeliveryOutcome::Unauthorized { reason },
        SlackDeliveryFailureKind::Permanent => PartDeliveryOutcome::Permanent { reason },
    }
}

fn parse_error(error: SlackPayloadParseError) -> ChannelError {
    ChannelError::Parse {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::product_adapter::ProductTriggerReason;

    use super::*;

    fn inbound(body: &[u8]) -> Result<InboundOutcome, ChannelError> {
        SlackChannelAdapter.inbound(VerifiedInbound {
            extension_id: "slack",
            installation_id: "install_alpha",
            body,
            headers: &[],
        })
    }

    #[test]
    fn url_verification_challenge_becomes_an_immediate_response() {
        let outcome = inbound(br#"{"type":"url_verification","challenge":"challenge-token"}"#)
            .expect("challenge parses");
        let InboundOutcome::Respond(response) = outcome else {
            panic!("expected Respond");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"challenge-token");
    }

    #[test]
    fn dm_message_normalizes_with_text_trigger_and_event_identity() {
        let outcome = inbound(
            br#"{
                "type": "event_callback",
                "event_id": "Ev123",
                "team_id": "T-A",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "D123",
                    "channel_type": "im",
                    "text": "hello there",
                    "ts": "1710000000.000100"
                }
            }"#,
        )
        .expect("dm parses");
        let InboundOutcome::Messages(messages) = outcome else {
            panic!("expected Messages");
        };
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.text, "hello there");
        assert_eq!(message.trigger, ProductTriggerReason::DirectChat);
        assert_eq!(message.event_id.as_str(), "slack-install_alpha-Ev123");
        assert_eq!(message.actor.id(), "U123");
        assert_eq!(message.conversation.conversation_id(), "D123");
        assert!(message.reply_context.is_none());
    }

    #[test]
    fn app_mention_strips_the_leading_mention_and_keeps_thread_anchor() {
        let outcome = inbound(
            br#"{
                "type": "event_callback",
                "event_id": "Ev124",
                "team_id": "T-A",
                "event": {
                    "type": "app_mention",
                    "user": "U123",
                    "channel": "C123",
                    "text": "<@UBOT> summarize this",
                    "ts": "1710000000.000200"
                }
            }"#,
        )
        .expect("mention parses");
        let InboundOutcome::Messages(messages) = outcome else {
            panic!("expected Messages");
        };
        assert_eq!(messages[0].text, "summarize this");
        assert_eq!(messages[0].trigger, ProductTriggerReason::BotMention);
        assert_eq!(
            messages[0].conversation.topic_id(),
            Some("1710000000.000200"),
            "mention without thread anchors on its own ts"
        );
    }

    #[test]
    fn gate_resolution_text_stays_a_plain_message_for_host_reclassification() {
        // The adapter must NOT classify gate resolutions — the shared host
        // sink applies the channel-neutral interaction grammar.
        let outcome = inbound(
            br#"{
                "type": "event_callback",
                "event_id": "Ev125",
                "team_id": "T-A",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "D123",
                    "channel_type": "im",
                    "text": "approve gate:approval-00000000-0000-0000-0000-000000000001",
                    "ts": "1710000000.000300"
                }
            }"#,
        )
        .expect("resolution text parses");
        let InboundOutcome::Messages(messages) = outcome else {
            panic!("expected Messages");
        };
        assert!(messages[0].text.starts_with("approve gate:"));
    }

    #[test]
    fn ignored_events_and_bot_echoes_are_authenticated_noops() {
        for body in [
            // Non event_callback wrapper.
            br#"{"type":"team_join","event_id":"Ev1"}"#.as_slice(),
            // Bot echo of its own message.
            br#"{
                "type": "event_callback",
                "event_id": "Ev2",
                "event": {
                    "type": "message",
                    "bot_id": "B123",
                    "channel": "D123",
                    "channel_type": "im",
                    "text": "echo",
                    "ts": "1710000000.000400"
                }
            }"#
            .as_slice(),
            // Channel message without a thread anchor (no mention).
            br#"{
                "type": "event_callback",
                "event_id": "Ev3",
                "event": {
                    "type": "message",
                    "user": "U123",
                    "channel": "C123",
                    "text": "ambient chatter",
                    "ts": "1710000000.000500"
                }
            }"#
            .as_slice(),
        ] {
            assert!(
                matches!(inbound(body), Ok(InboundOutcome::Ignore)),
                "expected Ignore for {}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn malformed_payloads_are_typed_parse_errors() {
        assert!(matches!(
            inbound(br#"{"type":"event_callback""#),
            Err(ChannelError::Parse { .. })
        ));
        // event_callback without event_id would collide dedupe keys.
        assert!(matches!(
            inbound(br#"{"type":"event_callback","event":{"type":"message"}}"#),
            Err(ChannelError::Parse { .. })
        ));
    }

    // ── deliver() (delivery coordinator cutover, extension-runtime P5) ──────

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use ironclaw_host_api::product_adapter::{OutboundPart, PartDeliveryOutcome};
    use ironclaw_host_api::{
        RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
    };

    struct ScriptedEgress {
        requests: Mutex<Vec<RestrictedEgressRequest>>,
        responses: Mutex<VecDeque<Result<RestrictedEgressResponse, RestrictedEgressError>>>,
    }

    impl ScriptedEgress {
        fn new(responses: Vec<Result<RestrictedEgressResponse, RestrictedEgressError>>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }

        fn ok(body: &str) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            Ok(RestrictedEgressResponse {
                status: 200,
                body: body.as_bytes().to_vec(),
            })
        }

        fn requests(&self) -> Vec<RestrictedEgressRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl RestrictedEgress for ScriptedEgress {
        async fn send(
            &self,
            request: RestrictedEgressRequest,
        ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(RestrictedEgressError::PolicyDenied))
        }
    }

    fn envelope(parts: Vec<OutboundPart>, thread_anchor: Option<&str>) -> OutboundEnvelope {
        OutboundEnvelope {
            extension_id: "slack".to_string(),
            installation_id: "install_alpha".to_string(),
            delivery_attempt_id: "attempt-1".to_string(),
            target: ironclaw_host_api::product_adapter::OutboundTarget {
                conversation: ironclaw_host_api::product_adapter::ExternalConversationRef::new(
                    Some("T-A"),
                    "D123",
                    Some("1710000000.000100"),
                    None,
                )
                .expect("conversation"),
                thread_anchor: thread_anchor.map(str::to_string),
            },
            parts,
            reply_context: None,
        }
    }

    fn body_json(request: &RestrictedEgressRequest) -> serde_json::Value {
        serde_json::from_slice(request.body.as_deref().unwrap_or_default()).expect("json body")
    }

    #[tokio::test]
    async fn deliver_posts_one_rendered_message_with_the_bot_token_handle() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":true,"ts":"1710000001.000001"}"#,
        )]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(
                    vec![OutboundPart::Text("**bold** reply".to_string())],
                    Some("1710000000.000100"),
                ),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert_eq!(report.parts.len(), 1);
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Sent { vendor_message_ref: Some(ts) } if ts == "1710000001.000001"
        ));
        let requests = egress.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://slack.com/api/chat.postMessage");
        assert_eq!(
            requests[0].credential.as_ref().map(|h| h.as_str()),
            Some("slack_bot_token"),
            "auth rides the declared handle; the adapter never sees bytes"
        );
        let body = body_json(&requests[0]);
        assert_eq!(body["channel"], "D123");
        assert_eq!(body["thread_ts"], "1710000000.000100");
        assert_eq!(body["text"], "*bold* reply", "markdown renders to mrkdwn");
    }

    #[tokio::test]
    async fn list_targets_im_query_opens_the_dm_conversation() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":true,"channel":{"id":"D777"}}"#,
        )]);
        let candidates = SlackChannelAdapter
            .list_targets(
                ironclaw_host_api::product_adapter::TargetQuery {
                    extension_id: "slack".to_string(),
                    installation_id: "install_alpha".to_string(),
                    query: Some("im:U123".to_string()),
                    limit: 1,
                },
                &egress,
            )
            .await
            .expect("list_targets drives");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].conversation.conversation_id(), "D777");
        let requests = egress.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://slack.com/api/conversations.open");
        assert_eq!(
            requests[0].credential.as_ref().map(|h| h.as_str()),
            Some("slack_bot_token")
        );
        let body = body_json(&requests[0]);
        assert_eq!(body["users"], "U123");
    }

    #[tokio::test]
    async fn list_targets_rejects_non_im_queries_without_egress() {
        let egress = ScriptedEgress::new(Vec::new());
        let error = SlackChannelAdapter
            .list_targets(
                ironclaw_host_api::product_adapter::TargetQuery {
                    extension_id: "slack".to_string(),
                    installation_id: "install_alpha".to_string(),
                    query: None,
                    limit: 10,
                },
                &egress,
            )
            .await
            .expect_err("free listing is not supported yet");
        assert!(matches!(error, ChannelError::Unsupported));
        assert!(egress.requests().is_empty());
    }

    #[tokio::test]
    async fn deliver_retract_part_deletes_the_referenced_message() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(r#"{"ok":true}"#)]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(
                    vec![OutboundPart::Retract {
                        vendor_message_ref: "1710000001.000001".to_string(),
                    }],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert_eq!(report.parts.len(), 1);
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Sent {
                vendor_message_ref: None
            }
        ));
        let requests = egress.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://slack.com/api/chat.delete");
        assert_eq!(
            requests[0].credential.as_ref().map(|h| h.as_str()),
            Some("slack_bot_token")
        );
        let body = body_json(&requests[0]);
        assert_eq!(body["channel"], "D123");
        assert_eq!(body["ts"], "1710000001.000001");
    }

    #[tokio::test]
    async fn deliver_retract_vendor_rejection_maps_to_permanent() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":false,"error":"message_not_found"}"#,
        )]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(
                    vec![OutboundPart::Retract {
                        vendor_message_ref: "1710000001.000001".to_string(),
                    }],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Permanent { reason } if reason.contains("message_not_found")
        ));
    }

    #[tokio::test]
    async fn deliver_threads_on_the_conversation_topic_when_no_anchor_is_given() {
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(r#"{"ok":true,"ts":"1"}"#)]);
        SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::Text("hi".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");
        let body = body_json(&egress.requests()[0]);
        assert_eq!(
            body["thread_ts"], "1710000000.000100",
            "falls back to the conversation's thread topic"
        );
    }

    #[tokio::test]
    async fn deliver_splits_oversized_text_into_sequenced_posts() {
        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(r#"{"ok":true,"ts":"1"}"#),
            ScriptedEgress::ok(r#"{"ok":true,"ts":"2"}"#),
        ]);
        let long_text = "line\n".repeat(9_000); // 45k chars > the 35k soft limit
        let report = SlackChannelAdapter
            .deliver(envelope(vec![OutboundPart::Text(long_text)], None), &egress)
            .await
            .expect("deliver drives");
        assert_eq!(report.parts.len(), 2, "split into two vendor posts");
        assert!(
            report
                .parts
                .iter()
                .all(|part| matches!(part, PartDeliveryOutcome::Sent { .. }))
        );
        let requests = egress.requests();
        assert_eq!(requests.len(), 2);
        let first = body_json(&requests[0]);
        assert!(
            first["text"].as_str().unwrap().starts_with("Part 1/2"),
            "chunks are sequenced"
        );
    }

    #[tokio::test]
    async fn deliver_maps_vendor_errors_and_stops_after_the_first_failure() {
        // ratelimited → Retryable; nothing further is attempted.
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":false,"error":"ratelimited"}"#,
        )]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(
                    vec![
                        OutboundPart::Text("one".to_string()),
                        OutboundPart::Text("two".to_string()),
                    ],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert_eq!(report.parts.len(), 1, "stops at the first failed part");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Retryable { .. }
        ));
        assert_eq!(egress.requests().len(), 1);

        // invalid_auth → Unauthorized.
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":false,"error":"invalid_auth"}"#,
        )]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::Text("x".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Unauthorized { .. }
        ));

        // channel_not_found → Permanent.
        let egress = ScriptedEgress::new(vec![ScriptedEgress::ok(
            r#"{"ok":false,"error":"channel_not_found"}"#,
        )]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::Text("x".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Permanent { .. }
        ));
    }

    #[tokio::test]
    async fn deliver_maps_egress_failures_without_leaking_details() {
        let egress = ScriptedEgress::new(vec![Err(RestrictedEgressError::Transport {
            reason: "connection timed out".to_string(),
        })]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::Text("x".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Retryable { .. }
        ));

        let egress = ScriptedEgress::new(vec![Err(RestrictedEgressError::AuthRequired {
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        })]);
        let report = SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::Text("x".to_string())], None),
                &egress,
            )
            .await
            .expect("deliver drives");
        assert!(matches!(
            &report.parts[0],
            PartDeliveryOutcome::Unauthorized { .. }
        ));
    }

    #[tokio::test]
    async fn deliver_rejects_empty_envelopes() {
        let egress = ScriptedEgress::new(Vec::new());
        let error = SlackChannelAdapter
            .deliver(envelope(Vec::new(), None), &egress)
            .await
            .expect_err("empty envelope is a render error");
        assert!(matches!(error, ChannelError::Render { .. }));
    }
}
