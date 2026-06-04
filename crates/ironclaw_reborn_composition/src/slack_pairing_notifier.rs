//! Slack API notifier for personal-binding pairing challenges.

use std::sync::Arc;

use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, ProtocolHttpEgress,
};
use ironclaw_slack_v2_adapter::SLACK_API_HOST;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::slack_personal_binding_pairing::{
    SlackPersonalBindingPairingError, SlackPersonalBindingPairingNotification,
    SlackPersonalBindingPairingNotifier,
};

const SLACK_CONVERSATIONS_OPEN_PATH: &str = "/api/conversations.open";
const SLACK_POST_MESSAGE_PATH: &str = "/api/chat.postMessage";
const SLACK_API_RESPONSE_LIMIT: usize = 64 * 1024;

pub(crate) struct SlackPairingChallengeHttpNotifier {
    egress: Arc<dyn ProtocolHttpEgress>,
    credential_handle: EgressCredentialHandle,
}

impl SlackPairingChallengeHttpNotifier {
    pub(crate) fn new(
        egress: Arc<dyn ProtocolHttpEgress>,
        credential_handle: EgressCredentialHandle,
    ) -> Self {
        Self {
            egress,
            credential_handle,
        }
    }
}

#[async_trait::async_trait]
impl SlackPersonalBindingPairingNotifier for SlackPairingChallengeHttpNotifier {
    async fn send_pairing_challenge(
        &self,
        notification: SlackPersonalBindingPairingNotification,
    ) -> Result<(), SlackPersonalBindingPairingError> {
        let channel = self
            .open_dm_channel(notification.slack_user_id.as_str())
            .await?;
        let body = serde_json::to_vec(&SlackPairingPostMessage {
            channel,
            text: format!(
                "Connect this Slack account to Ironclaw by entering code {} in WebChat.",
                notification.code.as_str()
            ),
            mrkdwn: false,
        })
        .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?;
        let response = self
            .send_slack_request(SLACK_POST_MESSAGE_PATH, body)
            .await?;
        slack_ok_response("Slack pairing DM", response.body())?;
        Ok(())
    }
}

impl SlackPairingChallengeHttpNotifier {
    async fn open_dm_channel(
        &self,
        slack_user_id: &str,
    ) -> Result<String, SlackPersonalBindingPairingError> {
        let body = serde_json::to_vec(&SlackConversationsOpenRequest {
            users: slack_user_id.to_string(),
        })
        .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?;
        let response = self
            .send_slack_request(SLACK_CONVERSATIONS_OPEN_PATH, body)
            .await?;
        let opened: SlackConversationsOpenResponse =
            slack_json_response("Slack conversations.open", response.body())?;
        if !opened.ok {
            return Err(SlackPersonalBindingPairingError::Backend(format!(
                "Slack rejected conversations.open ({})",
                opened.error.unwrap_or_else(|| "unknown_error".into())
            )));
        }
        opened
            .channel
            .map(|channel| channel.id)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                SlackPersonalBindingPairingError::Backend(
                    "Slack conversations.open response did not include a channel id".into(),
                )
            })
    }

    async fn send_slack_request(
        &self,
        path: &'static str,
        body: Vec<u8>,
    ) -> Result<ironclaw_product_adapters::EgressResponse, SlackPersonalBindingPairingError> {
        let response = self
            .egress
            .send(slack_api_request(
                path,
                body,
                self.credential_handle.clone(),
            ))
            .await
            .map_err(|error| SlackPersonalBindingPairingError::Backend(error.to_string()))?;
        if !(200..300).contains(&response.status()) {
            return Err(SlackPersonalBindingPairingError::Backend(format!(
                "Slack API request {path} failed with HTTP {}",
                response.status()
            )));
        }
        Ok(response)
    }
}

#[derive(Debug, Serialize)]
struct SlackConversationsOpenRequest {
    users: String,
}

#[derive(Debug, Deserialize)]
struct SlackConversationsOpenResponse {
    ok: bool,
    error: Option<String>,
    channel: Option<SlackConversationsOpenChannel>,
}

#[derive(Debug, Deserialize)]
struct SlackConversationsOpenChannel {
    id: String,
}

#[derive(Debug, Serialize)]
struct SlackPairingPostMessage {
    channel: String,
    text: String,
    mrkdwn: bool,
}

#[derive(Debug, Deserialize)]
struct SlackApiResponse {
    ok: bool,
    error: Option<String>,
}

fn slack_api_request(
    path: &'static str,
    body: Vec<u8>,
    credential_handle: EgressCredentialHandle,
) -> EgressRequest {
    let host = DeclaredEgressHost::new(SLACK_API_HOST).expect("static Slack host valid");
    let method = EgressMethod::post();
    let path = EgressPath::new(path).expect("static Slack API path valid");
    let content_type = EgressHeader::new("content-type", "application/json")
        .expect("static content-type header valid");
    EgressRequest::new(host, method, path)
        .with_header(content_type)
        .with_body(body)
        .with_credential_handle(Some(credential_handle))
}

