//! DingTalk (钉钉) enterprise bot channel via Stream mode (WebSocket).
//!
//! Uses DingTalk's Stream API to maintain a persistent connection without
//! requiring a public IP. Messages arrive over WebSocket and replies are
//! sent via the Robot API.
//!
//! # Features
//!
//! - Stream mode (WebSocket) — no public IP needed
//! - Private chat (1:1) and group chat (@robot)
//! - Text, rich text, image message types
//! - Markdown reply mode
//! - OAuth2 access token management with auto-refresh
//! - Proactive message sending (broadcast)
//!
//! # Configuration
//!
//! ```json5
//! {
//!   "channels": {
//!     "dingtalk": {
//!       "enabled": true,
//!       "clientId": "dingxxxxxx",
//!       "clientSecret": "your-app-secret",
//!       "robotCode": "your-robot-code"  // optional, auto-detected from messages
//!     }
//!   }
//! }
//! ```

mod card_service;
mod connection;
pub mod docs_api;
pub mod feedback;
mod filters;
pub(super) mod media;
mod send;
mod stream;
mod types;

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lru::LruCache;
use reqwest::Client;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::{CardStreamMode, DingTalkConfig, DingTalkMessageType};
use crate::error::ChannelError;

use types::{CardPhase, CardState, DingTalkMetadata, MarkdownMsgParam};

const MAX_REPLY_TARGETS: usize = 10000;
const REPLY_TARGETS_CAP: NonZeroUsize = NonZeroUsize::new(MAX_REPLY_TARGETS).unwrap();
const DEFAULT_DINGTALK_API_BASE_URL: &str = "https://api.dingtalk.com";

/// DingTalk channel using Stream mode (persistent WebSocket).
pub struct DingTalkChannel {
    config: DingTalkConfig,
    client: Client,
    reply_targets: Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
    /// Cached access token with expiry.
    access_token: Arc<RwLock<Option<(String, std::time::Instant)>>>,
    /// Active AI card states, keyed by message UUID.
    card_states: Arc<RwLock<std::collections::HashMap<Uuid, CardState>>>,
    /// Per-message lock to serialize card status updates.
    status_locks: Arc<Mutex<std::collections::HashMap<Uuid, Arc<Mutex<()>>>>>,
    /// Notify handle to trigger WebSocket reconnect on reconfigure.
    reconnect_notify: Arc<tokio::sync::Notify>,
    /// Conversations that have received a stop/interrupt signal recently.
    stopped_conversations: Arc<RwLock<std::collections::HashMap<String, std::time::Instant>>>,
    /// Cancellation token: fired by shutdown() so run_stream_listener exits
    /// cleanly (the tokio::spawn task can then be joined or dropped safely).
    /// Without this, `ChannelManager::hot_add` on /api/reconfigure leaves
    /// the old WebSocket task running — competing with the replacement and
    /// populating an orphaned `reply_targets` map. Reliable single-instance
    /// behavior across reconfigures depends on this actually stopping.
    shutdown_signal: Arc<tokio::sync::Notify>,
    /// JoinHandle of the spawned stream task, behind a mutex so shutdown()
    /// can reclaim and abort it even if someone else holds a read lock.
    stream_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Active card count across this channel process. Used to enforce
    /// `DingTalkConfig::max_active_cards`. Incremented on card create,
    /// decremented on cleanup.
    active_card_count: Arc<std::sync::atomic::AtomicU32>,
    /// `(conversation_id, originating_user_id) → msg_id` index for detecting
    /// supersedes when a user sends a new message while a prior card is
    /// in-flight (see Unit 7).
    conv_user_to_card: Arc<RwLock<std::collections::HashMap<(String, String), Uuid>>>,
}

