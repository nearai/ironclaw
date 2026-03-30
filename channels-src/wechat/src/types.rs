use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WechatConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_cdn_base_url")]
    pub cdn_base_url: String,
    #[serde(default = "default_bot_type")]
    pub bot_type: String,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u32,
    #[serde(default = "default_long_poll_timeout_ms")]
    pub long_poll_timeout_ms: u32,
    #[serde(default = "default_inbound_merge_window_ms")]
    pub inbound_merge_window_ms: u32,
}

fn default_base_url() -> String {
    "https://ilinkai.weixin.qq.com".to_string()
}

fn default_cdn_base_url() -> String {
    "https://novac2c.cdn.weixin.qq.com/c2c".to_string()
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

fn default_inbound_merge_window_ms() -> u32 {
    5_000
}

impl Default for WechatConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            cdn_base_url: default_cdn_base_url(),
            bot_type: default_bot_type(),
            poll_interval_ms: default_poll_interval_ms(),
            long_poll_timeout_ms: default_long_poll_timeout_ms(),
            inbound_merge_window_ms: default_inbound_merge_window_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BaseInfo {
    pub channel_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GetUploadUrlRequest {
    pub filekey: String,
    pub media_type: i32,
    pub to_user_id: String,
    pub rawsize: u64,
    pub rawfilemd5: String,
    pub filesize: u64,
    pub no_need_thumb: bool,
    pub aeskey: String,
    pub base_info: BaseInfo,
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
    pub ret: Option<i32>,
    pub errcode: Option<i32>,
    pub errmsg: Option<String>,
    #[serde(default)]
    pub msgs: Vec<WechatMessage>,
    pub get_updates_buf: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetUploadUrlResponse {
    pub upload_param: Option<String>,
    pub thumb_upload_param: Option<String>,
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
    pub message_id: Option<i64>,
    pub from_user_id: Option<String>,
    pub to_user_id: Option<String>,
    pub session_id: Option<String>,
    pub message_type: Option<i32>,
    pub context_token: Option<String>,
    #[serde(default)]
    pub item_list: Vec<MessageItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetConfigResponse {
    pub ret: Option<i32>,
    pub errmsg: Option<String>,
    pub typing_ticket: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendTypingResponse {
    pub ret: Option<i32>,
    pub errmsg: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MessageItem {
    pub r#type: Option<i32>,
    pub text_item: Option<TextItem>,
    pub image_item: Option<ImageItem>,
    pub voice_item: Option<VoiceItem>,
    pub file_item: Option<FileItem>,
    pub video_item: Option<VideoItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextItem {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CdnMedia {
    pub encrypt_query_param: Option<String>,
    pub aes_key: Option<String>,
    pub encrypt_type: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageItem {
    pub media: Option<CdnMedia>,
    pub aeskey: Option<String>,
    pub mid_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VoiceItem {
    pub media: Option<CdnMedia>,
    pub encode_type: Option<i32>,
    pub playtime: Option<u64>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileItem {
    pub media: Option<CdnMedia>,
    pub file_name: Option<String>,
    pub len: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoItem {
    pub media: Option<CdnMedia>,
    pub video_size: Option<u64>,
    pub play_length: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutboundMetadata {
    pub from_user_id: String,
    pub to_user_id: Option<String>,
    pub message_id: Option<i64>,
    pub session_id: Option<String>,
    pub context_token: Option<String>,
}

pub const MESSAGE_TYPE_USER: i32 = 1;
pub const MESSAGE_TYPE_BOT: i32 = 2;
pub const MESSAGE_STATE_FINISH: i32 = 2;
pub const MESSAGE_ITEM_TEXT: i32 = 1;
pub const MESSAGE_ITEM_IMAGE: i32 = 2;
pub const MESSAGE_ITEM_VOICE: i32 = 3;
pub const MESSAGE_ITEM_FILE: i32 = 4;
pub const MESSAGE_ITEM_VIDEO: i32 = 5;
pub const TYPING_STATUS_TYPING: i32 = 1;
pub const TYPING_STATUS_CANCEL: i32 = 2;
pub const UPLOAD_MEDIA_TYPE_IMAGE: i32 = 1;
pub const UPLOAD_MEDIA_TYPE_VIDEO: i32 = 2;
pub const UPLOAD_MEDIA_TYPE_FILE: i32 = 3;
