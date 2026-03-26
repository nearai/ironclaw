use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WechatConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_bot_type")]
    pub bot_type: String,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u32,
    #[serde(default = "default_long_poll_timeout_ms")]
    pub long_poll_timeout_ms: u32,
}

fn default_base_url() -> String {
    "https://ilinkai.weixin.qq.com".to_string()
}

fn default_bot_type() -> String {
    "3".to_string()
}

fn default_poll_interval_ms() -> u32 {
    30_000
}

fn default_long_poll_timeout_ms() -> u32 {
    35_000
}

impl Default for WechatConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            bot_type: default_bot_type(),
            poll_interval_ms: default_poll_interval_ms(),
            long_poll_timeout_ms: default_long_poll_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BaseInfo {
    pub channel_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GetUpdatesRequest {
    pub get_updates_buf: String,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GetConfigRequest {
    pub ilink_user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_token: Option<String>,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetUpdatesResponse {
    #[serde(default)]
    pub ret: Option<i32>,
    #[serde(default)]
    pub errcode: Option<i32>,
    #[serde(default)]
    pub errmsg: Option<String>,
    #[serde(default)]
    pub msgs: Vec<WechatMessage>,
    #[serde(default)]
    pub get_updates_buf: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SendMessageRequest {
    pub msg: OutboundWechatMessage,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SendTypingRequest {
    pub ilink_user_id: String,
    pub typing_ticket: String,
    pub status: i32,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutboundWechatMessage {
    pub from_user_id: String,
    pub to_user_id: String,
    pub client_id: String,
    pub message_type: i32,
    pub message_state: i32,
    pub item_list: Vec<MessageItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WechatMessage {
    #[serde(default)]
    pub message_id: Option<i64>,
    #[serde(default)]
    pub from_user_id: Option<String>,
    #[serde(default)]
    pub to_user_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub message_type: Option<i32>,
    #[serde(default)]
    pub context_token: Option<String>,
    #[serde(default)]
    pub item_list: Vec<MessageItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetConfigResponse {
    #[serde(default)]
    pub ret: Option<i32>,
    #[serde(default)]
    pub errmsg: Option<String>,
    #[serde(default)]
    pub typing_ticket: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendTypingResponse {
    #[serde(default)]
    pub ret: Option<i32>,
    #[serde(default)]
    pub errmsg: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MessageItem {
    #[serde(default)]
    pub r#type: Option<i32>,
    #[serde(default)]
    pub text_item: Option<TextItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextItem {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutboundMetadata {
    pub from_user_id: String,
    #[serde(default)]
    pub to_user_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<i64>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub context_token: Option<String>,
}

pub const MESSAGE_TYPE_USER: i32 = 1;
pub const MESSAGE_TYPE_BOT: i32 = 2;
pub const MESSAGE_STATE_FINISH: i32 = 2;
pub const MESSAGE_ITEM_TEXT: i32 = 1;
pub const TYPING_STATUS_TYPING: i32 = 1;
pub const TYPING_STATUS_CANCEL: i32 = 2;