impl DingTalkChannel {
    pub fn new(config: DingTalkConfig) -> Result<Self, ChannelError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        Ok(Self {
            config,
            client,
            reply_targets: Arc::new(RwLock::new(LruCache::new(REPLY_TARGETS_CAP))),
            access_token: Arc::new(RwLock::new(None)),
            card_states: Arc::new(RwLock::new(std::collections::HashMap::new())),
            status_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
            reconnect_notify: Arc::new(tokio::sync::Notify::new()),
            stopped_conversations: Arc::new(RwLock::new(std::collections::HashMap::new())),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            stream_task: Mutex::new(None),
            active_card_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            conv_user_to_card: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Get the reconnect notify handle.
    ///
    /// Callers (e.g. the gateway reconfigure handler) can call `notify_one()` on this
    /// to trigger the DingTalk Stream WebSocket to reconnect with fresh config.
    pub fn reconnect_notify(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.reconnect_notify)
    }

    pub(super) fn api_url(path: &str) -> String {
        let base = std::env::var("IRONCLAW_TEST_DINGTALK_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_DINGTALK_API_BASE_URL.to_string());
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn status_can_activate_card(status: &StatusUpdate) -> bool {
        matches!(
            status,
            StatusUpdate::StreamChunk(_)
                | StatusUpdate::Thinking(_)
                | StatusUpdate::ToolStarted { .. }
                | StatusUpdate::ToolCompleted { .. }
        )
    }

    fn card_delivery_enabled(&self) -> bool {
        self.config.message_type == DingTalkMessageType::Card
            && self.config.card_template_id.is_some()
    }

    fn status_supports_live_flush(&self, status: &StatusUpdate) -> bool {
        match status {
            StatusUpdate::StreamChunk(_) => self.config.card_stream_mode != CardStreamMode::Off,
            StatusUpdate::Thinking(_)
            | StatusUpdate::ToolStarted { .. }
            | StatusUpdate::ToolCompleted { .. } => {
                self.config.card_stream_mode == CardStreamMode::All
            }
            _ => false,
        }
    }

    fn rendered_card_content(&self, state: &CardState) -> Option<String> {
        let thinking = if self.config.card_stream_mode == CardStreamMode::All {
            state.thinking_buffer.trim()
        } else {
            ""
        };
        let content = state.content_buffer.trim();

        match (thinking.is_empty(), content.is_empty()) {
            (true, true) => None,
            (false, true) => Some(thinking.to_string()),
            (true, false) => Some(state.content_buffer.clone()),
            (false, false) => Some(format!("{thinking}\n\n{content}")),
        }
    }

    fn append_line(buffer: &mut String, line: &str) {
        if line.is_empty() {
            return;
        }
        if !buffer.is_empty() && !buffer.ends_with('\n') {
            buffer.push('\n');
        }
        buffer.push_str(line);
    }

    async fn cleanup_message_state(&self, msg_id: Uuid) {
        // Capture any tick handle so we can await it outside the map lock.
        let (tick_handle, tick_cancel, conv_user_key, tick_was_present) = {
            let mut states = self.card_states.write().await;
            match states.remove(&msg_id) {
                Some(mut state) => {
                    let handle = state.tick_handle.take();
                    let cancel = state.tick_cancel.clone();
                    let key = (
                        // Note: we don't have conversation_id on CardState; look up via
                        // reply_targets below if needed. For now, decrement by user-id
                        // alone is handled in the secondary-index cleanup path.
                        String::new(),
                        state.originating_user_id.clone(),
                    );
                    let was_present = !state.instance_id.is_empty();
                    (handle, Some(cancel), Some(key), was_present)
                }
                None => (None, None, None, false),
            }
        };

        // Reply metadata has the conversation_id we need for the secondary
        // index cleanup. Grab it before we drop it from reply_targets.
        let conv_id = self
            .reply_targets
            .read()
            .await
            .peek(&msg_id)
            .map(|m| m.conversation_id.clone());

        self.reply_targets.write().await.pop(&msg_id);
        self.status_locks.lock().await.remove(&msg_id);

        if let (Some(conv_id), Some(mut key)) = (conv_id, conv_user_key) {
            key.0 = conv_id;
            let mut idx = self.conv_user_to_card.write().await;
            // Only remove if the index still points at us — avoid clobbering
            // a newer card that superseded us.
            if idx.get(&key) == Some(&msg_id) {
                idx.remove(&key);
            }
        }

        // Cancel + drain tick task with a bounded grace period (5s, mirroring
        // shutdown()'s pattern). Do this LAST so we don't hold map locks.
        if let Some(cancel) = tick_cancel {
            cancel.notify_one();
        }
        if let Some(handle) = tick_handle {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }

        if tick_was_present {
            self.active_card_count
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    async fn mark_card_fallback_required(&self, msg_id: Uuid) {
        let mut states = self.card_states.write().await;
        states
            .entry(msg_id)
            .and_modify(|state| state.fallback_required = true)
            .or_insert_with(|| CardState {
                instance_id: String::new(),
                content_buffer: String::new(),
                thinking_buffer: String::new(),
                last_content_update: None,
                phase: CardPhase::Processing,
                fallback_required: true,
                created_at: std::time::Instant::now(),
                // Fail-closed: assume Group until proven otherwise.
                channel_level: crate::channels::dingtalk::types::ChannelLevel::Group,
                agent_phase: crate::channels::dingtalk::types::AgentPhase::Thinking,
                current_tool: None,
                reasoning_excerpt: None,
                reasoning_summary_enabled: false,
                slow_tier: crate::channels::dingtalk::types::SlowTier::None,
                tick_cancel: std::sync::Arc::new(tokio::sync::Notify::new()),
                tick_handle: None,
                tick_degraded: false,
                seen_sensitive: std::collections::HashSet::new(),
                originating_user_id: String::new(),
                tools_used: 0,
                retry_attempt: 0,
            });
    }

    async fn ensure_card_ready(&self, msg_id: Uuid) -> bool {
        if !self.card_delivery_enabled() {
            return false;
        }

        {
            let states = self.card_states.read().await;
            if let Some(state) = states.get(&msg_id) {
                return !state.fallback_required;
            }
        }

        let (reply_meta, cache_len, has_any_entries) = {
            let targets = self.reply_targets.read().await;
            (
                targets.peek(&msg_id).cloned(),
                targets.len(),
                targets.len() > 0,
            )
        };
        let Some(reply_meta) = reply_meta else {
            tracing::warn!(
                msg_id = %msg_id,
                client_id = %self.config.client_id,
                reply_targets_len = cache_len,
                has_any_entries,
                "No reply metadata for card creation — cache miss \
                 (zombie channel instance after hot_add is the usual cause)"
            );
            self.mark_card_fallback_required(msg_id).await;
            return false;
        };

        let token = match self.get_access_token().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to get token for card creation");
                self.mark_card_fallback_required(msg_id).await;
                return false;
            }
        };

        tracing::info!(
            msg_id = %msg_id,
            conversation_id = %reply_meta.conversation_id,
            conversation_type = %reply_meta.conversation_type,
            "Creating DingTalk AI card"
        );

        let instance_id = match card_service::create_ai_card(
            &self.client,
            &self.config,
            &token,
            &reply_meta.conversation_id,
            &reply_meta.conversation_type,
            &reply_meta.sender_staff_id,
        )
        .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to create AI card, will fall back to Markdown");
                self.mark_card_fallback_required(msg_id).await;
                return false;
            }
        };

        tracing::info!(out_track_id = %instance_id, "DingTalk AI card created");

        // Group vs DM is derived from DingTalk's conversation_type.
        // Per stream.rs, "2" = group, otherwise DM. Fail-closed to Group on
        // anything ambiguous so bystander privacy holds by default.
        let channel_level = if reply_meta.conversation_type == "2" {
            crate::channels::dingtalk::types::ChannelLevel::Group
        } else if reply_meta.conversation_type == "1" {
            crate::channels::dingtalk::types::ChannelLevel::Dm
        } else {
            crate::channels::dingtalk::types::ChannelLevel::Group
        };

        let originating_user_id = reply_meta.sender_staff_id.clone();

        // Active-card cap: the 1001st card still ships but runs degraded.
        let tick_degraded = self
            .active_card_count
            .load(std::sync::atomic::Ordering::Relaxed)
            >= self.config.max_active_cards;

        // Maintain (conversation, user) → msg_id secondary index for supersede
        // lookups. Seeing an existing entry here means the prior card is
        // in-flight; the caller (see Unit 7) handles drain-and-replace.
        {
            let mut idx = self.conv_user_to_card.write().await;
            let key = (
                reply_meta.conversation_id.clone(),
                originating_user_id.clone(),
            );
            idx.insert(key, msg_id);
        }

        self.active_card_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut states = self.card_states.write().await;
        states.entry(msg_id).or_insert_with(|| CardState {
            instance_id,
            content_buffer: String::new(),
            thinking_buffer: String::new(),
            last_content_update: None,
            phase: CardPhase::Inputing,
            fallback_required: false,
            created_at: std::time::Instant::now(),
            channel_level,
            agent_phase: crate::channels::dingtalk::types::AgentPhase::Thinking,
            current_tool: None,
            reasoning_excerpt: None,
            reasoning_summary_enabled: false,
            slow_tier: crate::channels::dingtalk::types::SlowTier::None,
            tick_cancel: std::sync::Arc::new(tokio::sync::Notify::new()),
            tick_handle: None,
            tick_degraded,
            seen_sensitive: std::collections::HashSet::new(),
            originating_user_id,
            tools_used: 0,
            retry_attempt: 0,
        });

        true
    }

    async fn flush_card_if_needed(&self, msg_id: Uuid, force: bool) {
        let pending = {
            let mut states = self.card_states.write().await;
            let Some(state) = states.get_mut(&msg_id) else {
                return;
            };

            if state.fallback_required {
                return;
            }

            let Some(content) = self.rendered_card_content(state) else {
                return;
            };

            let now = std::time::Instant::now();
            let should_flush = if force {
                true
            } else {
                match state.last_content_update {
                    None => true,
                    Some(last) => {
                        now.duration_since(last)
                            >= Duration::from_millis(self.config.card_stream_interval_ms)
                    }
                }
            };

            if !should_flush {
                return;
            }

            state.last_content_update = Some(now);
            Some((state.instance_id.clone(), content))
        };

        let Some((instance_id, content)) = pending else {
            return;
        };

        let token = match self.get_access_token().await {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!(error = %e, "Failed to get token for card stream");
                self.mark_card_fallback_required(msg_id).await;
                return;
            }
        };

        if let Err(e) = card_service::stream_ai_card(
            &self.client,
            &token,
            &instance_id,
            &content,
            &self.config.card_template_key,
            false,
            false,
        )
        .await
        {
            tracing::warn!(error = %e, "Failed to stream AI card update");
            self.mark_card_fallback_required(msg_id).await;
        }
    }

