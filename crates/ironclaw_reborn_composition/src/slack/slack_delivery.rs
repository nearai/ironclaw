//! Slack's implementation of the adapter-generic delivery machinery's
//! vendor seam.
//!
//! The final-reply observer and triggered-run delivery driver live in
//! [`ironclaw_channel_delivery`]; this module supplies only the
//! Slack-native protocol details behind [`ChannelDeliveryProtocol`]: stored
//! `reply:` ref decoding, the `D…` DM-channel classification, and the
//! `chat.postMessage`/`chat.delete` Web API status messages.

use async_trait::async_trait;
use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, ProductAdapterError, ProtocolHttpEgress,
};
use ironclaw_turns::ReplyTargetBindingRef;
use serde::{Deserialize, Serialize};

use crate::slack::slack_outbound_targets::{
    slack_conversation_id_from_reply_target_binding_ref, slack_reply_target_is_personal_dm,
};
use ironclaw_channel_delivery::{
    ChannelDeliveryProtocol, FinalReplyDeliveryError, PostedChannelMessage,
};

const SLACK_API_HOST: &str = "slack.com";
const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
// Model B: greeting for a first-contact DM from a Slack user who has not yet
// connected their account. Fixed, host-authored text only — no agent runs.
const SLACK_CONNECT_NUDGE_MESSAGE: &str = "\u{1F44B} To use me, connect your Slack account in the Ironclaw web app: install the Slack extension and finish the connect step, then message me here again.";

/// Slack's [`ChannelDeliveryProtocol`]: `reply:` segment-ref decoding and
/// Web API status messages under the `slack_bot_token` egress handle.
#[derive(Debug, Default)]
pub(crate) struct SlackDeliveryProtocol;

#[async_trait]
impl ChannelDeliveryProtocol for SlackDeliveryProtocol {
    fn run_notification_projection_prefix(&self) -> &'static str {
        "slack"
    }

    fn conversation_id_from_reply_target_binding_ref(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<(String, Option<String>)> {
        slack_conversation_id_from_reply_target_binding_ref(target)
    }

    fn reply_target_is_personal_dm(&self, target: &ReplyTargetBindingRef) -> bool {
        slack_reply_target_is_personal_dm(target)
    }

    fn posted_message_from_render_response(
        &self,
        path: &str,
        _request_body: &[u8],
        response_body: &[u8],
    ) -> Option<PostedChannelMessage> {
        if path != "/api/chat.postMessage" {
            return None;
        }
        posted_slack_message_from_response(response_body)
    }

    fn connect_nudge_message(&self) -> &'static str {
        SLACK_CONNECT_NUDGE_MESSAGE
    }

    fn is_direct_message_conversation(&self, conversation_id: &str) -> bool {
        // Slack DM (im) channel ids start with 'D'; shared channels ('C') and
        // multi-person/group DMs ('G') are excluded, and a missing/blank id
        // fails closed.
        conversation_id.starts_with('D')
    }

    async fn post_status_message(
        &self,
        egress: &dyn ProtocolHttpEgress,
        conversation: &ironclaw_product_adapters::ExternalConversationRef,
        text: &str,
    ) -> Result<PostedChannelMessage, FinalReplyDeliveryError> {
        let body = ChatPostMessageRequest {
            channel: conversation.conversation_id(),
            text,
            mrkdwn: false,
            thread_ts: conversation.topic_id(),
        };
        let response = egress
            .send(slack_web_api_request(
                "/api/chat.postMessage",
                serde_json::to_vec(&body).map_err(|error| {
                    FinalReplyDeliveryError::StatusMessage {
                        reason: error.to_string(),
                    }
                })?,
            )?)
            .await
            .map_err(|error| FinalReplyDeliveryError::StatusMessage {
                reason: error.to_string(),
            })?;
        if !(200..300).contains(&response.status()) {
            return Err(FinalReplyDeliveryError::StatusMessage {
                reason: format!("Slack chat.postMessage returned HTTP {}", response.status()),
            });
        }
        let parsed: SlackMessageResponse =
            serde_json::from_slice(response.body()).map_err(|error| {
                FinalReplyDeliveryError::StatusMessage {
                    reason: format!("Slack chat.postMessage response was not JSON: {error}"),
                }
            })?;
        if !parsed.ok {
            return Err(FinalReplyDeliveryError::StatusMessage {
                reason: format!(
                    "Slack chat.postMessage failed: {}",
                    parsed.error.unwrap_or_else(|| "unknown_error".to_string())
                ),
            });
        }
        let Some(channel) = parsed.channel else {
            return Err(FinalReplyDeliveryError::StatusMessage {
                reason: "Slack chat.postMessage response missing channel".to_string(),
            });
        };
        let Some(ts) = parsed.ts else {
            return Err(FinalReplyDeliveryError::StatusMessage {
                reason: "Slack chat.postMessage response missing ts".to_string(),
            });
        };
        Ok(PostedChannelMessage {
            conversation_id: channel,
            message_ref: ts,
        })
    }

    async fn delete_status_message(
        &self,
        egress: &dyn ProtocolHttpEgress,
        message: &PostedChannelMessage,
    ) -> Result<(), FinalReplyDeliveryError> {
        let body = ChatDeleteRequest {
            channel: &message.conversation_id,
            ts: &message.message_ref,
        };
        let response = egress
            .send(slack_web_api_request(
                "/api/chat.delete",
                serde_json::to_vec(&body).map_err(|error| {
                    FinalReplyDeliveryError::StatusMessage {
                        reason: error.to_string(),
                    }
                })?,
            )?)
            .await
            .map_err(|error| FinalReplyDeliveryError::StatusMessage {
                reason: error.to_string(),
            })?;
        if !(200..300).contains(&response.status()) {
            return Err(FinalReplyDeliveryError::StatusMessage {
                reason: format!("Slack chat.delete returned HTTP {}", response.status()),
            });
        }
        let parsed: SlackMessageResponse =
            serde_json::from_slice(response.body()).map_err(|error| {
                FinalReplyDeliveryError::StatusMessage {
                    reason: format!("Slack chat.delete response was not JSON: {error}"),
                }
            })?;
        if !parsed.ok {
            return Err(FinalReplyDeliveryError::StatusMessage {
                reason: format!(
                    "Slack chat.delete failed: {}",
                    parsed.error.unwrap_or_else(|| "unknown_error".to_string())
                ),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct ChatPostMessageRequest<'a> {
    channel: &'a str,
    text: &'a str,
    mrkdwn: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ChatDeleteRequest<'a> {
    channel: &'a str,
    ts: &'a str,
}

#[derive(Debug, Deserialize)]
struct SlackMessageResponse {
    ok: bool,
    channel: Option<String>,
    ts: Option<String>,
    error: Option<String>,
}

fn slack_web_api_request(
    path: &'static str,
    body: Vec<u8>,
) -> Result<EgressRequest, ProductAdapterError> {
    Ok(EgressRequest::new(
        DeclaredEgressHost::new(SLACK_API_HOST)?,
        EgressMethod::post(),
        EgressPath::new(path)?,
    )
    .with_header(EgressHeader::new("content-type", "application/json")?)
    .with_body(body)
    .with_credential_handle(Some(EgressCredentialHandle::new(SLACK_BOT_TOKEN_HANDLE)?)))
}

fn posted_slack_message_from_response(body: &[u8]) -> Option<PostedChannelMessage> {
    let parsed: SlackMessageResponse = serde_json::from_slice(body).ok()?;
    if !parsed.ok {
        return None;
    }
    Some(PostedChannelMessage {
        conversation_id: parsed.channel?,
        message_ref: parsed.ts?,
    })
}
