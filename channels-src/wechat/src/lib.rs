wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

mod api;
mod auth;
mod state;
mod types;

use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};
use serde_json::json;

use crate::auth::TOKEN_SECRET_NAME;
use crate::state::{
    clear_session_expired, load_config, load_context_tokens, load_get_updates_buf,
    load_typing_tickets, mark_session_expired, persist_config, persist_context_tokens,
    persist_get_updates_buf, persist_typing_tickets, session_expired, TypingTicketEntry,
};
use crate::types::{
    OutboundMetadata, WechatConfig, WechatMessage, MESSAGE_ITEM_TEXT, MESSAGE_TYPE_USER,
    TYPING_STATUS_CANCEL, TYPING_STATUS_TYPING,
};

const TYPING_TICKET_TTL_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WechatStatusAction {
    Typing,
    Cancel,
}

struct WechatChannel;

impl Guest for WechatChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config = serde_json::from_str::<WechatConfig>(&config_json)
            .map_err(|e| format!("Failed to parse WeChat config: {e}"))?;
        persist_config(&config)?;
        clear_session_expired();

        Ok(ChannelConfig {
            display_name: "WeChat".to_string(),
            http_endpoints: Vec::new(),
            poll: Some(PollConfig {
                interval_ms: config.poll_interval_ms.max(30_000),
                enabled: true,
            }),
        })
    }

    fn on_http_request(
        _req: exports::near::agent::channel::IncomingHttpRequest,
    ) -> exports::near::agent::channel::OutgoingHttpResponse {
        exports::near::agent::channel::OutgoingHttpResponse {
            status: 404,
            headers_json: "{}".to_string(),
            body: b"{\"error\":\"wechat channel does not expose webhooks\"}".to_vec(),
        }
    }

    fn on_poll() {
        if session_expired() {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "WeChat session is marked expired; reconnect the channel to resume polling",
            );
            return;
        }

        if !channel_host::secret_exists(TOKEN_SECRET_NAME) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "WeChat bot token is missing; skipping poll",
            );
            return;
        }

        let config = load_config();
        let cursor = load_get_updates_buf();
        let mut context_tokens = load_context_tokens();

        match api::get_updates(&config, &cursor) {
            Ok(response) => {
                if response.errcode == Some(-14) {
                    mark_session_expired();
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        "WeChat session expired; reconnect the channel",
                    );
                    return;
                }

                if response.ret.unwrap_or(0) != 0 {
                    let errmsg = response
                        .errmsg
                        .as_deref()
                        .unwrap_or("unknown WeChat polling error");
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "WeChat getUpdates returned ret={} errmsg={errmsg}",
                            response.ret.unwrap_or(-1)
                        ),
                    );
                }

                if let Some(next_cursor) = response.get_updates_buf.as_deref() {
                    if next_cursor != cursor {
                        if let Err(error) = persist_get_updates_buf(next_cursor) {
                            channel_host::log(
                                channel_host::LogLevel::Warn,
                                &format!("Failed to persist WeChat polling cursor: {error}"),
                            );
                        }
                    }
                }

                let mut context_tokens_changed = false;
                for message in response.msgs {
                    if let Some(from_user_id) = message.from_user_id.as_deref() {
                        if let Some(context_token) = message.context_token.as_deref() {
                            let changed = context_tokens
                                .insert(from_user_id.to_string(), context_token.to_string())
                                .as_deref()
                                != Some(context_token);
                            context_tokens_changed |= changed;
                        }
                    }
                    emit_incoming_message(message);
                }

                if context_tokens_changed {
                    if let Err(error) = persist_context_tokens(&context_tokens) {
                        channel_host::log(
                            channel_host::LogLevel::Warn,
                            &format!("Failed to persist WeChat context tokens: {error}"),
                        );
                    }
                }
            }
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("WeChat polling failed: {error}"),
                );
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata = serde_json::from_str::<OutboundMetadata>(&response.metadata_json)
            .map_err(|e| format!("Invalid WeChat response metadata: {e}"))?;
        let config = load_config();
        let context_tokens = load_context_tokens();
        let context_token = metadata
            .context_token
            .clone()
            .or_else(|| context_tokens.get(&metadata.from_user_id).cloned());
        if let Err(error) = send_typing_indicator(
            &config,
            &metadata,
            context_token.as_deref(),
            TYPING_STATUS_CANCEL,
            false,
        ) {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Failed to cancel WeChat typing indicator before reply: {error}"),
            );
        }

        api::send_text_message(
            &config,
            &metadata.from_user_id,
            response.content.trim(),
            context_token.as_deref(),
        )
    }

    fn on_status(update: StatusUpdate) {
        let Some(action) = classify_status_update(&update) else {
            return;
        };
        let metadata = match serde_json::from_str::<OutboundMetadata>(&update.metadata_json) {
            Ok(metadata) => metadata,
            Err(_) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    "on_status: no valid WeChat metadata, skipping typing update",
                );
                return;
            }
        };
        let config = load_config();
        let context_tokens = load_context_tokens();
        let context_token = resolve_context_token(&metadata, &context_tokens);

        let (typing_status, allow_ticket_fetch) = match action {
            WechatStatusAction::Typing => (TYPING_STATUS_TYPING, true),
            WechatStatusAction::Cancel => (TYPING_STATUS_CANCEL, false),
        };

        if let Err(error) = send_typing_indicator(
            &config,
            &metadata,
            context_token.as_deref(),
            typing_status,
            allow_ticket_fetch,
        ) {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("WeChat typing update failed: {error}"),
            );
        }
    }

    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
        Ok(())
    }

    fn on_shutdown() {}
}

