//! DingTalk AI card streaming service.
//!
//! Provides functions to create, update, and fail AI streaming cards via
//! DingTalk's HTTPS card API.

use reqwest::Client;
use serde_json::json;

use crate::config::DingTalkConfig;
use crate::error::ChannelError;

use super::send;

/// Create an AI streaming card and deliver it to the conversation.
///
/// Returns the `outTrackId` (card instance ID) from the API response.
///
/// # Arguments
///
/// * `client` - Shared HTTP client
/// * `config` - DingTalk configuration (must have `card_template_id`)
/// * `token` - Valid DingTalk access token
/// * `conversation_id` - DingTalk conversation ID (used for group chats)
/// * `conversation_type` - `"1"` for DM, `"2"` for group
/// * `sender_staff_id` - Sender's staffId (used as openSpaceId for DM)
pub async fn create_ai_card(
    client: &Client,
    config: &DingTalkConfig,
    token: &str,
    conversation_id: &str,
    conversation_type: &str,
    sender_staff_id: &str,
) -> Result<String, ChannelError> {
    let card_template_id =
        config
            .card_template_id
            .as_deref()
            .ok_or_else(|| ChannelError::SendFailed {
                name: "dingtalk".into(),
                reason: "card_template_id is not configured (set DINGTALK_CARD_TEMPLATE_ID)".into(),
            })?;

    let is_group = conversation_type == "2";
    let robot_code = config.robot_code.as_deref().unwrap_or(&config.client_id);

    // Build openSpaceId per DingTalk spec:
    // - Group: dtv1.card//IM_GROUP.<conversationId>
    // - DM:    dtv1.card//IM_ROBOT.<userId>
    let open_space_id = if is_group {
        format!("dtv1.card//IM_GROUP.{conversation_id}")
    } else {
        format!("dtv1.card//IM_ROBOT.{sender_staff_id}")
    };

    // Generate a client-side tracking ID following the reference implementation
    // format: `card_<uuid>`. DingTalk uses this to correlate streaming updates.
    let out_track_id = format!("card_{}", uuid::Uuid::new_v4());

    // `config` must be a JSON *string*, not an object — DingTalk parses it on their side
    let card_data = json!({
        "cardParamMap": {
            "config": r#"{"autoLayout":true,"enableForward":true}"#,
            "content": "",
            "stop_action": "true"
        }
    });

    // Deliver model includes robotCode and extension; space model enables forwarding
    let body = if is_group {
        json!({
            "openSpaceId": open_space_id,
            "outTrackId": out_track_id,
            "callbackType": "STREAM",
            "cardTemplateId": card_template_id,
            "cardData": card_data,
            "userIdType": 1,
            "imGroupOpenSpaceModel": { "supportForward": true },
            "imGroupOpenDeliverModel": {
                "robotCode": robot_code,
                "extension": { "dynamicSummary": "true" }
            }
        })
    } else {
        json!({
            "openSpaceId": open_space_id,
            "outTrackId": out_track_id,
            "callbackType": "STREAM",
            "cardTemplateId": card_template_id,
            "cardData": card_data,
            "userIdType": 1,
            "imRobotOpenSpaceModel": { "supportForward": true },
            "imRobotOpenDeliverModel": {
                "spaceType": "IM_ROBOT",
                "robotCode": robot_code,
                "extension": { "dynamicSummary": "true" }
            }
        })
    };

    tracing::debug!(
        conversation_id = conversation_id,
        conversation_type = conversation_type,
        is_group = is_group,
        "Creating DingTalk AI card"
    );

    let resp = client
        .post(super::DingTalkChannel::api_url(
            "/v1.0/card/instances/createAndDeliver",
        ))
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("create card HTTP request failed: {e}"),
        })?;

    let resp_json = send::parse_business_response(resp, "createAndDeliver API")
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: e.to_string(),
        })?
        .unwrap_or_else(|| json!({}));

    // Prefer the server-returned outTrackId (if present and non-empty);
    // fall back to the client-generated one we sent in the request.
    let server_track_id = resp_json
        .get("result")
        .and_then(|r| r.get("outTrackId"))
        .or_else(|| resp_json.get("outTrackId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let out_track_id = server_track_id.unwrap_or(out_track_id);

    tracing::debug!(
        out_track_id = %out_track_id,
        "DingTalk AI card created"
    );

    // Kick the card from PROCESSING → INPUTING on the server side by sending
    // an empty streaming update immediately. Without this the card stays in
    // "正在思考..." and never renders content.
    stream_ai_card(
        client,
        token,
        &out_track_id,
        "",
        &config.card_template_key,
        false,
        false,
    )
    .await?;

    Ok(out_track_id)
}

/// Stream content to an existing AI card.
///
/// # Arguments
///
/// * `client` - Shared HTTP client
/// * `token` - Valid DingTalk access token
/// * `card_instance_id` - `outTrackId` returned by [`create_ai_card`]
/// * `content` - Full content to display (replaces previous content)
/// * `is_finalize` - Whether this is the last update
/// * `is_error` - Whether this update signals an error state
pub async fn stream_ai_card(
    client: &Client,
    token: &str,
    card_instance_id: &str,
    content: &str,
    content_key: &str,
    is_finalize: bool,
    is_error: bool,
) -> Result<(), ChannelError> {
    let body = json!({
        "outTrackId": card_instance_id,
        "guid": uuid::Uuid::new_v4().to_string(),
        "key": content_key,
        "content": content,
        "isFull": true,
        "isFinalize": is_finalize,
        "isError": is_error,
    });

    tracing::debug!(
        card_instance_id = card_instance_id,
        content_len = content.len(),
        is_finalize = is_finalize,
        is_error = is_error,
        "Streaming DingTalk AI card update"
    );

    let resp = client
        .put(super::DingTalkChannel::api_url("/v1.0/card/streaming"))
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("stream card HTTP request failed: {e}"),
        })?;

    send::ensure_business_success(resp, "card streaming API")
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: e.to_string(),
        })
}

