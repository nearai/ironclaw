//! Shared Slack conversations.open client for host-beta DM flows.

use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, ProtocolHttpEgress,
};
use ironclaw_slack_v2_adapter::SLACK_API_HOST;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

const SLACK_CONVERSATIONS_OPEN_PATH: &str = "/api/conversations.open";
pub(crate) const SLACK_API_RESPONSE_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum SlackDmOpenError {
    #[error("Slack DM open failed: {0}")]
    Backend(String),
    #[error("Slack conversations.open response did not return a direct-message channel")]
    InvalidChannel,
    #[error("Slack conversations.open response did not include a channel id")]
    MissingChannel,
}

pub(crate) async fn open_slack_dm_channel(
    egress: &dyn ProtocolHttpEgress,
    credential_handle: EgressCredentialHandle,
    slack_user_id: &str,
) -> Result<String, SlackDmOpenError> {
    let body = serde_json::to_vec(&SlackConversationsOpenRequest {
        users: slack_user_id.to_string(),
    })
    .map_err(|error| SlackDmOpenError::Backend(error.to_string()))?;
    let request = slack_api_request(SLACK_CONVERSATIONS_OPEN_PATH, body, credential_handle)?;
    let response = egress
        .send(request)
        .await
        .map_err(|error| SlackDmOpenError::Backend(error.to_string()))?;
    if !(200..300).contains(&response.status()) {
        return Err(SlackDmOpenError::Backend(format!(
            "Slack API request {SLACK_CONVERSATIONS_OPEN_PATH} failed with HTTP {}",
            response.status()
        )));
    }
    let opened: SlackConversationsOpenResponse =
        slack_json_response("Slack conversations.open", response.body())?;
    if !opened.ok {
        return Err(SlackDmOpenError::Backend(format!(
            "Slack rejected conversations.open ({})",
            opened.error.unwrap_or_else(|| "unknown_error".into())
        )));
    }
    opened
        .channel
        .map(|channel| channel.id)
        .filter(|id| !id.is_empty())
        .ok_or(SlackDmOpenError::MissingChannel)
}

