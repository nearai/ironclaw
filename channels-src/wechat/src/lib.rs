wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

mod api;
mod auth;
mod media;
mod state;
mod types;

use exports::near::agent::channel::{
    AgentResponse, Attachment, ChannelConfig, Guest, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};
use serde_json::json;

use crate::auth::TOKEN_SECRET_NAME;
use crate::state::{
    has_processed_message_id, load_config, load_context_tokens, load_get_updates_buf,
    load_pending_inbound_bundles, load_processed_message_ids, load_typing_tickets, persist_config,
    persist_context_tokens, persist_get_updates_buf, persist_pending_inbound_bundles,
    persist_processed_message_ids, persist_typing_tickets, remember_processed_message_id,
    PendingInboundBundle, StoredInboundAttachment, TypingTicketEntry,
};
use crate::types::{
    OutboundMetadata, WechatConfig, WechatMessage, MESSAGE_ITEM_TEXT, MESSAGE_TYPE_USER,
    TYPING_STATUS_CANCEL, TYPING_STATUS_TYPING,
};

const TYPING_TICKET_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_PROCESSED_MESSAGE_IDS: usize = 512;
const ATTACHMENT_DELIVERY_FAILED_FALLBACK: &str =
    "I finished the request, but WeChat couldn't deliver the attachment.";

#[cfg(not(test))]
pub(crate) fn debug_log(message: &str) {
    channel_host::log(channel_host::LogLevel::Debug, message);
}

#[cfg(test)]
pub(crate) fn debug_log(_message: &str) {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WechatStatusAction {
    Typing,
    Cancel,
}

struct WechatChannel;

struct FollowUpState<'a> {
    current_cursor: &'a mut String,
    context_tokens: &'a mut std::collections::HashMap<String, String>,
    context_tokens_changed: &'a mut bool,
    pending_inbound: &'a mut std::collections::HashMap<String, PendingInboundBundle>,
    pending_inbound_changed: &'a mut bool,
    processed_message_ids: &'a mut Vec<i64>,
    processed_message_ids_changed: &'a mut bool,
}

fn get_updates_error_message(response: &crate::types::GetUpdatesResponse) -> Option<String> {
    let errmsg = response
        .errmsg
        .as_deref()
        .unwrap_or("unknown WeChat polling error");

    if let Some(ret) = response.ret {
        if ret != 0 {
            return Some(format!("ret={ret} errmsg={errmsg}"));
        }
    }

    if let Some(errcode) = response.errcode {
        if errcode != 0 {
            return Some(format!("errcode={errcode} errmsg={errmsg}"));
        }
    }

    None
}

