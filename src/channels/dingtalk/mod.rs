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
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::{CardStreamMode, DingTalkConfig};
use crate::error::ChannelError;

use types::{AccessTokenResponse, CardPhase, CardState, DingTalkMetadata, MarkdownMsgParam};

const MAX_REPLY_TARGETS: usize = 10000;
const REPLY_TARGETS_CAP: NonZeroUsize = NonZeroUsize::new(MAX_REPLY_TARGETS).unwrap();

/// DingTalk channel using Stream mode (persistent WebSocket).
pub struct DingTalkChannel {
    config: DingTalkConfig,
    client: Client,
    reply_targets: Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
    /// Cached access token with expiry.
    access_token: Arc<RwLock<Option<(String, std::time::Instant)>>>,
    /// Active AI card states, keyed by message UUID.
    card_states: Arc<RwLock<std::collections::HashMap<Uuid, CardState>>>,
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
        })
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
            .post("https://api.dingtalk.com/v1.0/oauth2/accessToken")
            .json(&serde_json::json!({
                "appKey": self.config.client_id,
                "appSecret": self.config.client_secret.expose_secret(),
            }))
            .send()
            .await
            .map_err(|e| ChannelError::Http(format!("token request: {e}")))?;

        if !resp.status().is_success() {
            return Err(ChannelError::Http(format!(
                "token API returned {}",
                resp.status()
            )));
        }

        let token_resp: AccessTokenResponse = resp
            .json()
            .await
            .map_err(|e| ChannelError::Http(format!("parse token: {e}")))?;

        let token = token_resp
            .access_token
            .ok_or_else(|| ChannelError::Http("no access_token in response".to_string()))?;

        let expires_in = token_resp.expires_in.unwrap_or(7200);
        // Refresh 5 minutes before expiry
        let expiry =
            std::time::Instant::now() + Duration::from_secs(expires_in.saturating_sub(300));

        let mut cache = self.access_token.write().await;
        *cache = Some((token.clone(), expiry));

        Ok(token)
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
            "https://api.dingtalk.com/v1.0/robot/groupMessages/send"
        } else {
            body["userIds"] = serde_json::Value::Array(
                user_ids
                    .iter()
                    .map(|u| serde_json::Value::String(u.to_string()))
                    .collect(),
            );
            "https://api.dingtalk.com/v1.0/robot/oToMessages/batchSend"
        };

        let resp = self
            .client
            .post(url)
            .header("x-acs-dingtalk-access-token", token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Http(format!("send: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::Http(format!(
                "Robot API returned {status}: {body_text}"
            )));
        }

        Ok(())
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

        tokio::spawn(async move {
            if let Err(e) = stream::run_stream_listener(config, client, tx, reply_targets).await {
                tracing::error!("DingTalk Stream listener exited with error: {e}");
            }
        });

        tracing::debug!(
            client_id = %self.config.client_id,
            "DingTalk channel started (Stream mode)"
        );

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let metadata = {
            let targets = self.reply_targets.read().await;
            targets.peek(&msg.id).cloned()
        };

        let Some(metadata) = metadata else {
            tracing::warn!(msg_id = %msg.id, "No reply metadata found for DingTalk message");
            return Ok(());
        };

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
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(
                        path = %attachment_path_str,
                        msg_key,
                        "DingTalk: attachment sent"
                    );
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body_text = resp.text().await.unwrap_or_default();
                    tracing::debug!(
                        path = %attachment_path_str,
                        status = %status,
                        response = %body_text,
                        "DingTalk: attachment send failed, skipping"
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

        // Clean up reply target
        self.reply_targets.write().await.pop(&msg.id);

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
        // Extract message UUID from metadata
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

        match status {
            StatusUpdate::StreamChunk(chunk) => {
                // No card mode if template not configured
                if self.config.card_template_id.is_none() {
                    return Ok(());
                }

                let mut states = self.card_states.write().await;

                // If no CardState exists yet, create the card
                if !states.contains_key(&msg_uuid) {
                    // Look up conversation metadata from reply_targets
                    let reply_meta = {
                        let targets = self.reply_targets.read().await;
                        targets.peek(&msg_uuid).cloned()
                    };
                    let Some(reply_meta) = reply_meta else {
                        tracing::debug!(msg_id = %msg_uuid, "No reply metadata for card creation");
                        return Ok(());
                    };

                    let token = match self.get_access_token().await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::debug!(error = %e, "Failed to get token for card creation");
                            return Ok(());
                        }
                    };

                    let instance_id = match card_service::create_ai_card(
                        &self.client,
                        &self.config,
                        &token,
                        &reply_meta.conversation_id,
                        &reply_meta.conversation_type,
                    )
                    .await
                    {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::debug!(error = %e, "Failed to create AI card, will fall back to Markdown");
                            return Ok(());
                        }
                    };

                    states.insert(
                        msg_uuid,
                        CardState {
                            instance_id,
                            content_buffer: String::new(),
                            thinking_buffer: String::new(),
                            last_update: std::time::Instant::now(),
                            phase: CardPhase::Processing,
                            conversation_id: reply_meta.conversation_id.clone(),
                            conversation_type: reply_meta.conversation_type.clone(),
                        },
                    );
                }

                let Some(state) = states.get_mut(&msg_uuid) else {
                    return Ok(());
                };

                state.content_buffer.push_str(&chunk);

                if state.phase == CardPhase::Processing {
                    state.phase = CardPhase::Inputing;
                }

                // Check if we should stream an update based on mode and throttle
                let interval = Duration::from_millis(self.config.card_stream_interval_ms);
                let should_stream = match self.config.card_stream_mode {
                    CardStreamMode::Off => false,
                    CardStreamMode::Answer | CardStreamMode::All => {
                        std::time::Instant::now().duration_since(state.last_update) >= interval
                    }
                };

                if should_stream {
                    let content = if self.config.card_stream_mode == CardStreamMode::All
                        && !state.thinking_buffer.is_empty()
                    {
                        format!("{}\n\n{}", state.thinking_buffer, state.content_buffer)
                    } else {
                        state.content_buffer.clone()
                    };

                    let instance_id = state.instance_id.clone();
                    state.last_update = std::time::Instant::now();

                    // Drop the lock before making the API call
                    drop(states);

                    let token = match self.get_access_token().await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::debug!(error = %e, "Failed to get token for card stream");
                            return Ok(());
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
                        tracing::debug!(error = %e, "Failed to stream AI card update");
                    }
                }
            }

            StatusUpdate::Status(ref msg) if msg.contains("done") => {
                let state = {
                    let mut states = self.card_states.write().await;
                    states.remove(&msg_uuid)
                };

                if let Some(state) = state {
                    let token = match self.get_access_token().await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::debug!(error = %e, "Failed to get token for card finalize");
                            return Ok(());
                        }
                    };

                    if let Err(e) = card_service::stream_ai_card(
                        &self.client,
                        &token,
                        &state.instance_id,
                        &state.content_buffer,
                        &self.config.card_template_key,
                        true,
                        false,
                    )
                    .await
                    {
                        tracing::debug!(error = %e, "Failed to finalize AI card");
                    }
                }
            }

            StatusUpdate::Thinking(text) => {
                if self.config.card_stream_mode == CardStreamMode::All {
                    let mut states = self.card_states.write().await;
                    if let Some(state) = states.get_mut(&msg_uuid) {
                        state.thinking_buffer.push_str(&text);
                    }
                }
            }

            StatusUpdate::ToolStarted { ref name, .. } => {
                let mut states = self.card_states.write().await;
                if let Some(state) = states.get_mut(&msg_uuid) {
                    state
                        .content_buffer
                        .push_str(&format!("\n🔧 Using {name}..."));
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
                    if success {
                        state
                            .content_buffer
                            .push_str(&format!("\n✅ {name} completed"));
                    } else {
                        let err_msg = error.as_deref().unwrap_or("unknown error");
                        state
                            .content_buffer
                            .push_str(&format!("\n❌ {name} failed: {err_msg}"));
                    }
                }
            }

            _ => {}
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
        tracing::debug!("DingTalk channel shutting down");
        Ok(())
    }
}