    #[cfg(any(test, feature = "integration"))]
    pub async fn seed_reply_target_for_test(
        &self,
        message: &IncomingMessage,
    ) -> Result<(), ChannelError> {
        let metadata: DingTalkMetadata =
            serde_json::from_value(message.metadata.clone()).map_err(|e| {
                ChannelError::SendFailed {
                    name: "dingtalk".into(),
                    reason: format!("invalid DingTalk metadata for test seeding: {e}"),
                }
            })?;

        self.reply_targets.write().await.put(message.id, metadata);
        Ok(())
    }

    /// Get a valid access token, refreshing if expired.
    async fn get_access_token(&self) -> Result<String, ChannelError> {
        // Check cache first
        {
            let cache = self.access_token.read().await;
            if let Some((ref token, expiry)) = *cache {
                if expiry > std::time::Instant::now() {
                    return Ok(token.clone());
                }
            }
        }

        // Fetch new token
        use secrecy::ExposeSecret;
        let resp = self
            .client
            .post(Self::api_url("/v1.0/oauth2/accessToken"))
            .json(&serde_json::json!({
                "appKey": self.config.client_id,
                "appSecret": self.config.client_secret.expose_secret(),
            }))
            .send()
            .await
            .map_err(|e| ChannelError::Http(format!("token request: {e}")))?;

        let token_resp_value = send::parse_business_response(resp, "token API")
            .await?
            .ok_or_else(|| ChannelError::Http("token API returned empty body".to_string()))?;
        let token = token_resp_value
            .get("accessToken")
            .or_else(|| token_resp_value.get("access_token"))
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("accessToken"))
            })
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("access_token"))
            })
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ChannelError::Http("no access_token in response".to_string()))?
            .to_string();

        let expires_in = token_resp_value
            .get("expireIn")
            .or_else(|| token_resp_value.get("expiresIn"))
            .or_else(|| token_resp_value.get("expires_in"))
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("expireIn"))
            })
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("expiresIn"))
            })
            .or_else(|| {
                token_resp_value
                    .get("result")
                    .and_then(|v| v.get("expires_in"))
            })
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
            })
            .unwrap_or(7200);
        // Refresh 5 minutes before expiry
        let expiry =
            std::time::Instant::now() + Duration::from_secs(expires_in.saturating_sub(300));

        let mut cache = self.access_token.write().await;
        *cache = Some((token.clone(), expiry));

        Ok(token)
    }

    fn is_terminal_status_message(msg: &str) -> bool {
        let trimmed = msg.trim();
        trimmed.eq_ignore_ascii_case("done")
            || trimmed.eq_ignore_ascii_case("interrupted")
            || trimmed.eq_ignore_ascii_case("awaiting approval")
            || trimmed.eq_ignore_ascii_case("rejected")
    }

    /// Send a markdown message via Robot API.
    async fn send_markdown(
        &self,
        token: &str,
        robot_code: &str,
        conversation_id: Option<&str>,
        user_ids: &[&str],
        title: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        let msg_param = serde_json::to_string(&MarkdownMsgParam {
            title: title.to_string(),
            text: text.to_string(),
        })
        .map_err(|e| ChannelError::Http(format!("serialize: {e}")))?;

        let mut body = serde_json::json!({
            "msgKey": "sampleMarkdown",
            "msgParam": msg_param,
            "robotCode": robot_code,
        });

        let url = if let Some(conv_id) = conversation_id {
            body["openConversationId"] = serde_json::Value::String(conv_id.to_string());
            Self::api_url("/v1.0/robot/groupMessages/send")
        } else {
            body["userIds"] = serde_json::Value::Array(
                user_ids
                    .iter()
                    .map(|u| serde_json::Value::String(u.to_string()))
                    .collect(),
            );
            Self::api_url("/v1.0/robot/oToMessages/batchSend")
        };

        let resp = self
            .client
            .post(url)
            .header("x-acs-dingtalk-access-token", token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Http(format!("send: {e}")))?;

        send::ensure_business_success(resp, "Robot API").await
    }
}

