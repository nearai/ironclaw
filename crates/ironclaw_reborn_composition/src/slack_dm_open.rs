//! Shared Slack conversations.open client for host-beta DM flows.

use ironclaw_product_adapters::{
    DeclaredEgressHost, EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath,
    EgressRequest, ProtocolHttpEgress,
};
use ironclaw_slack_v2_adapter::SLACK_API_HOST;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

const SLACK_CONVERSATIONS_OPEN_PATH: &str = "/api/conversations.open";
const SLACK_API_RESPONSE_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum SlackDmOpenError {
    #[error("Slack DM open failed: {0}")]
    Backend(String),
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