pub(crate) fn validate_slack_dm_channel_id(value: &str) -> Result<(), SlackDmOpenError> {
    if value.is_empty()
        || value.len() > 128
        || value.chars().any(|c| {
            c == '\0' || c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\' | ':' | ';')
        })
    {
        return Err(SlackDmOpenError::InvalidChannel);
    }
    if !value.starts_with('D') {
        return Err(SlackDmOpenError::InvalidChannel);
    }
    Ok(())
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

fn slack_api_request(
    path: &'static str,
    body: Vec<u8>,
    credential_handle: EgressCredentialHandle,
) -> Result<EgressRequest, SlackDmOpenError> {
    let host = DeclaredEgressHost::new(SLACK_API_HOST)
        .map_err(|error| SlackDmOpenError::Backend(error.to_string()))?;
    let method = EgressMethod::post();
    let path =
        EgressPath::new(path).map_err(|error| SlackDmOpenError::Backend(error.to_string()))?;
    let content_type = EgressHeader::new("content-type", "application/json")
        .map_err(|error| SlackDmOpenError::Backend(error.to_string()))?;
    Ok(EgressRequest::new(host, method, path)
        .with_header(content_type)
        .with_body(body)
        .with_credential_handle(Some(credential_handle)))
}

fn slack_json_response<T>(label: &'static str, body: &[u8]) -> Result<T, SlackDmOpenError>
where
    T: DeserializeOwned,
{
    if body.len() > SLACK_API_RESPONSE_LIMIT {
        return Err(SlackDmOpenError::Backend(format!(
            "{label} response exceeded body limit"
        )));
    }
    serde_json::from_slice(body).map_err(|error| {
        SlackDmOpenError::Backend(format!("{label} response was invalid JSON: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironclaw_product_adapters::{EgressResponse, ProtocolHttpEgressError};

    use super::*;

    #[tokio::test]
    async fn open_slack_dm_channel_returns_missing_channel_when_id_empty() {
        for body in [
            br#"{"ok":true}"#.to_vec(),
            br#"{"ok":true,"channel":{"id":""}}"#.to_vec(),
        ] {
            let error = open_slack_dm_channel(
                &ScriptedEgress::new(EgressResponse::new(200, body)),
                credential_handle(),
                "U123",
            )
            .await
            .expect_err("missing channel fails");

            assert!(matches!(error, SlackDmOpenError::MissingChannel));
        }
    }

    #[tokio::test]
    async fn open_slack_dm_channel_fails_on_non_2xx() {
        let error = open_slack_dm_channel(
            &ScriptedEgress::new(EgressResponse::new(503, b"unavailable".to_vec())),
            credential_handle(),
            "U123",
        )
        .await
        .expect_err("non-2xx fails");

        assert!(matches!(error, SlackDmOpenError::Backend(_)));
    }

    #[tokio::test]
    async fn open_slack_dm_channel_fails_on_oversized_body() {
        let error = open_slack_dm_channel(
            &ScriptedEgress::new(EgressResponse::new(
                200,
                vec![b'{'; SLACK_API_RESPONSE_LIMIT + 1],
            )),
            credential_handle(),
            "U123",
        )
        .await
        .expect_err("oversized body fails");

        assert!(matches!(error, SlackDmOpenError::Backend(_)));
    }

    #[tokio::test]
    async fn open_slack_dm_channel_returns_channel_id_on_success() {
        let channel_id = open_slack_dm_channel(
            &ScriptedEgress::new(EgressResponse::new(
                200,
                br#"{"ok":true,"channel":{"id":"D123ABCD"}}"#.to_vec(),
            )),
            credential_handle(),
            "U123",
        )
        .await
        .expect("happy path succeeds");

        assert_eq!(channel_id, "D123ABCD");
    }

    #[test]
    fn validate_slack_dm_channel_id_rejects_invalid_shapes() {
        for value in ["", "C123", "G123", "D 123", "D/123", "D\x01123"] {
            assert!(matches!(
                validate_slack_dm_channel_id(value),
                Err(SlackDmOpenError::InvalidChannel)
            ));
        }
        assert!(matches!(
            validate_slack_dm_channel_id(&format!("D{}", "1".repeat(128))),
            Err(SlackDmOpenError::InvalidChannel)
        ));
        validate_slack_dm_channel_id("D123").expect("valid DM channel");
    }

    #[test]
    fn validate_slack_dm_channel_id_accepts_exactly_128_bytes_and_rejects_129() {
        // Boundary: "D" + 127 ASCII chars = 128 bytes total — must be accepted.
        let at_limit = format!("D{}", "X".repeat(127));
        assert_eq!(at_limit.len(), 128, "fixture must be exactly 128 bytes");
        validate_slack_dm_channel_id(&at_limit).expect("exactly 128-byte id must be valid");

        // One byte over the limit — must be rejected.
        let over_limit = format!("D{}", "X".repeat(128));
        assert_eq!(over_limit.len(), 129, "fixture must be exactly 129 bytes");
        assert!(
            matches!(
                validate_slack_dm_channel_id(&over_limit),
                Err(SlackDmOpenError::InvalidChannel)
            ),
            "129-byte id must be rejected"
        );
    }

    fn credential_handle() -> EgressCredentialHandle {
        EgressCredentialHandle::new("slack_bot_token").expect("credential handle")
    }

    #[derive(Debug)]
    struct ScriptedEgress {
        response: Mutex<Option<EgressResponse>>,
    }

    impl ScriptedEgress {
        fn new(response: EgressResponse) -> Self {
            Self {
                response: Mutex::new(Some(response)),
            }
        }
    }

    #[async_trait::async_trait]
    impl ProtocolHttpEgress for ScriptedEgress {
        async fn send(
            &self,
            _request: EgressRequest,
        ) -> Result<EgressResponse, ProtocolHttpEgressError> {
            self.response
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .ok_or(ProtocolHttpEgressError::Timeout)
        }
    }
}
