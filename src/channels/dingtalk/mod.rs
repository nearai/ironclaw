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
use crate::config::DingTalkConfig;
use crate::error::ChannelError;

use types::{DingTalkMetadata, MarkdownMsgParam, AccessTokenResponse};

const MAX_REPLY_TARGETS: usize = 10000;
const REPLY_TARGETS_CAP: NonZeroUsize = NonZeroUsize::new(MAX_REPLY_TARGETS).unwrap();

/// DingTalk channel using Stream mode (persistent WebSocket).
pub struct DingTalkChannel {
    config: DingTalkConfig,
    client: Client,
    reply_targets: Arc<RwLock<LruCache<Uuid, DingTalkMetadata>>>,
    /// Cached access token with expiry.
    access_token: Arc<RwLock<Option<(String, std::time::Instant)>>>,
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

        let token = token_resp.access_token.ok_or_else(|| {
            ChannelError::Http("no access_token in response".to_string())
        })?;

        let expires_in = token_resp.expires_in.unwrap_or(7200);
        // Refresh 5 minutes before expiry
        let expiry = std::time::Instant::now() + Duration::from_secs(expires_in.saturating_sub(300));

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
                user_ids.iter().map(|u| serde_json::Value::String(u.to_string())).collect(),
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

        tracing::info!(
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

        let robot_code = metadata.robot_code.as_deref()
            .or(self.config.robot_code.as_deref())
            .unwrap_or_default();

        let token = self.get_access_token().await?;
        let is_group = metadata.conversation_type == "2";

        let user_ids_vec = vec![metadata.sender_staff_id.as_str()];
        self.send_markdown(
            &token,
            robot_code,
            if is_group { Some(&metadata.conversation_id) } else { None },
            if is_group { &[] } else { &user_ids_vec },
            "Reply",
            &response.content,
        )
        .await?;

        // Clean up reply target
        self.reply_targets.write().await.pop(&msg.id);

        tracing::debug!(
            sender = %metadata.sender_nick,
            mode = if is_group { "group" } else { "dm" },
            "DingTalk reply sent"
        );

        Ok(())
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // DingTalk doesn't have a native typing indicator API for bots.
        // Could implement via AI Card status in the future.
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
        tracing::info!("DingTalk channel shutting down");
        Ok(())
    }
}