impl Guest for WechatChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config = serde_json::from_str::<WechatConfig>(&config_json)
            .map_err(|e| format!("Failed to parse WeChat config: {e}"))?;
        persist_config(&config)?;

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
        if !channel_host::secret_exists(TOKEN_SECRET_NAME) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                "WeChat bot token is missing; skipping poll",
            );
            return;
        }

        let config = load_config();
        let cursor = load_get_updates_buf();
        let mut current_cursor = cursor.clone();
        let mut context_tokens = load_context_tokens();
        let mut pending_inbound = match load_pending_inbound_bundles() {
            Ok(bundles) => bundles,
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to load WeChat pending inbound bundles: {error}"),
                );
                return;
            }
        };
        let mut processed_message_ids = match load_processed_message_ids() {
            Ok(message_ids) => message_ids,
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to load WeChat processed message ids: {error}"),
                );
                return;
            }
        };
        let mut pending_inbound_changed = false;
        let mut processed_message_ids_changed = false;

        for bundle in take_due_pending_bundles(&mut pending_inbound, channel_host::now_millis()) {
            pending_inbound_changed = true;
            emit_buffered_bundle(bundle);
        }

        match api::get_updates(&config, &current_cursor) {
            Ok(response) => {
                if response.errcode == Some(-14) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        "WeChat getUpdates returned errcode=-14; reconnect the channel",
                    );
                    return;
                }

                if let Some(error) = get_updates_error_message(&response) {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("WeChat getUpdates returned {error}"),
                    );
                }

                if let Some(next_cursor) = response.get_updates_buf.as_deref() {
                    if next_cursor != current_cursor {
                        current_cursor = next_cursor.to_string();
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
                    let message_id = message.message_id;
                    if let Some(message_id) = message_id {
                        if has_processed_message_id(&processed_message_ids, message_id) {
                            continue;
                        }
                    }
                    if let Some(from_user_id) = message.from_user_id.as_deref() {
                        if let Some(context_token) = message.context_token.as_deref() {
                            let changed = context_tokens
                                .insert(from_user_id.to_string(), context_token.to_string())
                                .as_deref()
                                != Some(context_token);
                            context_tokens_changed |= changed;
                        }
                    }
                    match incoming_bundle_from_message(&config, message) {
                        Ok(Some(bundle)) => {
                            let bundle_message_id = bundle.message_id;
                            let emitted = process_incoming_bundle(
                                &mut pending_inbound,
                                bundle,
                                &mut pending_inbound_changed,
                                channel_host::now_millis(),
                                u64::from(config.inbound_merge_window_ms),
                            );
                            for emitted_bundle in emitted {
                                emit_buffered_bundle(emitted_bundle);
                            }
                            if let Some(message_id) = bundle_message_id {
                                processed_message_ids_changed |= remember_processed_message_id(
                                    &mut processed_message_ids,
                                    message_id,
                                    MAX_PROCESSED_MESSAGE_IDS,
                                );
                            }
                        }
                        Ok(None) => {
                            if let Some(message_id) = message_id {
                                processed_message_ids_changed |= remember_processed_message_id(
                                    &mut processed_message_ids,
                                    message_id,
                                    MAX_PROCESSED_MESSAGE_IDS,
                                );
                            }
                        }
                        Err(error) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Failed to map WeChat inbound message: {error}"),
                            );
                        }
                    }
                }

                collect_follow_up_bundles(
                    &config,
                    FollowUpState {
                        current_cursor: &mut current_cursor,
                        context_tokens: &mut context_tokens,
                        context_tokens_changed: &mut context_tokens_changed,
                        pending_inbound: &mut pending_inbound,
                        pending_inbound_changed: &mut pending_inbound_changed,
                        processed_message_ids: &mut processed_message_ids,
                        processed_message_ids_changed: &mut processed_message_ids_changed,
                    },
                );

                for bundle in
                    take_due_pending_bundles(&mut pending_inbound, channel_host::now_millis())
                {
                    pending_inbound_changed = true;
                    emit_buffered_bundle(bundle);
                }

                if context_tokens_changed {
                    if let Err(error) = persist_context_tokens(&context_tokens) {
                        channel_host::log(
                            channel_host::LogLevel::Warn,
                            &format!("Failed to persist WeChat context tokens: {error}"),
                        );
                    }
                }

                if pending_inbound_changed {
                    if let Err(error) = persist_pending_inbound_bundles(&pending_inbound) {
                        channel_host::log(
                            channel_host::LogLevel::Warn,
                            &format!("Failed to persist WeChat pending inbound bundles: {error}"),
                        );
                    }
                }

                if processed_message_ids_changed {
                    if let Err(error) = persist_processed_message_ids(&processed_message_ids) {
                        channel_host::log(
                            channel_host::LogLevel::Warn,
                            &format!("Failed to persist WeChat processed message ids: {error}"),
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

        debug_log(&format!(
            "WeChat on_respond: text_len={} attachments={}",
            response.content.len(),
            response.attachments.len()
        ));

        send_response(&config, &metadata, &response, context_token.as_deref())
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

fn incoming_bundle_from_message(
    config: &WechatConfig,
    message: WechatMessage,
) -> Result<Option<PendingInboundBundle>, String> {
    if message.message_type != Some(MESSAGE_TYPE_USER) {
        return Ok(None);
    }

    let from_user_id = match message.from_user_id.as_deref() {
        Some(user_id) => user_id,
        None => return Ok(None),
    };

    let text = extract_text(&message);
    let attachments = media::extract_inbound_attachments(config, &message)?
        .into_iter()
        .map(StoredInboundAttachment::from)
        .collect::<Vec<_>>();
    if text.trim().is_empty() && attachments.is_empty() {
        return Ok(None);
    }

    Ok(Some(PendingInboundBundle {
        from_user_id: from_user_id.to_string(),
        to_user_id: message.to_user_id,
        session_id: message.session_id,
        context_token: message.context_token,
        message_id: message.message_id,
        flush_at_ms: 0,
        text,
        attachments,
    }))
}

fn process_incoming_bundle(
    pending_inbound: &mut std::collections::HashMap<String, PendingInboundBundle>,
    mut bundle: PendingInboundBundle,
    pending_inbound_changed: &mut bool,
    now_ms: u64,
    inbound_merge_window_ms: u64,
) -> Vec<PendingInboundBundle> {
    let key = bundle.from_user_id.clone();
    let bundle_has_text = !bundle.text.trim().is_empty();
    let bundle_has_attachments = !bundle.attachments.is_empty();

    if let Some(mut pending) = pending_inbound.remove(&key) {
        *pending_inbound_changed = true;

        if bundle_has_text {
            let incoming_metadata = bundle.clone();
            pending.text = merge_text(&pending.text, &bundle.text);
            pending.attachments.extend(bundle.attachments);
            merge_bundle_metadata(&mut pending, &incoming_metadata);
            return vec![pending];
        }

        let incoming_metadata = bundle.clone();
        pending.attachments.extend(bundle.attachments);
        merge_bundle_metadata(&mut pending, &incoming_metadata);
        pending.flush_at_ms = next_flush_deadline(now_ms, inbound_merge_window_ms);
        pending_inbound.insert(key, pending);
        return Vec::new();
    }

    if bundle_has_attachments && !bundle_has_text {
        *pending_inbound_changed = true;
        bundle.flush_at_ms = next_flush_deadline(now_ms, inbound_merge_window_ms);
        pending_inbound.insert(key, bundle);
        Vec::new()
    } else {
        vec![bundle]
    }
}

fn collect_follow_up_bundles(config: &WechatConfig, state: FollowUpState<'_>) {
    while !state.pending_inbound.is_empty() {
        let now_ms = channel_host::now_millis();
        let Some(timeout_ms) = next_follow_up_timeout_ms(state.pending_inbound, now_ms) else {
            break;
        };
        if timeout_ms == 0 {
            break;
        }

        let timeout_ms_u32 = timeout_ms.min(u64::from(u32::MAX)) as u32;
        let response =
            match api::get_updates_with_timeout(config, state.current_cursor, timeout_ms_u32) {
                Ok(response) => response,
                Err(_) => break,
            };

        if response.errcode == Some(-14) {
            channel_host::log(
                channel_host::LogLevel::Error,
                "WeChat getUpdates returned errcode=-14 during follow-up merge window; reconnect the channel",
            );
            break;
        }

        if let Some(error) = get_updates_error_message(&response) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("WeChat getUpdates returned {error} during follow-up merge window"),
            );
        }

        if let Some(next_cursor) = response.get_updates_buf.as_deref() {
            if next_cursor != state.current_cursor {
                *state.current_cursor = next_cursor.to_string();
                if let Err(error) = persist_get_updates_buf(next_cursor) {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to persist WeChat polling cursor: {error}"),
                    );
                }
            }
        }

        let mut saw_relevant_message = false;
        for message in response.msgs {
            let message_id = message.message_id;
            if let Some(message_id) = message_id {
                if has_processed_message_id(state.processed_message_ids, message_id) {
                    continue;
                }
            }
            if let Some(from_user_id) = message.from_user_id.as_deref() {
                if let Some(context_token) = message.context_token.as_deref() {
                    let changed = state
                        .context_tokens
                        .insert(from_user_id.to_string(), context_token.to_string())
                        .as_deref()
                        != Some(context_token);
                    *state.context_tokens_changed |= changed;
                }
            }
            match incoming_bundle_from_message(config, message) {
                Ok(Some(bundle)) => {
                    let bundle_message_id = bundle.message_id;
                    let emitted = process_incoming_bundle(
                        state.pending_inbound,
                        bundle,
                        state.pending_inbound_changed,
                        channel_host::now_millis(),
                        u64::from(config.inbound_merge_window_ms),
                    );
                    if let Some(message_id) = bundle_message_id {
                        *state.processed_message_ids_changed |= remember_processed_message_id(
                            state.processed_message_ids,
                            message_id,
                            MAX_PROCESSED_MESSAGE_IDS,
                        );
                    }
                    for emitted_bundle in emitted {
                        saw_relevant_message = true;
                        emit_buffered_bundle(emitted_bundle);
                    }
                }
                Ok(None) => {
                    if let Some(message_id) = message_id {
                        *state.processed_message_ids_changed |= remember_processed_message_id(
                            state.processed_message_ids,
                            message_id,
                            MAX_PROCESSED_MESSAGE_IDS,
                        );
                    }
                }
                Err(error) => {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to map WeChat inbound message: {error}"),
                    );
                }
            }
        }

        if !saw_relevant_message && state.pending_inbound.is_empty() {
            break;
        }
    }
}