fn slack_json_response<T>(
    label: &'static str,
    body: &[u8],
) -> Result<T, SlackPersonalBindingPairingError>
where
    T: DeserializeOwned,
{
    if body.len() > SLACK_API_RESPONSE_LIMIT {
        return Err(SlackPersonalBindingPairingError::Backend(format!(
            "{label} response exceeded body limit"
        )));
    }
    serde_json::from_slice(body).map_err(|error| {
        SlackPersonalBindingPairingError::Backend(format!(
            "{label} response was invalid JSON: {error}"
        ))
    })
}

fn slack_ok_response(
    label: &'static str,
    body: &[u8],
) -> Result<(), SlackPersonalBindingPairingError> {
    let response: SlackApiResponse = slack_json_response(label, body)?;
    if response.ok {
        Ok(())
    } else {
        Err(SlackPersonalBindingPairingError::Backend(format!(
            "Slack rejected {label} ({})",
            response.error.unwrap_or_else(|| "unknown_error".into())
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use ironclaw_product_adapters::{
        AdapterInstallationId, EgressResponse, ProtocolHttpEgressError,
    };

    use super::*;
    use crate::slack_personal_binding_pairing::SlackPersonalBindingPairingCode;
    use crate::slack_serve::SlackUserId;

    #[tokio::test]
    async fn slack_pairing_notifier_posts_code_to_slack_user() {
        let egress = Arc::new(RecordingEgress::default());
        let notifier = SlackPairingChallengeHttpNotifier::new(
            egress.clone(),
            EgressCredentialHandle::new("slack_bot_token").unwrap(),
        );

        notifier
            .send_pairing_challenge(notification())
            .await
            .expect("notification succeeds");

        let calls = egress.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].path().as_str(), SLACK_CONVERSATIONS_OPEN_PATH);
        let open_body: serde_json::Value = serde_json::from_slice(calls[0].body()).unwrap();
        assert_eq!(open_body["users"], "U123");
        assert_eq!(calls[1].path().as_str(), SLACK_POST_MESSAGE_PATH);
        let post_body: serde_json::Value = serde_json::from_slice(calls[1].body()).unwrap();
        assert_eq!(post_body["channel"], "D123");
        assert!(post_body["text"].as_str().unwrap().contains("ABCD1234"));
    }

    #[tokio::test]
    async fn slack_pairing_notifier_maps_slack_api_failures_to_backend_errors() {
        let rejected_open = SlackPairingChallengeHttpNotifier::new(
            Arc::new(ScriptedEgress::new([EgressResponse::new(
                200,
                br#"{"ok":false,"error":"not_allowed"}"#.to_vec(),
            )])),
            EgressCredentialHandle::new("slack_bot_token").unwrap(),
        );
        assert!(matches!(
            rejected_open.send_pairing_challenge(notification()).await,
            Err(SlackPersonalBindingPairingError::Backend(_))
        ));

        let rejected_post = SlackPairingChallengeHttpNotifier::new(
            Arc::new(ScriptedEgress::new([
                EgressResponse::new(200, br#"{"ok":true,"channel":{"id":"D123"}}"#.to_vec()),
                EgressResponse::new(200, br#"{"ok":false,"error":"channel_not_found"}"#.to_vec()),
            ])),
            EgressCredentialHandle::new("slack_bot_token").unwrap(),
        );
        assert!(matches!(
            rejected_post.send_pairing_challenge(notification()).await,
            Err(SlackPersonalBindingPairingError::Backend(_))
        ));
    }

    fn notification() -> SlackPersonalBindingPairingNotification {
        SlackPersonalBindingPairingNotification {
            installation_id: AdapterInstallationId::new("install-alpha").unwrap(),
            slack_user_id: SlackUserId::new("U123"),
            code: SlackPersonalBindingPairingCode::new("ABCD1234").unwrap(),
        }
    }

    #[derive(Default)]
    struct RecordingEgress {
        calls: Mutex<Vec<EgressRequest>>,
    }

    impl RecordingEgress {
        fn calls(&self) -> Vec<EgressRequest> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl ProtocolHttpEgress for RecordingEgress {
        async fn send(
            &self,
            request: EgressRequest,
        ) -> Result<EgressResponse, ProtocolHttpEgressError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            let response = match self.calls().last().map(|request| request.path().as_str()) {
                Some(SLACK_CONVERSATIONS_OPEN_PATH) => {
                    br#"{"ok":true,"channel":{"id":"D123"}}"#.to_vec()
                }
                _ => br#"{"ok":true}"#.to_vec(),
            };
            Ok(EgressResponse::new(200, response))
        }
    }

    struct ScriptedEgress {
        responses: Mutex<VecDeque<EgressResponse>>,
    }

    impl ScriptedEgress {
        fn new(responses: impl IntoIterator<Item = EgressResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProtocolHttpEgress for ScriptedEgress {
        async fn send(
            &self,
            _request: EgressRequest,
        ) -> Result<EgressResponse, ProtocolHttpEgressError> {
            self.responses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
                .ok_or(ProtocolHttpEgressError::Timeout)
        }
    }
}