/// Finalize an AI card: send final content and hide the stop button.
///
/// Calls [`stream_ai_card`] with `is_finalize: true`, then updates card
/// variables to set `stop_action` to `"false"`.
pub async fn finalize_ai_card(
    client: &Client,
    config: &DingTalkConfig,
    token: &str,
    card_instance_id: &str,
    content: &str,
) -> Result<(), ChannelError> {
    // Send final streaming update
    stream_ai_card(
        client,
        token,
        card_instance_id,
        content,
        &config.card_template_key,
        true,
        false,
    )
    .await?;

    // Hide stop button after finalization
    hide_stop_button(client, token, card_instance_id).await
}

/// Mark an AI card as failed with an error message.
///
/// Calls [`stream_ai_card`] with `is_error: true` and `is_finalize: true`,
/// then hides the stop button.
#[allow(dead_code)]
pub async fn fail_ai_card(
    client: &Client,
    config: &DingTalkConfig,
    token: &str,
    card_instance_id: &str,
    error_message: &str,
) -> Result<(), ChannelError> {
    let content = format!("⚠️ {error_message}");

    tracing::debug!(
        card_instance_id = card_instance_id,
        "Failing DingTalk AI card"
    );

    stream_ai_card(
        client,
        token,
        card_instance_id,
        &content,
        &config.card_template_key,
        true,
        true,
    )
    .await?;

    // Hide stop button after failure too
    hide_stop_button(client, token, card_instance_id).await
}

/// Hide the stop button on a card by updating `stop_action` to `"false"`.
async fn hide_stop_button(
    client: &Client,
    token: &str,
    card_instance_id: &str,
) -> Result<(), ChannelError> {
    let body = json!({
        "outTrackId": card_instance_id,
        "cardData": {
            "cardParamMap": {
                "stop_action": "false"
            }
        },
        "cardUpdateOptions": {
            "updateCardDataByKey": true,
            "updatePrivateDataByKey": true
        }
    });

    let resp = client
        .put(super::DingTalkChannel::api_url("/v1.0/card/instances"))
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("hide stop button HTTP request failed: {e}"),
        })?;

    if let Err(e) = send::ensure_business_success(resp, "hide stop button").await {
        tracing::debug!(error = %e, "Failed to hide stop button (non-fatal)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DingTalkConfig;
    use secrecy::SecretString;

    fn make_config(template_id: Option<&str>) -> DingTalkConfig {
        DingTalkConfig {
            enabled: true,
            client_id: "test-client".to_string(),
            client_secret: SecretString::from("secret"),
            robot_code: None,
            message_type: Default::default(),
            card_template_id: template_id.map(|s| s.to_string()),
            card_template_key: "content".to_string(),
            card_stream_mode: Default::default(),
            card_stream_interval_ms: 1000,
            ack_reaction: None,
            require_mention: false,
            dm_policy: Default::default(),
            group_policy: Default::default(),
            allow_from: vec![],
            group_allow_from: vec![],
            group_session_scope: Default::default(),
            display_name_resolution: Default::default(),
            max_reconnect_cycles: 10,
            reconnect_deadline_ms: 50000,
            additional_accounts: vec![],
        }
    }

    #[test]
    fn create_ai_card_missing_template_returns_error() {
        let config = make_config(None);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = Client::new();
        let err = rt.block_on(create_ai_card(
            &client,
            &config,
            "fake-token",
            "conv-123",
            "1",
            "user-456",
        ));
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("card_template_id"),
            "expected card_template_id mention, got: {msg}"
        );
    }

    #[test]
    fn fail_ai_card_content_format() {
        // Verify error message gets the warning prefix
        let error_msg = "connection timed out";
        let expected_content = format!("⚠️ {error_msg}");
        assert!(expected_content.contains("⚠️"));
        assert!(expected_content.contains(error_msg));
    }
}