fn next_flush_deadline(now_ms: u64, inbound_merge_window_ms: u64) -> u64 {
    now_ms.saturating_add(inbound_merge_window_ms)
}

fn next_follow_up_timeout_ms(
    pending_inbound: &std::collections::HashMap<String, PendingInboundBundle>,
    now_ms: u64,
) -> Option<u64> {
    pending_inbound
        .values()
        .map(|bundle| bundle.flush_at_ms.saturating_sub(now_ms))
        .min()
}

fn take_due_pending_bundles(
    pending_inbound: &mut std::collections::HashMap<String, PendingInboundBundle>,
    now_ms: u64,
) -> Vec<PendingInboundBundle> {
    let due_keys = pending_inbound
        .iter()
        .filter_map(|(key, bundle)| (bundle.flush_at_ms <= now_ms).then_some(key.clone()))
        .collect::<Vec<_>>();

    due_keys
        .into_iter()
        .filter_map(|key| pending_inbound.remove(&key))
        .collect()
}

fn emit_buffered_bundle(bundle: PendingInboundBundle) {
    let metadata = json!({
        "from_user_id": bundle.from_user_id,
        "to_user_id": bundle.to_user_id,
        "message_id": bundle.message_id,
        "session_id": bundle.session_id,
        "context_token": bundle.context_token,
    });

    channel_host::emit_message(&EmittedMessage {
        user_id: bundle.from_user_id.clone(),
        user_name: None,
        content: bundle.text,
        thread_id: Some(format!("wechat:{}", bundle.from_user_id)),
        metadata_json: metadata.to_string(),
        attachments: bundle.attachments.into_iter().map(Into::into).collect(),
    });
}