fn emit_incoming_message(message: WechatMessage) {
    if message.message_type != Some(MESSAGE_TYPE_USER) {
        return;
    }

    let Some(from_user_id) = message.from_user_id.as_deref() else {
        return;
    };

    let text = extract_text(&message);
    if text.trim().is_empty() {
        return;
    }

    let metadata = json!({
        "from_user_id": from_user_id,
        "to_user_id": message.to_user_id,
        "message_id": message.message_id,
        "session_id": message.session_id,
        "context_token": message.context_token,
    });

    channel_host::emit_message(&EmittedMessage {
        user_id: from_user_id.to_string(),
        user_name: None,
        content: text,
        thread_id: Some(format!("wechat:{from_user_id}")),
        metadata_json: metadata.to_string(),
        attachments: Vec::new(),
    });
}

fn extract_text(message: &WechatMessage) -> String {
    message
        .item_list
        .iter()
        .find_map(|item| {
            if item.r#type == Some(MESSAGE_ITEM_TEXT) {
                item.text_item.as_ref().map(|item| item.text.clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn is_terminal_text_status(message: &str) -> bool {
    let trimmed = message.trim();
    trimmed.eq_ignore_ascii_case("done")
        || trimmed.eq_ignore_ascii_case("interrupted")
        || trimmed.eq_ignore_ascii_case("awaiting approval")
        || trimmed.eq_ignore_ascii_case("rejected")
}

fn classify_status_update(update: &StatusUpdate) -> Option<WechatStatusAction> {
    match update.status {
        StatusType::Thinking => Some(WechatStatusAction::Typing),
        StatusType::Done
        | StatusType::Interrupted
        | StatusType::ApprovalNeeded
        | StatusType::AuthRequired => Some(WechatStatusAction::Cancel),
        StatusType::Status if is_terminal_text_status(&update.message) => {
            Some(WechatStatusAction::Cancel)
        }
        StatusType::ToolStarted
        | StatusType::ToolCompleted
        | StatusType::ToolResult
        | StatusType::Status
        | StatusType::JobStarted
        | StatusType::AuthCompleted => None,
    }
}

fn resolve_context_token(
    metadata: &OutboundMetadata,
    context_tokens: &std::collections::HashMap<String, String>,
) -> Option<String> {
    metadata
        .context_token
        .clone()
        .or_else(|| context_tokens.get(&metadata.from_user_id).cloned())
}

fn cached_typing_ticket(user_id: &str) -> Option<String> {
    let tickets = load_typing_tickets();
    let ticket = tickets.get(user_id)?;
    let trimmed = ticket.ticket.trim();
    if trimmed.is_empty() {
        return None;
    }

    let age_ms = channel_host::now_millis().saturating_sub(ticket.fetched_at_ms);
    if age_ms >= TYPING_TICKET_TTL_MS {
        return None;
    }

    Some(trimmed.to_string())
}

fn persist_typing_ticket(user_id: &str, ticket: &str) -> Result<(), String> {
    let mut tickets = load_typing_tickets();
    tickets.insert(
        user_id.to_string(),
        TypingTicketEntry {
            ticket: ticket.to_string(),
            fetched_at_ms: channel_host::now_millis(),
        },
    );
    persist_typing_tickets(&tickets)
}

fn clear_typing_ticket(user_id: &str) -> Result<(), String> {
    let mut tickets = load_typing_tickets();
    if tickets.remove(user_id).is_some() {
        persist_typing_tickets(&tickets)?;
    }
    Ok(())
}

fn resolve_typing_ticket(
    config: &WechatConfig,
    user_id: &str,
    context_token: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(ticket) = cached_typing_ticket(user_id) {
        return Ok(Some(ticket));
    }

    let response = api::get_config(config, user_id, context_token)?;
    if response.ret.unwrap_or(0) != 0 {
        let errmsg = response
            .errmsg
            .as_deref()
            .unwrap_or("unknown WeChat getConfig error");
        return Err(format!(
            "WeChat getConfig returned ret={} errmsg={errmsg}",
            response.ret.unwrap_or(-1)
        ));
    }

    let Some(ticket) = response
        .typing_ticket
        .as_deref()
        .map(str::trim)
        .filter(|ticket| !ticket.is_empty())
    else {
        return Ok(None);
    };

    if let Err(error) = persist_typing_ticket(user_id, ticket) {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Failed to persist WeChat typing ticket: {error}"),
        );
    }

    Ok(Some(ticket.to_string()))
}

fn send_typing_indicator(
    config: &WechatConfig,
    metadata: &OutboundMetadata,
    context_token: Option<&str>,
    status: i32,
    allow_ticket_fetch: bool,
) -> Result<(), String> {
    let ticket = if allow_ticket_fetch {
        resolve_typing_ticket(config, &metadata.from_user_id, context_token)?
    } else {
        cached_typing_ticket(&metadata.from_user_id)
    };

    let Some(ticket) = ticket else {
        return Ok(());
    };

    if let Err(error) = api::send_typing(config, &metadata.from_user_id, &ticket, status) {
        let _ = clear_typing_ticket(&metadata.from_user_id);
        return Err(error);
    }

    Ok(())
}

export!(WechatChannel);

#[cfg(test)]
mod tests {
    use super::{classify_status_update, WechatStatusAction};
    use crate::exports::near::agent::channel::{StatusType, StatusUpdate};

    #[test]
    fn test_classify_status_update_thinking_starts_typing() {
        let update = StatusUpdate {
            status: StatusType::Thinking,
            message: "Thinking...".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(WechatStatusAction::Typing)
        );
    }

    #[test]
    fn test_classify_status_update_done_cancels_typing() {
        let update = StatusUpdate {
            status: StatusType::Done,
            message: "Done".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(WechatStatusAction::Cancel)
        );
    }

    #[test]
    fn test_classify_status_update_approval_needed_cancels_typing() {
        let update = StatusUpdate {
            status: StatusType::ApprovalNeeded,
            message: "Approval needed".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(WechatStatusAction::Cancel)
        );
    }

    #[test]
    fn test_classify_status_update_tool_started_is_ignored() {
        let update = StatusUpdate {
            status: StatusType::ToolStarted,
            message: "Tool started".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_terminal_text_status_cancels_typing() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Awaiting approval".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(WechatStatusAction::Cancel)
        );
    }

    #[test]
    fn test_classify_status_update_progress_status_is_ignored() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Context compaction started".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }
}
