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

    async fn fetch_attachment(
        &self,
        attachment: &ironclaw_host_api::product_adapter::ChannelAttachmentRef,
        egress: &dyn RestrictedEgress,
    ) -> Result<ironclaw_host_api::InboundAttachment, ChannelError> {
        crate::attachment_transfer::fetch_attachment(attachment, egress).await
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
        let mut part_index = 0usize;
        'parts: while part_index < envelope.parts.len() {
            let part = &envelope.parts[part_index];
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
                OutboundPart::File(_) => {
                    let files: Vec<_> = envelope.parts[part_index..]
                        .iter()
                        .map_while(|part| match part {
                            OutboundPart::File(file) => Some(file),
                            _ => None,
                        })
                        .collect();
                    let outcomes = crate::attachment_transfer::send_files(
                        egress,
                        &credential,
                        &channel,
                        thread_ts.as_deref(),
                        &files,
                    )
                    .await;
                    let all_sent = outcomes
                        .iter()
                        .all(|outcome| matches!(outcome, PartDeliveryOutcome::Sent { .. }));
                    parts.extend(outcomes);
                    if !all_sent {
                        break 'parts;
                    }
                    part_index += files.len();
                    continue;
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
            part_index += 1;
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

pub(crate) fn part_outcome_for_egress_error(error: &RestrictedEgressError) -> PartDeliveryOutcome {
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

pub(crate) fn part_outcome_for_kind(
    kind: SlackDeliveryFailureKind,
    reason: String,
) -> PartDeliveryOutcome {
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
            config: &[],
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

    use ironclaw_host_api::product_adapter::{
        ChannelAttachmentRef, OutboundPart, PartDeliveryOutcome, ProductAttachmentDescriptor,
        ProductAttachmentKind,
    };
    use ironclaw_host_api::{
        RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
        ScopedPath, WorkspaceFile,
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

        fn status(status: u16) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            Ok(RestrictedEgressResponse {
                status,
                body: Vec::new(),
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

    fn attachment() -> ChannelAttachmentRef {
        ChannelAttachmentRef {
            descriptor: ProductAttachmentDescriptor::new(
                "F123",
                "text/plain",
                Some("notes.txt".to_string()),
                Some(5),
                ProductAttachmentKind::Document,
            )
            .expect("descriptor"),
            vendor_ref: "F123".to_string(),
        }
    }

    fn workspace_file() -> WorkspaceFile {
        WorkspaceFile {
            path: ScopedPath::new("/workspace/report.txt").expect("path"),
            filename: Some("report.txt".to_string()),
            mime_type: "text/plain".to_string(),
            bytes: b"hello".to_vec(),
        }
    }

    fn successful_upload_prefix() -> Vec<Result<RestrictedEgressResponse, RestrictedEgressError>> {
        vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/ticket","file_id":"FNEW"}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"OK - 5".to_vec(),
            }),
            ScriptedEgress::ok(r#"{"ok":true,"files":[{"id":"FNEW","title":"report.txt"}]}"#),
        ]
    }

    #[tokio::test]
    async fn inbound_attachment_is_resolved_then_downloaded_through_restricted_egress() {
        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"F123","name":"notes.txt","mimetype":"text/plain","size":5,"url_private_download":"https://files.slack.com/files-pri/T-F/download/notes.txt"}}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"hello".to_vec(),
            }),
        ]);

        let fetched = SlackChannelAdapter
            .fetch_attachment(&attachment(), &egress)
            .await
            .expect("attachment fetch succeeds");

        assert_eq!(fetched.id, "F123");
        assert_eq!(fetched.filename.as_deref(), Some("notes.txt"));
        assert_eq!(fetched.mime_type, "text/plain");
        assert_eq!(fetched.bytes, b"hello");
        let requests = egress.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, NetworkMethod::Get);
        assert_eq!(
            requests[0].url,
            "https://slack.com/api/files.info?file=F123"
        );
        assert_eq!(requests[1].method, NetworkMethod::Get);
        assert_eq!(
            requests[1].url,
            "https://files.slack.com/files-pri/T-F/download/notes.txt"
        );
        assert_eq!(
            requests[1].credential.as_ref().map(SecretHandle::as_str),
            Some("slack_bot_token")
        );
    }

    #[tokio::test]
    async fn inbound_attachment_rejects_auth_metadata_url_and_size_failures() {
        let cases = vec![
            (
                ScriptedEgress::ok(r#"{"ok":false,"error":"missing_scope"}"#),
                false,
            ),
            (
                ScriptedEgress::ok(
                    r#"{"ok":true,"file":{"id":"F123","mimetype":"application/pdf","size":5,"url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
                ),
                false,
            ),
            (
                ScriptedEgress::ok(
                    r#"{"ok":true,"file":{"id":"F123","mimetype":"text/plain","size":6,"url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
                ),
                false,
            ),
            (
                ScriptedEgress::ok(
                    r#"{"ok":true,"file":{"id":"F123","mimetype":"text/plain","size":5,"url_private":"https://evil.example/files-pri/T-F/x?token=secret"}}"#,
                ),
                false,
            ),
            (
                Err(RestrictedEgressError::Transport {
                    reason: "timeout".to_string(),
                }),
                true,
            ),
        ];

        for (response, expected_retryable) in cases {
            let egress = ScriptedEgress::new(vec![response]);
            let error = SlackChannelAdapter
                .fetch_attachment(&attachment(), &egress)
                .await
                .expect_err("unsafe or unavailable attachment must fail");
            assert!(matches!(
                error,
                ChannelError::AttachmentTransfer { retryable, .. }
                    if retryable == expected_retryable
            ));
            assert!(
                !error.to_string().contains("evil.example")
                    && !error.to_string().contains("secret"),
                "transient provider URLs must not leak through errors"
            );
        }
    }

    #[tokio::test]
    async fn inbound_attachment_enforces_actual_body_and_response_caps() {
        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"F123","mimetype":"text/plain","size":5,"url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"four".to_vec(),
            }),
        ]);
        let error = SlackChannelAdapter
            .fetch_attachment(&attachment(), &egress)
            .await
            .expect_err("truncated body must fail");
        assert!(matches!(
            error,
            ChannelError::AttachmentTransfer {
                retryable: true,
                ..
            }
        ));

        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"F123","mimetype":"text/plain","size":5,"url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
            ),
            Err(RestrictedEgressError::ResponseTooLarge),
        ]);
        let error = SlackChannelAdapter
            .fetch_attachment(&attachment(), &egress)
            .await
            .expect_err("host response cap must fail");
        assert!(matches!(
            error,
            ChannelError::AttachmentTransfer {
                retryable: false,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn inbound_attachment_fails_closed_across_provider_edge_cases() {
        let mut descriptor_oversized = attachment();
        descriptor_oversized.descriptor.size_bytes =
            Some(crate::attachment_transfer::SLACK_MAX_TRANSFER_BYTES + 1);

        let mut provider_sized = attachment();
        provider_sized.descriptor.size_bytes = None;

        let mut provider_named = provider_sized.clone();
        provider_named.descriptor.filename = None;

        let cases = vec![
            (descriptor_oversized, Vec::new(), false, "size limit"),
            (
                attachment(),
                vec![ScriptedEgress::status(401)],
                false,
                "unauthorized",
            ),
            (
                attachment(),
                vec![ScriptedEgress::status(503)],
                true,
                "temporarily unavailable",
            ),
            (
                attachment(),
                vec![ScriptedEgress::ok(
                    r#"{"ok":true,"file":{"id":"FOTHER","mimetype":"text/plain","size":5,"url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
                )],
                false,
                "identity",
            ),
            (
                provider_sized.clone(),
                vec![ScriptedEgress::ok(&format!(
                    r#"{{"ok":true,"file":{{"id":"F123","mimetype":"text/plain","size":{},"url_private":"https://files.slack.com/files-pri/T-F/x"}}}}"#,
                    crate::attachment_transfer::SLACK_MAX_TRANSFER_BYTES + 1
                ))],
                false,
                "size limit",
            ),
            (
                provider_sized.clone(),
                vec![
                    ScriptedEgress::ok(
                        r#"{"ok":true,"file":{"id":"F123","mimetype":"text/plain","url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
                    ),
                    ScriptedEgress::status(404),
                ],
                false,
                "could not be downloaded",
            ),
            (
                provider_sized,
                vec![
                    ScriptedEgress::ok(
                        r#"{"ok":true,"file":{"id":"F123","mimetype":"text/plain","url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
                    ),
                    Ok(RestrictedEgressResponse {
                        status: 200,
                        body: vec![
                            0;
                            crate::attachment_transfer::SLACK_MAX_TRANSFER_BYTES as usize + 1
                        ],
                    }),
                ],
                false,
                "size limit",
            ),
            (
                provider_named,
                vec![
                    ScriptedEgress::ok(
                        r#"{"ok":true,"file":{"id":"F123","name":"../secret","mimetype":"text/plain","size":5,"url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
                    ),
                    Ok(RestrictedEgressResponse {
                        status: 200,
                        body: b"hello".to_vec(),
                    }),
                ],
                false,
                "filename",
            ),
            (
                attachment(),
                vec![Err(RestrictedEgressError::UndeclaredCredential {
                    handle: "slack_bot_token".to_string(),
                })],
                false,
                "unauthorized",
            ),
            (
                attachment(),
                vec![Err(RestrictedEgressError::PolicyDenied)],
                false,
                "denied",
            ),
            (
                attachment(),
                vec![ScriptedEgress::ok(
                    r#"{"ok":false,"error":"internal_error"}"#,
                )],
                true,
                "temporarily unavailable",
            ),
        ];

        for (attachment, responses, expected_retryable, expected_reason) in cases {
            let egress = ScriptedEgress::new(responses);
            let error = SlackChannelAdapter
                .fetch_attachment(&attachment, &egress)
                .await
                .expect_err("untrusted provider edge case must fail closed");
            assert!(matches!(
                error,
                ChannelError::AttachmentTransfer {
                    ref reason,
                    retryable,
                } if retryable == expected_retryable && reason.contains(expected_reason)
            ));
        }

        let mut unnamed = attachment();
        unnamed.descriptor.filename = None;
        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"F123","mimetype":"text/plain","size":5,"url_private":"https://files.slack.com/files-pri/T-F/x"}}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"hello".to_vec(),
            }),
        ]);
        let fetched = SlackChannelAdapter
            .fetch_attachment(&unnamed, &egress)
            .await
            .expect("a provider file without a display name remains valid");
        assert_eq!(fetched.filename, None);
    }

    #[tokio::test]
    async fn outbound_attachment_uses_external_upload_and_verifies_the_destination() {
        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/ticket","file_id":"FNEW"}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"OK - 5".to_vec(),
            }),
            ScriptedEgress::ok(r#"{"ok":true,"files":[{"id":"FNEW","title":"report.txt"}]}"#),
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"FNEW","name":"report.txt","mimetype":"text/plain","size":5,"ims":["D123"],"shares":{"private":{"D123":[{"ts":"1710000001.000001","thread_ts":"1710000000.000100"}]}}}}"#,
            ),
        ]);

        let report = SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::File(workspace_file())], None),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert!(matches!(
            report.parts.as_slice(),
            [PartDeliveryOutcome::Sent {
                vendor_message_ref: Some(file_id)
            }] if file_id == "FNEW"
        ));
        let requests = egress.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, NetworkMethod::Get);
        assert_eq!(
            requests[0].url,
            "https://slack.com/api/files.getUploadURLExternal?filename=report.txt&length=5"
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.url.ends_with("/api/files.upload")),
            "the retired files.upload endpoint must never be used"
        );
        assert_eq!(requests[1].url, "https://files.slack.com/upload/v1/ticket");
        assert_eq!(requests[1].body.as_deref(), Some(b"hello".as_slice()));
        assert!(requests[1].credential.is_none());
        assert_eq!(
            requests[1].headers,
            vec![("content-type".to_string(), "text/plain".to_string())]
        );
        let completion = body_json(&requests[2]);
        assert_eq!(completion["channel_id"], "D123");
        assert_eq!(completion["thread_ts"], "1710000000.000100");
        assert_eq!(completion["files"][0]["id"], "FNEW");
        assert_eq!(completion["files"][0]["title"], "report.txt");
        assert_eq!(
            requests[3].url,
            "https://slack.com/api/files.info?file=FNEW"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn outbound_attachment_retries_eventually_consistent_destination_readback() {
        let mut responses = vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/ticket","file_id":"FNEW"}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"OK - 5".to_vec(),
            }),
            ScriptedEgress::ok(r#"{"ok":true,"files":[{"id":"FNEW","title":"report.txt"}]}"#),
        ];
        // Slack accepted the batch in production but needed longer than the
        // former six-attempt, 500 ms window to expose the DM share through
        // files.info. Keep the provider-visible upload one-shot while
        // tolerating that bounded propagation delay.
        responses.extend((0..6).map(|_| {
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"FNEW","name":"report.txt","mimetype":"text/plain","size":5}}"#,
            )
        }));
        responses.push(ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"FNEW","name":"report.txt","mimetype":"text/plain","size":5,"ims":["D123"],"shares":{"private":{"D123":[{"ts":"1710000001.000001","thread_ts":"1710000000.000100"}]}}}}"#,
        ));
        let egress = ScriptedEgress::new(responses);

        let report = SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::File(workspace_file())], None),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert!(matches!(
            report.parts.as_slice(),
            [PartDeliveryOutcome::Sent {
                vendor_message_ref: Some(file_id)
            }] if file_id == "FNEW"
        ));
        assert_eq!(
            egress.requests().len(),
            10,
            "destination propagation beyond the old retry window should be retried without re-uploading"
        );
        assert!(
            egress.requests()[3..]
                .iter()
                .all(|request| request.url.ends_with("/api/files.info?file=FNEW"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn outbound_attachment_readback_exhaustion_is_terminal_after_completion() {
        let mut responses = vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/ticket","file_id":"FNEW"}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"OK - 5".to_vec(),
            }),
            ScriptedEgress::ok(r#"{"ok":true,"files":[{"id":"FNEW","title":"report.txt"}]}"#),
        ];
        responses.extend(
            (0..crate::attachment_transfer::SLACK_FILE_READBACK_MAX_ATTEMPTS).map(|_| {
                Ok(RestrictedEgressResponse {
                    status: 503,
                    body: Vec::new(),
                })
            }),
        );
        let egress = ScriptedEgress::new(responses);

        let report = SlackChannelAdapter
            .deliver(
                envelope(vec![OutboundPart::File(workspace_file())], None),
                &egress,
            )
            .await
            .expect("completed upload with unavailable evidence is reported");

        assert!(matches!(
            report.parts.as_slice(),
            [PartDeliveryOutcome::Permanent { reason }]
                if reason.contains("read-back remained unavailable")
        ));
        let requests = egress.requests();
        assert_eq!(
            requests.len(),
            3 + crate::attachment_transfer::SLACK_FILE_READBACK_MAX_ATTEMPTS as usize
        );
        assert!(
            requests[3..]
                .iter()
                .all(|request| request.url.ends_with("/api/files.info?file=FNEW")),
            "only read-back is safe to retry after Slack accepted completion"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn outbound_attachment_readback_edge_cases_never_replay_the_upload() {
        let mut invalid_json = successful_upload_prefix();
        invalid_json.extend(
            (0..crate::attachment_transfer::SLACK_FILE_READBACK_MAX_ATTEMPTS)
                .map(|_| ScriptedEgress::ok("not-json")),
        );

        let mut missing_file = successful_upload_prefix();
        missing_file.extend(
            (0..crate::attachment_transfer::SLACK_FILE_READBACK_MAX_ATTEMPTS)
                .map(|_| ScriptedEgress::ok(r#"{"ok":true}"#)),
        );

        let mut file_not_found = successful_upload_prefix();
        file_not_found.extend(
            (0..crate::attachment_transfer::SLACK_FILE_READBACK_MAX_ATTEMPTS)
                .map(|_| ScriptedEgress::ok(r#"{"ok":false,"error":"file_not_found"}"#)),
        );

        let mut wrong_destination = successful_upload_prefix();
        wrong_destination.extend(
            (0..crate::attachment_transfer::SLACK_FILE_READBACK_MAX_ATTEMPTS).map(|_| {
                ScriptedEgress::ok(
                    r#"{"ok":true,"file":{"id":"FNEW","name":"report.txt","size":5,"ims":["DOTHER"]}}"#,
                )
            }),
        );

        let mut policy_denied = successful_upload_prefix();
        policy_denied.push(Err(RestrictedEgressError::PolicyDenied));

        for (responses, expected_reason) in [
            (invalid_json, "remained unavailable"),
            (missing_file, "remained unavailable"),
            (file_not_found, "file_not_found"),
            (wrong_destination, "did not match"),
            (policy_denied, "network policy"),
        ] {
            let egress = ScriptedEgress::new(responses);
            let report = SlackChannelAdapter
                .deliver(
                    envelope(vec![OutboundPart::File(workspace_file())], None),
                    &egress,
                )
                .await
                .expect("accepted upload returns a terminal read-back outcome");

            assert!(matches!(
                report.parts.as_slice(),
                [PartDeliveryOutcome::Permanent { reason }]
                    if reason.contains(expected_reason)
            ));
            let requests = egress.requests();
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.url.contains("getUploadURLExternal"))
                    .count(),
                1,
                "provider upload tickets are never replayed after completion"
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| request.url.contains("completeUploadExternal"))
                    .count(),
                1,
                "provider completion is never replayed during read-back"
            );
        }
    }

    #[tokio::test]
    async fn outbound_attachment_fails_closed_at_each_external_upload_stage() {
        let ticket = || {
            ScriptedEgress::ok(
                r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/ticket","file_id":"FNEW"}"#,
            )
        };
        let uploaded = || {
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"OK - 5".to_vec(),
            })
        };
        let cases = vec![
            (
                vec![Err(RestrictedEgressError::PolicyDenied)],
                1,
                "network policy",
            ),
            (vec![ScriptedEgress::status(503)], 1, "status 503"),
            (vec![ScriptedEgress::ok("not-json")], 1, "invalid response"),
            (
                vec![ScriptedEgress::ok(
                    r#"{"ok":false,"error":"internal_error"}"#,
                )],
                1,
                "internal_error",
            ),
            (
                vec![ScriptedEgress::ok(r#"{"ok":true,"file_id":"FNEW"}"#)],
                1,
                "omitted the upload URL",
            ),
            (
                vec![ScriptedEgress::ok(
                    r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/ticket","file_id":"not-valid"}"#,
                )],
                1,
                "valid file ID",
            ),
            (
                vec![ScriptedEgress::ok(
                    r#"{"ok":true,"upload_url":"https://evil.example/upload/v1/secret","file_id":"FNEW"}"#,
                )],
                1,
                "invalid upload URL",
            ),
            (
                vec![
                    ticket(),
                    Err(RestrictedEgressError::Transport {
                        reason: "timeout".to_string(),
                    }),
                ],
                2,
                "transport failed",
            ),
            (vec![ticket(), ScriptedEgress::status(503)], 2, "status 503"),
            (
                vec![
                    ticket(),
                    uploaded(),
                    Err(RestrictedEgressError::PolicyDenied),
                ],
                3,
                "network policy",
            ),
            (
                vec![ticket(), uploaded(), ScriptedEgress::status(503)],
                3,
                "status 503",
            ),
            (
                vec![ticket(), uploaded(), ScriptedEgress::ok("not-json")],
                3,
                "invalid response",
            ),
            (
                vec![
                    ticket(),
                    uploaded(),
                    ScriptedEgress::ok(r#"{"ok":false,"error":"missing_scope"}"#),
                ],
                3,
                "missing_scope",
            ),
            (
                vec![
                    ticket(),
                    uploaded(),
                    ScriptedEgress::ok(r#"{"ok":true,"files":[]}"#),
                ],
                3,
                "did not confirm",
            ),
            (
                vec![
                    ticket(),
                    uploaded(),
                    ScriptedEgress::ok(
                        r#"{"ok":true,"files":[{"id":"FNEW","title":"report.txt"}]}"#,
                    ),
                    ScriptedEgress::ok(
                        r#"{"ok":true,"file":{"id":"FOTHER","name":"report.txt","size":5,"ims":["D123"]}}"#,
                    ),
                ],
                4,
                "read-back",
            ),
        ];

        for (responses, expected_requests, expected_reason) in cases {
            let egress = ScriptedEgress::new(responses);
            let report = SlackChannelAdapter
                .deliver(
                    envelope(vec![OutboundPart::File(workspace_file())], None),
                    &egress,
                )
                .await
                .expect("deliver reports provider failure");
            assert_eq!(egress.requests().len(), expected_requests);
            assert!(matches!(
                report.parts.as_slice(),
                [PartDeliveryOutcome::Permanent { reason }
                    | PartDeliveryOutcome::Retryable { reason }
                    | PartDeliveryOutcome::Unauthorized { reason }]
                    if reason.contains(expected_reason)
            ));
            assert!(
                report
                    .parts
                    .iter()
                    .all(|outcome| !format!("{outcome:?}").contains("evil.example")),
                "upload URLs must not leak through outcomes"
            );
        }
    }

    #[tokio::test]
    async fn outbound_attachments_stage_all_files_before_one_ordered_completion() {
        let egress = ScriptedEgress::new(vec![
            ScriptedEgress::ok(
                r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/one","file_id":"FONE"}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"OK - 5".to_vec(),
            }),
            ScriptedEgress::ok(
                r#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/two","file_id":"FTWO"}"#,
            ),
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"OK - 3".to_vec(),
            }),
            ScriptedEgress::ok(r#"{"ok":true,"files":[{"id":"FONE"},{"id":"FTWO"}]}"#),
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"FONE","name":"report.txt","size":5,"ims":["D123"],"shares":{"private":{"D123":[{"thread_ts":"1710000000.000100"}]}}}}"#,
            ),
            ScriptedEgress::ok(
                r#"{"ok":true,"file":{"id":"FTWO","name":"report.csv","size":3,"ims":["D123"],"shares":{"private":{"D123":[{"thread_ts":"1710000000.000100"}]}}}}"#,
            ),
        ]);
        let second = WorkspaceFile {
            path: ScopedPath::new("/workspace/report.csv").expect("path"),
            filename: Some("report.csv".to_string()),
            mime_type: "text/csv".to_string(),
            bytes: b"a,b".to_vec(),
        };

        let report = SlackChannelAdapter
            .deliver(
                envelope(
                    vec![
                        OutboundPart::File(workspace_file()),
                        OutboundPart::File(second),
                    ],
                    None,
                ),
                &egress,
            )
            .await
            .expect("deliver drives");

        assert!(matches!(
            report.parts.as_slice(),
            [
                PartDeliveryOutcome::Sent {
                    vendor_message_ref: Some(first)
                },
                PartDeliveryOutcome::Sent {
                    vendor_message_ref: Some(second)
                }
            ] if first == "FONE" && second == "FTWO"
        ));
        let requests = egress.requests();
        assert_eq!(requests.len(), 7);
        assert!(requests[0].url.contains("filename=report.txt&length=5"));
        assert!(requests[2].url.contains("filename=report.csv&length=3"));
        let completion = body_json(&requests[4]);
        assert_eq!(completion["files"][0]["id"], "FONE");
        assert_eq!(completion["files"][0]["title"], "report.txt");
        assert_eq!(completion["files"][1]["id"], "FTWO");
        assert_eq!(completion["files"][1]["title"], "report.csv");
    }

    #[tokio::test]
    async fn outbound_zero_byte_attachment_fails_before_provider_egress() {
        let egress = ScriptedEgress::new(Vec::new());
        let empty = WorkspaceFile {
            path: ScopedPath::new("/workspace/empty.bin").expect("path"),
            filename: Some("empty.bin".to_string()),
            mime_type: "application/octet-stream".to_string(),
            bytes: Vec::new(),
        };

        let report = SlackChannelAdapter
            .deliver(envelope(vec![OutboundPart::File(empty)], None), &egress)
            .await
            .expect("unsupported empty file is a reported delivery outcome");

        assert!(matches!(
            report.parts.as_slice(),
            [PartDeliveryOutcome::Permanent { reason }]
                if reason.contains("empty")
        ));
        assert!(
            egress.requests().is_empty(),
            "Slack rejects zero-byte external upload tickets, so no provider request is safe"
        );
    }

    #[tokio::test]
    async fn outbound_attachment_rejects_unsafe_local_metadata_before_provider_egress() {
        let mut oversized = workspace_file();
        oversized.bytes =
            vec![0; crate::attachment_transfer::SLACK_MAX_TRANSFER_BYTES as usize + 1];

        let mut invalid_filename = workspace_file();
        invalid_filename.filename = Some("../secret.txt".to_string());

        let mut invalid_mime = workspace_file();
        invalid_mime.mime_type = "Text/Plain".to_string();

        for (file, expected_reason) in [
            (oversized, "size limit"),
            (invalid_filename, "filename"),
            (invalid_mime, "MIME type"),
        ] {
            let egress = ScriptedEgress::new(Vec::new());
            let report = SlackChannelAdapter
                .deliver(envelope(vec![OutboundPart::File(file)], None), &egress)
                .await
                .expect("unsafe local metadata is a reported delivery outcome");

            assert!(matches!(
                report.parts.as_slice(),
                [PartDeliveryOutcome::Permanent { reason }]
                    if reason.contains(expected_reason)
            ));
            assert!(
                egress.requests().is_empty(),
                "the complete local batch must validate before Slack receives a request"
            );
        }
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
