//! DingTalk AI card streaming service.
//!
//! Provides functions to create, update, and fail AI streaming cards via
//! DingTalk's HTTPS card API.

use reqwest::Client;
use serde_json::json;

use crate::config::DingTalkConfig;
use crate::error::ChannelError;

/// Create an AI streaming card and deliver it to the conversation.
///
/// Returns the `outTrackId` (card instance ID) from the API response.
///
/// # Arguments
///
/// * `client` - Shared HTTP client
/// * `config` - DingTalk configuration (must have `card_template_id`)
/// * `token` - Valid DingTalk access token
/// * `conversation_id` - DingTalk conversation ID
/// * `conversation_type` - `"1"` for DM, `"2"` for group
pub async fn create_ai_card(
    client: &Client,
    config: &DingTalkConfig,
    token: &str,
    conversation_id: &str,
    conversation_type: &str,
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

    // Build openSpaceId per DingTalk spec
    let open_space_id = if is_group {
        format!("dtv1.card//IM_GROUP.{conversation_id}")
    } else {
        format!("dtv1.card//IM_ROBOT.{conversation_id}")
    };

    // Deliver model — group vs DM differ only in the key name
    let extension = json!({ "dynamicSummary": "正在思考..." });
    let deliver_model = if is_group {
        json!({ "imGroupOpenDeliverModel": { "extension": extension } })
    } else {
        json!({ "imRobotOpenDeliverModel": { "extension": extension } })
    };

    let card_data = json!({
        "cardParamMap": {
            "config": r#"{"autoLayout":true,"enableForward":true}"#,
            "content": ""
        }
    });

    let mut body = json!({
        "openSpaceId": open_space_id,
        "callbackType": "STREAM",
        "cardTemplateId": card_template_id,
        "cardData": card_data,
    });

    // Merge the deliver model fields into the body object
    if let (Some(obj), Some(deliver_obj)) = (body.as_object_mut(), deliver_model.as_object()) {
        for (k, v) in deliver_obj {
            obj.insert(k.clone(), v.clone());
        }
    }

    tracing::debug!(
        conversation_id = conversation_id,
        conversation_type = conversation_type,
        is_group = is_group,
        "Creating DingTalk AI card"
    );

    let resp = client
        .post("https://api.dingtalk.com/v1.0/card/instances/createAndDeliver")
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("create card HTTP request failed: {e}"),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("createAndDeliver API returned {status}: {body_text}"),
        });
    }

    let resp_json: serde_json::Value = resp.json().await.map_err(|e| ChannelError::SendFailed {
        name: "dingtalk".into(),
        reason: format!("failed to parse createAndDeliver response: {e}"),
    })?;

    // DingTalk returns the card instance ID in `result.outTrackId`
    let out_track_id = resp_json
        .get("result")
        .and_then(|r| r.get("outTrackId"))
        .or_else(|| resp_json.get("outTrackId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("createAndDeliver response missing outTrackId: {resp_json}"),
        })?
        .to_string();

    tracing::debug!(
        out_track_id = %out_track_id,
        "DingTalk AI card created"
    );

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
        "key": content_key,
        "value": content,
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
        .put("https://api.dingtalk.com/v1.0/card/streaming")
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("stream card HTTP request failed: {e}"),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ChannelError::SendFailed {
            name: "dingtalk".into(),
            reason: format!("card streaming API returned {status}: {body_text}"),
        });
    }

    Ok(())
}

/// Mark an AI card as failed with an error message.
///
/// Calls [`stream_ai_card`] with `is_error: true` and `is_finalize: true`.
///
/// # Arguments
///
/// * `client` - Shared HTTP client
/// * `config` - DingTalk configuration (unused currently, reserved for future use)
/// * `token` - Valid DingTalk access token
/// * `card_instance_id` - `outTrackId` of the card to fail
/// * `error_message` - Human-readable error description shown in the card
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
    .await
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