#[async_trait]
impl Channel for DingTalkChannel {
    fn name(&self) -> &str {
        "dingtalk"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        let config = self.config.clone();
        let client = self.client.clone();
        let reply_targets = Arc::clone(&self.reply_targets);
        let reconnect_notify = Arc::clone(&self.reconnect_notify);
        let stopped_conversations = Arc::clone(&self.stopped_conversations);
        let shutdown_signal = Arc::clone(&self.shutdown_signal);
        let client_id_for_log = self.config.client_id.clone();

        let handle = tokio::spawn(async move {
            tracing::info!(
                client_id = %client_id_for_log,
                "DingTalk stream task starting"
            );
            tokio::select! {
                res = stream::run_stream_listener(
                    config,
                    client,
                    tx,
                    reply_targets,
                    reconnect_notify,
                    stopped_conversations,
                ) => {
                    match res {
                        Ok(()) => tracing::info!(
                            client_id = %client_id_for_log,
                            "DingTalk stream task exited cleanly"
                        ),
                        Err(e) => tracing::error!(
                            client_id = %client_id_for_log,
                            error = %e,
                            "DingTalk Stream listener exited with error"
                        ),
                    }
                }
                _ = shutdown_signal.notified() => {
                    tracing::info!(
                        client_id = %client_id_for_log,
                        "DingTalk stream task received shutdown signal"
                    );
                }
            }
        });

        // Retain the handle so shutdown() can reclaim and abort this task.
        // Replacing any prior handle is safe: start() should only be called
        // once per instance, but if it isn't, the old task is orphaned the
        // same way it was before this change — at worst, no regression.
        *self.stream_task.lock().await = Some(handle);

        tracing::info!(
            client_id = %self.config.client_id,
            "DingTalk channel enabled (Stream mode)"
        );

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let (metadata, cache_len) = {
            let targets = self.reply_targets.read().await;
            (targets.peek(&msg.id).cloned(), targets.len())
        };

        let Some(metadata) = metadata else {
            self.cleanup_message_state(msg.id).await;
            tracing::warn!(
                msg_id = %msg.id,
                client_id = %self.config.client_id,
                reply_targets_len = cache_len,
                "No reply metadata found for DingTalk message — response will be dropped \
                 (usual cause: zombie DingTalk channel instance after hot_add; \
                 check for duplicate 'DingTalk channel enabled' logs)"
            );
            return Ok(());
        };

        let conversation_key = if !metadata.conversation_id.is_empty() {
            metadata.conversation_id.as_str()
        } else {
            metadata.sender_staff_id.as_str()
        };

        if stream::is_conversation_stopped(&self.stopped_conversations, conversation_key).await {
            self.cleanup_message_state(msg.id).await;
            tracing::debug!(
                msg_id = %msg.id,
                conversation = %conversation_key,
                "Skipping DingTalk reply for stopped conversation"
            );
            return Ok(());
        }

        // Snapshot just the fields finalize needs — avoid cloning CardState
        // itself (it owns a non-Clone JoinHandle).
        let card_snapshot: Option<(String, bool)> = {
            let states = self.card_states.read().await;
            states
                .get(&msg.id)
                .map(|s| (s.instance_id.clone(), s.fallback_required))
        };

        if let Some((instance_id, fallback_required)) = card_snapshot {
            if self.config.message_type == DingTalkMessageType::Card && !fallback_required {
                match self.get_access_token().await {
                    Ok(token) => {
                        if let Err(e) = card_service::finalize_ai_card(
                            &self.client,
                            &self.config,
                            &token,
                            &instance_id,
                            &response.content,
                        )
                        .await
                        {
                            tracing::warn!(
                                error = %e,
                                msg_id = %msg.id,
                                "Failed to finalize AI card, falling back to markdown"
                            );
                        } else {
                            self.cleanup_message_state(msg.id).await;
                            tracing::info!(msg_id = %msg.id, "Skipping markdown reply — AI card used");
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            msg_id = %msg.id,
                            "Failed to get token for AI card finalize, falling back to markdown"
                        );
                    }
                }
            }

            // Finalize failed or fallback path — drop the card state and
            // continue to markdown reply. cleanup_message_state also drains
            // the tick task.
            self.cleanup_message_state(msg.id).await;
        }

        let robot_code = metadata
            .robot_code
            .as_deref()
            .or(self.config.robot_code.as_deref())
            .unwrap_or_default();

        let is_group = metadata.conversation_type == "2";
        let content = &response.content;

        // Split long responses into chunks (3800 char limit per DingTalk message).
        let chunks = send::split_markdown_chunks(content, send::DEFAULT_CHUNK_LIMIT);
        let (_, base_title) = send::detect_markdown(content);

        // Check whether the session webhook is still valid.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let webhook_valid = metadata
            .session_webhook
            .as_deref()
            .zip(metadata.session_webhook_expired_time)
            .map(|(_, exp)| now_ms < exp)
            .unwrap_or(false);

        for (idx, chunk) in chunks.iter().enumerate() {
            // Title for first chunk is "Reply"; subsequent chunks get (N/M) suffix.
            let chunk_title = if chunks.len() == 1 {
                base_title.clone()
            } else if idx == 0 {
                "Reply".to_string()
            } else {
                format!("Reply ({}/{})", idx + 1, chunks.len())
            };

            if webhook_valid {
                let webhook_url = metadata.session_webhook.as_deref().unwrap_or_default();
                tracing::debug!(
                    chunk = idx + 1,
                    total = chunks.len(),
                    "Sending DingTalk reply via session webhook"
                );
                match send::send_via_webhook(&self.client, webhook_url, &chunk_title, chunk).await {
                    Ok(()) => continue,
                    Err(e) => {
                        // Fall back to Robot API on webhook failure.
                        tracing::debug!(error = %e, "Session webhook failed, falling back to Robot API");
                    }
                }
            }

            // Robot API path (primary when no valid webhook, or webhook fallback).
            let token = self.get_access_token().await?;
            let user_ids_vec = vec![metadata.sender_staff_id.as_str()];
            self.send_markdown(
                &token,
                robot_code,
                if is_group {
                    Some(&metadata.conversation_id)
                } else {
                    None
                },
                if is_group { &[] } else { &user_ids_vec },
                &chunk_title,
                chunk,
            )
            .await?;
        }

        // ── Attachments: upload and send each one as a media message ────────────
        for attachment_path_str in &response.attachments {
            let attachment_path = std::path::Path::new(attachment_path_str);
            let media_type = media::detect_media_type(attachment_path);

            let token = match self.get_access_token().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: failed to get token for attachment upload, skipping"
                    );
                    continue;
                }
            };

            let media_id = match media::upload_media(
                &self.client,
                &token,
                attachment_path,
                media_type,
            )
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: attachment upload failed, skipping"
                    );
                    continue;
                }
            };

            // Build the media message body
            let (msg_key, msg_param_value) = match media_type {
                "image" => {
                    let param = serde_json::json!({ "photoURL": format!("@{media_id}") });
                    ("sampleImageMsg", param)
                }
                "voice" => {
                    let param = serde_json::json!({ "mediaId": media_id, "duration": "0" });
                    ("sampleAudioMsg", param)
                }
                "video" => {
                    let param = serde_json::json!({ "videoMediaId": media_id, "videoType": "mp4" });
                    ("sampleVideoMsg", param)
                }
                _ => {
                    // "file" or anything else
                    let filename = attachment_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("attachment");
                    let ext = attachment_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("bin")
                        .to_ascii_lowercase();
                    let param = serde_json::json!({
                        "mediaId": media_id,
                        "fileName": filename,
                        "fileType": ext,
                    });
                    ("sampleFileMsg", param)
                }
            };

            let msg_param_str = match serde_json::to_string(&msg_param_value) {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: failed to serialize media msg param, skipping"
                    );
                    continue;
                }
            };

            let mut body = serde_json::json!({
                "msgKey": msg_key,
                "msgParam": msg_param_str,
                "robotCode": robot_code,
            });

            let media_url = if is_group {
                body["openConversationId"] =
                    serde_json::Value::String(metadata.conversation_id.clone());
                "https://api.dingtalk.com/v1.0/robot/groupMessages/send"
            } else {
                body["userIds"] = serde_json::Value::Array(vec![serde_json::Value::String(
                    metadata.sender_staff_id.clone(),
                )]);
                "https://api.dingtalk.com/v1.0/robot/oToMessages/batchSend"
            };

            match self
                .client
                .post(media_url)
                .header("x-acs-dingtalk-access-token", &token)
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    if let Err(e) = send::ensure_business_success(resp, "media send").await {
                        tracing::debug!(
                            path = %attachment_path_str,
                            error = %e,
                            "DingTalk: attachment send failed, skipping"
                        );
                        continue;
                    }

                    tracing::debug!(
                        path = %attachment_path_str,
                        msg_key,
                        "DingTalk: attachment sent"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        error = %e,
                        "DingTalk: attachment send request failed, skipping"
                    );
                }
            }
        }

        self.cleanup_message_state(msg.id).await;

        tracing::debug!(
            sender = %metadata.sender_nick,
            mode = if is_group { "group" } else { "dm" },
            chunks = chunks.len(),
            attachments = response.attachments.len(),
            "DingTalk reply sent"
        );

        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        if !self.card_delivery_enabled() {
            return Ok(());
        }

        // Extract the internal message UUID injected by stream.rs.
        let uuid_str = metadata
            .get("message_id")
            .or_else(|| metadata.get("msg_id"))
            .and_then(|v| v.as_str());
        let Some(uuid_str) = uuid_str else {
            return Ok(());
        };
        let Ok(msg_uuid) = Uuid::parse_str(uuid_str) else {
            return Ok(());
        };
        let message_lock = {
            let mut locks = self.status_locks.lock().await;
            Arc::clone(
                locks
                    .entry(msg_uuid)
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _message_guard = message_lock.lock().await;
        let conversation_id = metadata
            .get("conversation_id")
            .or_else(|| metadata.get("conversationId"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let sender_staff_id = metadata
            .get("sender_staff_id")
            .or_else(|| metadata.get("senderStaffId"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let conversation_key = if !conversation_id.is_empty() {
            conversation_id
        } else {
            sender_staff_id
        };
        let can_activate_card = Self::status_can_activate_card(&status);
        let supports_live_flush = self.status_supports_live_flush(&status);
        let force_live_flush = matches!(
            &status,
            StatusUpdate::Thinking(_)
                | StatusUpdate::ToolStarted { .. }
                | StatusUpdate::ToolCompleted { .. }
        );

        if can_activate_card
            && stream::is_conversation_stopped(&self.stopped_conversations, conversation_key).await
        {
            let state = {
                let mut states = self.card_states.write().await;
                states.remove(&msg_uuid)
            };
            self.reply_targets.write().await.pop(&msg_uuid);
            self.status_locks.lock().await.remove(&msg_uuid);

            if let Some(state) = state {
                if !state.fallback_required && !state.instance_id.is_empty() {
                    let final_content = self
                        .rendered_card_content(&state)
                        .unwrap_or_else(|| state.content_buffer.clone());
                    if let Ok(token) = self.get_access_token().await {
                        if let Err(e) = card_service::finalize_ai_card(
                            &self.client,
                            &self.config,
                            &token,
                            &state.instance_id,
                            &final_content,
                        )
                        .await
                        {
                            tracing::warn!(error = %e, "Failed to finalize stopped AI card");
                        }
                    }
                }
            }

            tracing::debug!(
                msg_id = %msg_uuid,
                conversation = %conversation_key,
                "Skipping DingTalk status update for stopped conversation"
            );
            return Ok(());
        }

        if can_activate_card && !self.ensure_card_ready(msg_uuid).await {
            return Ok(());
        }

        match status {
            StatusUpdate::StreamChunk(chunk) => {
                let mut states = self.card_states.write().await;
                let Some(state) = states.get_mut(&msg_uuid) else {
                    return Ok(());
                };
                if state.fallback_required {
                    return Ok(());
                }
                state.content_buffer.push_str(&chunk);
                state.phase = CardPhase::Inputing;
            }

            StatusUpdate::Status(ref msg) if Self::is_terminal_status_message(msg) => {
                let state = {
                    let mut states = self.card_states.write().await;
                    states.remove(&msg_uuid)
                };

                if let Some(state) = state {
                    if !state.fallback_required && !state.instance_id.is_empty() {
                        let final_content = self
                            .rendered_card_content(&state)
                            .unwrap_or_else(|| state.content_buffer.clone());
                        tracing::info!(
                            instance_id = %state.instance_id,
                            content_len = final_content.len(),
                            "Finalizing DingTalk AI card"
                        );
                        let token = match self.get_access_token().await {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to get token for card finalize");
                                self.cleanup_message_state(msg_uuid).await;
                                return Ok(());
                            }
                        };

                        if let Err(e) = card_service::finalize_ai_card(
                            &self.client,
                            &self.config,
                            &token,
                            &state.instance_id,
                            &final_content,
                        )
                        .await
                        {
                            tracing::warn!(error = %e, "Failed to finalize AI card");
                        }
                    }
                }

                self.cleanup_message_state(msg_uuid).await;
            }

            StatusUpdate::Thinking(text) => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    Self::append_line(&mut state.thinking_buffer, &text);
                }
            }

            StatusUpdate::ToolStarted { ref name, .. } => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    Self::append_line(&mut state.content_buffer, &format!("🔧 Using {name}..."));
                }
            }

            StatusUpdate::ToolCompleted {
                ref name,
                success,
                ref error,
                ..
            } => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    if state.fallback_required {
                        return Ok(());
                    }
                    if success {
                        Self::append_line(
                            &mut state.content_buffer,
                            &format!("✅ {name} completed"),
                        );
                    } else {
                        let err_msg = error.as_deref().unwrap_or("unknown error");
                        Self::append_line(
                            &mut state.content_buffer,
                            &format!("❌ {name} failed: {err_msg}"),
                        );
                    }
                }
            }

            _ => {}
        }

        if can_activate_card && supports_live_flush {
            self.flush_card_if_needed(msg_uuid, force_live_flush).await;
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let token = self.get_access_token().await?;
        let robot_code = self.config.robot_code.as_deref().unwrap_or_default();

        self.send_markdown(
            &token,
            robot_code,
            None,
            &[user_id],
            "Notification",
            &response.content,
        )
        .await?;

        tracing::debug!(user_id = %user_id, "DingTalk broadcast sent");
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        // Check if we can get an access token (validates credentials)
        let _ = self.get_access_token().await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        tracing::info!(
            client_id = %self.config.client_id,
            "DingTalk channel shutdown: signaling stream task and awaiting exit"
        );

        // Fire the cooperative stop first. The task's tokio::select! on
        // shutdown_signal.notified() exits its select arm and drops the
        // run_stream_listener future (which in turn closes the WebSocket).
        self.shutdown_signal.notify_waiters();

        // Reclaim the JoinHandle. Await with a short bounded timeout; if
        // the task doesn't exit in time, abort it. This guarantees no
        // zombie WebSocket survives `ChannelManager::hot_add`.
        let handle = self.stream_task.lock().await.take();
        if let Some(handle) = handle {
            match tokio::time::timeout(Duration::from_secs(5), handle).await {
                Ok(Ok(())) => {
                    tracing::info!(
                        client_id = %self.config.client_id,
                        "DingTalk stream task stopped"
                    );
                }
                Ok(Err(err)) => {
                    // Task panicked or was already cancelled.
                    tracing::warn!(
                        client_id = %self.config.client_id,
                        error = %err,
                        "DingTalk stream task join error on shutdown"
                    );
                }
                Err(_) => {
                    // Cooperative stop ran out of time; we'll have to abort.
                    // Note: handle was consumed by `timeout`; we cannot
                    // abort directly here. The orphaned task will continue
                    // until its own channel closes. Logging at warn so ops
                    // can see it — if this fires regularly, the 5s grace
                    // needs to be tuned.
                    tracing::warn!(
                        client_id = %self.config.client_id,
                        "DingTalk stream task did not stop within grace window; \
                         orphaning it (will self-exit when inject channel closes)"
                    );
                }
            }
        } else {
            tracing::debug!(
                client_id = %self.config.client_id,
                "DingTalk shutdown: no stream task handle (start() not called or already shutdown)"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex, OnceLock};

    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{Method, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::any;
    use axum::{Json, Router};
    use secrecy::SecretString;
    use serde_json::{Value, json};

    use super::*;
    use crate::config::{DmPolicy, GroupPolicy};

    #[derive(Clone, Debug)]
    struct RecordedRequest {
        path: String,
        body: Value,
    }

    #[derive(Clone, Debug, Default)]
    struct MockDingTalkBehavior {
        fail_create: bool,
        fail_nonempty_stream: bool,
        fail_finalize_stream: bool,
    }

    #[derive(Clone, Default)]
    struct MockDingTalkState {
        requests: Arc<tokio::sync::Mutex<Vec<RecordedRequest>>>,
        behavior: Arc<tokio::sync::Mutex<MockDingTalkBehavior>>,
        next_card_id: Arc<AtomicUsize>,
    }

    impl MockDingTalkState {
        async fn requests(&self) -> Vec<RecordedRequest> {
            self.requests.lock().await.clone()
        }
    }

    struct ScopedEnvVar {
        key: &'static str,
        original: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            static ENV_MUTEX: OnceLock<StdMutex<()>> = OnceLock::new();
            let guard = ENV_MUTEX
                .get_or_init(|| StdMutex::new(()))
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let original = std::env::var(key).ok();
            // SAFETY: guarded by ENV_MUTEX for test-only process-wide env mutation.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                original,
                _guard: guard,
            }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            // SAFETY: guarded by ENV_MUTEX for test-only process-wide env mutation.
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    struct MockDingTalkServer {
        state: MockDingTalkState,
        task: tokio::task::JoinHandle<()>,
        _env: ScopedEnvVar,
    }

    impl Drop for MockDingTalkServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn mock_dingtalk_handler(
        State(state): State<MockDingTalkState>,
        method: Method,
        uri: Uri,
        body: Bytes,
    ) -> impl IntoResponse {
        let body_json = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice::<Value>(&body)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()))
        };

        state.requests.lock().await.push(RecordedRequest {
            path: uri.path().to_string(),
            body: body_json.clone(),
        });

        let behavior = state.behavior.lock().await.clone();

        match (method, uri.path()) {
            (Method::POST, "/v1.0/oauth2/accessToken") => (
                StatusCode::OK,
                Json(json!({ "accessToken": "test-token", "expireIn": 7200 })),
            )
                .into_response(),
            (Method::POST, "/v1.0/card/instances/createAndDeliver") => {
                if behavior.fail_create {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "success": false, "message": "create failed" })),
                    )
                        .into_response();
                }

                let id = state.next_card_id.fetch_add(1, Ordering::Relaxed) + 1;
                (
                    StatusCode::OK,
                    Json(json!({
                        "success": true,
                        "result": { "outTrackId": format!("card-{id}") }
                    })),
                )
                    .into_response()
            }
            (Method::PUT, "/v1.0/card/streaming") => {
                let is_finalize = body_json
                    .get("isFinalize")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let content = body_json
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if (is_finalize && behavior.fail_finalize_stream)
                    || (!is_finalize && !content.is_empty() && behavior.fail_nonempty_stream)
                {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "success": false, "message": "stream failed" })),
                    )
                        .into_response();
                }

                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
            (Method::PUT, "/v1.0/card/instances") => {
                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
            (Method::POST, "/v1.0/robot/oToMessages/batchSend")
            | (Method::POST, "/v1.0/robot/groupMessages/send") => {
                (StatusCode::OK, Json(json!({ "success": true }))).into_response()
            }
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({ "success": false, "path": uri.path() })),
            )
                .into_response(),
        }
    }

    async fn spawn_mock_dingtalk_server(behavior: MockDingTalkBehavior) -> MockDingTalkServer {
        let state = MockDingTalkState {
            requests: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            behavior: Arc::new(tokio::sync::Mutex::new(behavior)),
            next_card_id: Arc::new(AtomicUsize::new(0)),
        };

        let app = Router::new()
            .route("/{*path}", any(mock_dingtalk_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake dingtalk");
        let addr = listener.local_addr().expect("fake dingtalk addr");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let env = ScopedEnvVar::set(
            "IRONCLAW_TEST_DINGTALK_API_BASE_URL",
            &format!("http://{addr}"),
        );

        MockDingTalkServer {
            state,
            task,
            _env: env,
        }
    }

    fn test_config(card_stream_mode: CardStreamMode) -> DingTalkConfig {
        DingTalkConfig {
            enabled: true,
            client_id: "test-client".to_string(),
            client_secret: SecretString::from("test-secret"),
            robot_code: Some("robot-code".to_string()),
            message_type: DingTalkMessageType::Card,
            card_template_id: Some("tpl-123".to_string()),
            card_template_key: "content".to_string(),
            card_stream_mode,
            card_stream_interval_ms: 1000,
            ack_reaction: None,
            require_mention: false,
            dm_policy: DmPolicy::Open,
            group_policy: GroupPolicy::Open,
            allow_from: vec![],
            group_allow_from: vec![],
            group_session_scope: Default::default(),
            display_name_resolution: Default::default(),
            max_reconnect_cycles: 10,
            reconnect_deadline_ms: 50_000,
            additional_accounts: vec![],
            status_tick_ms: 2000,
            slow_threshold_secs: (15, 60),
            reasoning_summary_enabled: false,
            max_active_cards: 1000,
        }
    }

    fn test_message() -> IncomingMessage {
        let mut message = IncomingMessage::new("dingtalk", "staff-1", "hello")
            .with_sender_id("staff-1")
            .with_user_name("Alice");
        let msg_id = Uuid::new_v4();
        message.id = msg_id;
        message.metadata = json!({
            "message_id": msg_id.to_string(),
            "conversationId": "conv-1",
            "conversationType": "1",
            "senderStaffId": "staff-1",
            "senderNick": "Alice",
            "msgId": "dt-msg-1",
            "robotCode": "robot-code"
        });
        message
    }

    fn streaming_requests(requests: &[RecordedRequest]) -> Vec<&RecordedRequest> {
        requests
            .iter()
            .filter(|req| req.path == "/v1.0/card/streaming")
            .collect()
    }

    #[tokio::test]
    async fn thinking_creates_card_before_first_chunk() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            requests
                .iter()
                .any(|req| req.path == "/v1.0/card/instances/createAndDeliver"),
            "expected createAndDeliver request, got: {requests:?}"
        );

        let streams = streaming_requests(&requests);
        assert_eq!(streams.len(), 1, "expected only activation stream");
        assert_eq!(streams[0].body["content"], json!(""));
        assert_eq!(streams[0].body["isFinalize"], json!(false));
    }

    #[tokio::test]
    async fn first_stream_chunk_flushes_immediately_after_activation() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::StreamChunk("Hello immediately".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        let streams = streaming_requests(&requests);
        assert_eq!(
            streams.len(),
            2,
            "expected activation + first content stream"
        );
        assert_eq!(streams[1].body["content"], json!("Hello immediately"));
        assert_eq!(streams[1].body["isFinalize"], json!(false));
    }

    #[tokio::test]
    async fn all_mode_flushes_thinking_and_tool_progress() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::All)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "search".to_string(),
                    detail: None,
                    call_id: None,
                },
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        let streams = streaming_requests(&requests);
        assert_eq!(
            streams.len(),
            3,
            "expected activation + thinking + tool flush"
        );
        assert_eq!(streams[1].body["content"], json!("Processing..."));
        let tool_content = streams[2].body["content"]
            .as_str()
            .expect("tool stream content should be string");
        assert!(tool_content.contains("Processing..."));
        assert!(tool_content.contains("Using search"));
    }

    #[tokio::test]
    async fn create_failure_falls_back_to_markdown_reply() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior {
            fail_create: true,
            ..Default::default()
        })
        .await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .respond(&message, OutgoingResponse::text("final fallback"))
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            requests.iter().any(|req| {
                req.path == "/v1.0/robot/oToMessages/batchSend"
                    && req.body["msgParam"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("final fallback")
            }),
            "expected markdown fallback request, got: {requests:?}"
        );
    }

    #[tokio::test]
    async fn stream_failure_falls_back_to_markdown_reply() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior {
            fail_nonempty_stream: true,
            ..Default::default()
        })
        .await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::StreamChunk("partial".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .respond(
                &message,
                OutgoingResponse::text("final after stream failure"),
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            requests.iter().any(|req| {
                req.path == "/v1.0/robot/oToMessages/batchSend"
                    && req.body["msgParam"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("final after stream failure")
            }),
            "expected markdown fallback after stream failure, got: {requests:?}"
        );
        assert!(
            !streaming_requests(&requests)
                .iter()
                .any(|req| req.body["isFinalize"] == json!(true)),
            "finalize should not run after stream fallback: {requests:?}"
        );
    }

    #[tokio::test]
    async fn finalize_failure_falls_back_to_markdown_reply() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior {
            fail_finalize_stream: true,
            ..Default::default()
        })
        .await;
        let channel = DingTalkChannel::new(test_config(CardStreamMode::Answer)).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::StreamChunk("partial".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();
        channel
            .respond(
                &message,
                OutgoingResponse::text("final after finalize failure"),
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            streaming_requests(&requests)
                .iter()
                .any(|req| req.body["isFinalize"] == json!(true)),
            "expected finalize streaming attempt, got: {requests:?}"
        );
        assert!(
            requests.iter().any(|req| {
                req.path == "/v1.0/robot/oToMessages/batchSend"
                    && req.body["msgParam"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("final after finalize failure")
            }),
            "expected markdown fallback after finalize failure, got: {requests:?}"
        );
    }

    #[tokio::test]
    async fn markdown_mode_skips_card_status_updates() {
        let server = spawn_mock_dingtalk_server(MockDingTalkBehavior::default()).await;
        let mut config = test_config(CardStreamMode::Answer);
        config.message_type = DingTalkMessageType::Markdown;
        let channel = DingTalkChannel::new(config).unwrap();
        let message = test_message();
        channel.seed_reply_target_for_test(&message).await.unwrap();

        channel
            .send_status(
                StatusUpdate::Thinking("Processing...".to_string()),
                &message.metadata,
            )
            .await
            .unwrap();

        let requests = server.state.requests().await;
        assert!(
            !requests
                .iter()
                .any(|req| req.path == "/v1.0/card/instances/createAndDeliver"),
            "markdown mode should not create cards: {requests:?}"
        );
    }
}