fn merge_bundle_metadata(target: &mut PendingInboundBundle, incoming: &PendingInboundBundle) {
    if incoming.to_user_id.is_some() {
        target.to_user_id = incoming.to_user_id.clone();
    }
    if incoming.session_id.is_some() {
        target.session_id = incoming.session_id.clone();
    }
    if incoming.context_token.is_some() {
        target.context_token = incoming.context_token.clone();
    }
    if incoming.message_id.is_some() {
        target.message_id = incoming.message_id;
    }
}

fn merge_text(existing: &str, incoming: &str) -> String {
    let existing = existing.trim();
    let incoming = incoming.trim();
    match (existing.is_empty(), incoming.is_empty()) {
        (true, true) => String::new(),
        (true, false) => incoming.to_string(),
        (false, true) => existing.to_string(),
        (false, false) => format!("{existing}\n\n{incoming}"),
    }
}

fn send_response(
    config: &WechatConfig,
    metadata: &OutboundMetadata,
    response: &AgentResponse,
    context_token: Option<&str>,
) -> Result<(), String> {
    send_response_with_handlers(
        response,
        |text| api::send_text_message(config, &metadata.from_user_id, text, context_token),
        |attachment| match media::classify_outbound_media_kind(&attachment.mime_type) {
            media::OutboundMediaKind::Image => media::send_image_attachment(
                config,
                &metadata.from_user_id,
                attachment,
                context_token,
            ),
            media::OutboundMediaKind::Video => media::send_video_attachment(
                config,
                &metadata.from_user_id,
                attachment,
                context_token,
            ),
            media::OutboundMediaKind::File => media::send_file_attachment(
                config,
                &metadata.from_user_id,
                attachment,
                context_token,
            ),
        },
        |message| channel_host::log(channel_host::LogLevel::Warn, &message),
    )
}

fn send_response_with_handlers<FText, FAttachment, FWarn>(
    response: &AgentResponse,
    mut send_text: FText,
    mut send_attachment: FAttachment,
    mut warn: FWarn,
) -> Result<(), String>
where
    FText: FnMut(&str) -> Result<(), String>,
    FAttachment: FnMut(&Attachment) -> Result<(), String>,
    FWarn: FnMut(String),
{
    let remaining_text = response.content.trim().to_string();
    let mut sent_attachment = false;
    let mut attachment_failures = 0usize;

    for attachment in &response.attachments {
        debug_log(&format!(
            "WeChat send_response: sending attachment filename='{}' mime='{}' bytes={}",
            attachment.filename,
            attachment.mime_type,
            attachment.data.len()
        ));
        match send_attachment(attachment) {
            Ok(()) => {
                sent_attachment = true;
                debug_log(&format!(
                    "WeChat send_response: attachment sent filename='{}'",
                    attachment.filename
                ));
            }
            Err(error) => {
                attachment_failures += 1;
                let filename = if attachment.filename.trim().is_empty() {
                    "<unnamed>"
                } else {
                    attachment.filename.as_str()
                };
                warn(format!(
                    "Failed to send WeChat attachment '{}' ({}): {}",
                    filename, attachment.mime_type, error
                ));
            }
        }
    }

    let should_send_text = !remaining_text.is_empty() || !sent_attachment;
    if should_send_text {
        let fallback_text = if !remaining_text.is_empty() {
            remaining_text.as_str()
        } else if attachment_failures > 0 {
            ATTACHMENT_DELIVERY_FAILED_FALLBACK
        } else {
            remaining_text.as_str()
        };

        debug_log(&format!(
            "WeChat send_response: sending final text len={}",
            fallback_text.len()
        ));
        send_text(fallback_text)?;
    }

    Ok(())
}

fn extract_text(message: &WechatMessage) -> String {
    message
        .item_list
        .iter()
        .find_map(|item| {
            if item.r#type == Some(MESSAGE_ITEM_TEXT) {
                item.text_item.as_ref().map(|item| item.text.clone())
            } else if item.r#type == Some(crate::types::MESSAGE_ITEM_VOICE) {
                item.voice_item
                    .as_ref()
                    .and_then(|item| item.text.as_ref())
                    .cloned()
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
    if !matches!(response.ret, Some(0)) {
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
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::{
        classify_status_update, extract_text, get_updates_error_message, merge_text,
        process_incoming_bundle, send_response_with_handlers, take_due_pending_bundles,
        PendingInboundBundle, StoredInboundAttachment, WechatStatusAction,
        ATTACHMENT_DELIVERY_FAILED_FALLBACK,
    };
    use crate::exports::near::agent::channel::{
        AgentResponse, Attachment, StatusType, StatusUpdate,
    };
    use crate::types::{
        GetUpdatesResponse, MessageItem, VoiceItem, WechatMessage, MESSAGE_ITEM_VOICE,
    };

    fn make_bundle(user_id: &str, text: &str, image_count: usize) -> PendingInboundBundle {
        PendingInboundBundle {
            from_user_id: user_id.to_string(),
            to_user_id: Some("bot".to_string()),
            session_id: Some("session-1".to_string()),
            context_token: Some("ctx-1".to_string()),
            message_id: Some(1),
            flush_at_ms: 0,
            text: text.to_string(),
            attachments: (0..image_count)
                .map(|index| StoredInboundAttachment {
                    id: format!("att-{index}"),
                    mime_type: "image/jpeg".to_string(),
                    filename: Some(format!("photo-{index}.jpg")),
                    size_bytes: Some(128),
                    source_url: Some("https://example.com/image.jpg".to_string()),
                    storage_key: None,
                    extracted_text: None,
                    extras_json: "{}".to_string(),
                })
                .collect(),
        }
    }

    fn make_updates_response(ret: Option<i32>, errcode: Option<i32>) -> GetUpdatesResponse {
        GetUpdatesResponse {
            ret,
            errcode,
            errmsg: None,
            msgs: Vec::new(),
            get_updates_buf: None,
        }
    }

    #[test]
    fn test_get_updates_error_message_allows_empty_poll_without_ret() {
        let response = make_updates_response(None, None);
        assert_eq!(get_updates_error_message(&response), None);
    }

    #[test]
    fn test_get_updates_error_message_reports_nonzero_ret() {
        let mut response = make_updates_response(Some(42), None);
        response.errmsg = Some("bad cursor".to_string());
        assert_eq!(
            get_updates_error_message(&response),
            Some("ret=42 errmsg=bad cursor".to_string())
        );
    }

    #[test]
    fn test_get_updates_error_message_reports_nonzero_errcode() {
        let response = make_updates_response(None, Some(-14));
        assert_eq!(
            get_updates_error_message(&response),
            Some("errcode=-14 errmsg=unknown WeChat polling error".to_string())
        );
    }

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

    #[test]
    fn test_merge_text_joins_non_empty_segments() {
        assert_eq!(merge_text("", "hello"), "hello");
        assert_eq!(merge_text("look", "what is this"), "look\n\nwhat is this");
        assert_eq!(merge_text("look", ""), "look");
    }

    #[test]
    fn test_extract_text_uses_voice_transcript_when_present() {
        let message = WechatMessage {
            message_id: Some(1),
            from_user_id: Some("user-1".to_string()),
            to_user_id: Some("bot-1".to_string()),
            session_id: None,
            message_type: None,
            context_token: None,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_VOICE),
                text_item: None,
                image_item: None,
                voice_item: Some(VoiceItem {
                    media: None,
                    encode_type: Some(6),
                    playtime: Some(1500),
                    text: Some("voice transcript".to_string()),
                }),
                file_item: None,
                video_item: None,
            }],
        };

        assert_eq!(extract_text(&message), "voice transcript");
    }

    #[test]
    fn test_process_incoming_bundle_merges_buffered_image_with_follow_up_text() {
        let mut pending = HashMap::new();
        let mut changed = false;

        let emitted = process_incoming_bundle(
            &mut pending,
            make_bundle("u1", "", 1),
            &mut changed,
            100,
            5_000,
        );
        assert!(emitted.is_empty());
        assert!(changed);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending["u1"].flush_at_ms, 5100);

        changed = false;
        let emitted = process_incoming_bundle(
            &mut pending,
            make_bundle("u1", "What is in this image?", 0),
            &mut changed,
            200,
            5_000,
        );
        assert!(changed);
        assert!(pending.is_empty());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].text, "What is in this image?");
        assert_eq!(emitted[0].attachments.len(), 1);
    }

    #[test]
    fn test_process_incoming_bundle_extends_window_for_attachment_only_follow_up() {
        let mut pending = HashMap::new();
        let mut changed = false;

        let emitted = process_incoming_bundle(
            &mut pending,
            make_bundle("u1", "", 1),
            &mut changed,
            100,
            5_000,
        );
        assert!(emitted.is_empty());
        assert!(changed);
        assert_eq!(pending["u1"].flush_at_ms, 5_100);

        changed = false;
        let emitted = process_incoming_bundle(
            &mut pending,
            make_bundle("u1", "", 1),
            &mut changed,
            700,
            5_000,
        );
        assert!(emitted.is_empty());
        assert!(changed);
        assert_eq!(pending["u1"].attachments.len(), 2);
        assert_eq!(pending["u1"].flush_at_ms, 5_700);
    }

    #[test]
    fn test_process_incoming_bundle_emits_text_and_images_together_without_buffering() {
        let mut pending = HashMap::new();
        let mut changed = false;

        let emitted = process_incoming_bundle(
            &mut pending,
            make_bundle("u1", "Look at this image", 1),
            &mut changed,
            100,
            5_000,
        );
        assert!(!changed);
        assert!(pending.is_empty());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].text, "Look at this image");
        assert_eq!(emitted[0].attachments.len(), 1);
    }

    #[test]
    fn test_take_due_pending_bundles_emits_only_expired_entries() {
        let mut pending = HashMap::new();
        let mut expired = make_bundle("u1", "", 1);
        expired.flush_at_ms = 100;
        let mut fresh = make_bundle("u2", "", 1);
        fresh.flush_at_ms = 300;
        pending.insert(expired.from_user_id.clone(), expired);
        pending.insert(fresh.from_user_id.clone(), fresh);

        let due = take_due_pending_bundles(&mut pending, 200);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].from_user_id, "u1");
        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key("u2"));
    }

    #[test]
    fn test_send_response_sends_attachments_before_text() {
        let response = AgentResponse {
            message_id: "msg-1".to_string(),
            content: "Here is the image you asked for.".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: vec![Attachment {
                filename: "cat.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                data: vec![1, 2, 3],
            }],
        };
        let sent_events = RefCell::new(Vec::new());

        let result = send_response_with_handlers(
            &response,
            |text| {
                sent_events.borrow_mut().push(format!("text:{text}"));
                Ok(())
            },
            |attachment| {
                sent_events
                    .borrow_mut()
                    .push(format!("attachment:{}", attachment.filename));
                Ok(())
            },
            |_message| {},
        );

        assert!(result.is_ok());
        assert_eq!(
            sent_events.into_inner(),
            vec![
                "attachment:cat.jpg".to_string(),
                "text:Here is the image you asked for.".to_string()
            ]
        );
    }

    #[test]
    fn test_send_response_falls_back_to_text_when_attachment_send_fails() {
        let response = AgentResponse {
            message_id: "msg-1".to_string(),
            content: "Here is the image you asked for.".to_string(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: vec![Attachment {
                filename: "cat.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                data: vec![1, 2, 3],
            }],
        };
        let mut sent_texts = Vec::new();
        let mut warnings = Vec::new();

        let result = send_response_with_handlers(
            &response,
            |text| {
                sent_texts.push(text.to_string());
                Ok(())
            },
            |_attachment| Err("upload failed".to_string()),
            |message| warnings.push(message),
        );

        assert!(result.is_ok());
        assert_eq!(sent_texts, vec!["Here is the image you asked for."]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("upload failed"));
    }

    #[test]
    fn test_send_response_sends_generic_text_when_attachment_only_reply_fails() {
        let response = AgentResponse {
            message_id: "msg-1".to_string(),
            content: String::new(),
            thread_id: None,
            metadata_json: "{}".to_string(),
            attachments: vec![Attachment {
                filename: "cat.jpg".to_string(),
                mime_type: "image/jpeg".to_string(),
                data: vec![1, 2, 3],
            }],
        };
        let mut sent_texts = Vec::new();

        let result = send_response_with_handlers(
            &response,
            |text| {
                sent_texts.push(text.to_string());
                Ok(())
            },
            |_attachment| Err("upload failed".to_string()),
            |_message| {},
        );

        assert!(result.is_ok());
        assert_eq!(
            sent_texts,
            vec![ATTACHMENT_DELIVERY_FAILED_FALLBACK.to_string()]
        );
    }
}
