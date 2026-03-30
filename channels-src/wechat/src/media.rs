use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
use aes::Aes128;
use base64::Engine as _;
use md5::{Digest, Md5};
use rand::RngCore;
use serde_json::json;

use crate::exports::near::agent::channel::Attachment;
use crate::near::agent::channel_host::{self, InboundAttachment};
use crate::types::{
    CdnMedia, FileItem, ImageItem, MessageItem, SendMessageRequest, VideoItem, WechatConfig,
    MESSAGE_ITEM_FILE, MESSAGE_ITEM_IMAGE, MESSAGE_ITEM_VIDEO, MESSAGE_ITEM_VOICE,
    MESSAGE_STATE_FINISH, MESSAGE_TYPE_BOT, UPLOAD_MEDIA_TYPE_FILE, UPLOAD_MEDIA_TYPE_IMAGE,
    UPLOAD_MEDIA_TYPE_VIDEO,
};

const AES_BLOCK_SIZE: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundMediaKind {
    Image,
    Video,
    File,
}

#[derive(Debug, Clone)]
pub struct UploadedMedia {
    pub download_encrypted_query_param: String,
    pub aes_key_base64: String,
    pub file_size_ciphertext: u64,
    pub plaintext_size: u64,
}

pub fn extract_inbound_attachments(
    config: &WechatConfig,
    message: &crate::types::WechatMessage,
) -> Result<Vec<InboundAttachment>, String> {
    message
        .item_list
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            map_inbound_attachment(config, message, item, index).transpose()
        })
        .collect()
}

pub fn send_image_attachment(
    config: &WechatConfig,
    to_user_id: &str,
    attachment: &Attachment,
    context_token: Option<&str>,
    text: &str,
) -> Result<(), String> {
    if attachment.data.is_empty() {
        return Err(format!(
            "WeChat image attachment '{}' has no data",
            attachment.filename
        ));
    }

    let upload = upload_media_attachment(config, to_user_id, attachment, UPLOAD_MEDIA_TYPE_IMAGE)?;
    if !text.trim().is_empty() {
        crate::api::send_text_message(config, to_user_id, text.trim(), context_token)?;
    }
    let request = SendMessageRequest {
        msg: crate::types::OutboundWechatMessage {
            from_user_id: String::new(),
            to_user_id: to_user_id.to_string(),
            client_id: format!("wechat-{}", channel_host::now_millis()),
            message_type: MESSAGE_TYPE_BOT,
            message_state: MESSAGE_STATE_FINISH,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_IMAGE),
                text_item: None,
                image_item: Some(ImageItem {
                    media: Some(CdnMedia {
                        encrypt_query_param: Some(upload.download_encrypted_query_param.clone()),
                        aes_key: Some(upload.aes_key_base64.clone()),
                        encrypt_type: Some(1),
                    }),
                    aeskey: None,
                    mid_size: Some(upload.file_size_ciphertext),
                }),
                voice_item: None,
                file_item: None,
                video_item: None,
            }],
            context_token: context_token.map(str::to_string),
        },
        base_info: crate::api::base_info(),
    };

    crate::api::send_message_request(config, &request)
}

pub fn send_video_attachment(
    config: &WechatConfig,
    to_user_id: &str,
    attachment: &Attachment,
    context_token: Option<&str>,
    text: &str,
) -> Result<(), String> {
    let upload = upload_media_attachment(config, to_user_id, attachment, UPLOAD_MEDIA_TYPE_VIDEO)?;
    if !text.trim().is_empty() {
        crate::api::send_text_message(config, to_user_id, text.trim(), context_token)?;
    }
    let request = SendMessageRequest {
        msg: crate::types::OutboundWechatMessage {
            from_user_id: String::new(),
            to_user_id: to_user_id.to_string(),
            client_id: format!("wechat-{}", channel_host::now_millis()),
            message_type: MESSAGE_TYPE_BOT,
            message_state: MESSAGE_STATE_FINISH,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_VIDEO),
                text_item: None,
                image_item: None,
                voice_item: None,
                file_item: None,
                video_item: Some(VideoItem {
                    media: Some(CdnMedia {
                        encrypt_query_param: Some(upload.download_encrypted_query_param.clone()),
                        aes_key: Some(upload.aes_key_base64.clone()),
                        encrypt_type: Some(1),
                    }),
                    video_size: Some(upload.file_size_ciphertext),
                    play_length: None,
                }),
            }],
            context_token: context_token.map(str::to_string),
        },
        base_info: crate::api::base_info(),
    };

    crate::api::send_message_request(config, &request)
}

pub fn send_file_attachment(
    config: &WechatConfig,
    to_user_id: &str,
    attachment: &Attachment,
    context_token: Option<&str>,
    text: &str,
) -> Result<(), String> {
    let upload = upload_media_attachment(config, to_user_id, attachment, UPLOAD_MEDIA_TYPE_FILE)?;
    if !text.trim().is_empty() {
        crate::api::send_text_message(config, to_user_id, text.trim(), context_token)?;
    }
    let request = SendMessageRequest {
        msg: crate::types::OutboundWechatMessage {
            from_user_id: String::new(),
            to_user_id: to_user_id.to_string(),
            client_id: format!("wechat-{}", channel_host::now_millis()),
            message_type: MESSAGE_TYPE_BOT,
            message_state: MESSAGE_STATE_FINISH,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_FILE),
                text_item: None,
                image_item: None,
                voice_item: None,
                file_item: Some(FileItem {
                    media: Some(CdnMedia {
                        encrypt_query_param: Some(upload.download_encrypted_query_param.clone()),
                        aes_key: Some(upload.aes_key_base64.clone()),
                        encrypt_type: Some(1),
                    }),
                    file_name: Some(normalize_outbound_file_name(attachment)),
                    len: Some(upload.plaintext_size.to_string()),
                }),
                video_item: None,
            }],
            context_token: context_token.map(str::to_string),
        },
        base_info: crate::api::base_info(),
    };

    crate::api::send_message_request(config, &request)
}

pub fn classify_outbound_media_kind(mime_type: &str) -> OutboundMediaKind {
    if mime_type.starts_with("image/") {
        OutboundMediaKind::Image
    } else if mime_type.starts_with("video/") {
        OutboundMediaKind::Video
    } else {
        OutboundMediaKind::File
    }
}

fn map_inbound_attachment(
    config: &WechatConfig,
    message: &crate::types::WechatMessage,
    item: &MessageItem,
    index: usize,
) -> Result<Option<InboundAttachment>, String> {
    if item.r#type == Some(MESSAGE_ITEM_IMAGE) {
        return map_image_attachment(config, message, item, index);
    }
    if item.r#type == Some(MESSAGE_ITEM_VOICE) {
        return map_voice_attachment(config, message, item, index);
    }
    if item.r#type == Some(MESSAGE_ITEM_FILE) {
        return map_file_attachment(config, message, item, index);
    }
    if item.r#type == Some(MESSAGE_ITEM_VIDEO) {
        return map_video_attachment(config, message, item, index);
    }
    Ok(None)
}

fn map_image_attachment(
    config: &WechatConfig,
    message: &crate::types::WechatMessage,
    item: &MessageItem,
    index: usize,
) -> Result<Option<InboundAttachment>, String> {
    if item.r#type != Some(MESSAGE_ITEM_IMAGE) {
        return Ok(None);
    }

    let image = item.image_item.as_ref().ok_or_else(|| {
        format!(
            "WeChat image message {:?} is missing image_item payload",
            message.message_id
        )
    })?;
    let media = image.media.as_ref().ok_or_else(|| {
        format!(
            "WeChat image message {:?} is missing media payload",
            message.message_id
        )
    })?;
    let encrypt_query_param = media.encrypt_query_param.as_deref().ok_or_else(|| {
        format!(
            "WeChat image message {:?} is missing encrypt_query_param",
            message.message_id
        )
    })?;
    let message_id = message
        .message_id
        .ok_or_else(|| "WeChat image message is missing message_id".to_string())?;
    let aes_key = preferred_image_aes_key(image, media).map(str::to_string);

    Ok(Some(InboundAttachment {
        id: format!("wechat-image-{}-{}", message_id, index),
        mime_type: "image/jpeg".to_string(),
        filename: Some(format!("wechat-image-{}-{}.jpg", message_id, index)),
        size_bytes: image.mid_size,
        source_url: Some(build_cdn_download_url(
            &config.cdn_base_url,
            encrypt_query_param,
        )),
        storage_key: None,
        extracted_text: None,
        extras_json: json!({ "wechat_aes_key": aes_key }).to_string(),
    }))
}

fn map_file_attachment(
    config: &WechatConfig,
    message: &crate::types::WechatMessage,
    item: &MessageItem,
    index: usize,
) -> Result<Option<InboundAttachment>, String> {
    if item.r#type != Some(MESSAGE_ITEM_FILE) {
        return Ok(None);
    }

    let file = item.file_item.as_ref().ok_or_else(|| {
        format!(
            "WeChat file message {:?} is missing file_item payload",
            message.message_id
        )
    })?;
    let media = file.media.as_ref().ok_or_else(|| {
        format!(
            "WeChat file message {:?} is missing media payload",
            message.message_id
        )
    })?;
    let encrypt_query_param = media.encrypt_query_param.as_deref().ok_or_else(|| {
        format!(
            "WeChat file message {:?} is missing encrypt_query_param",
            message.message_id
        )
    })?;
    let aes_key = media
        .aes_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "WeChat file message {:?} is missing aes_key",
                message.message_id
            )
        })?;
    let message_id = message
        .message_id
        .ok_or_else(|| "WeChat file message is missing message_id".to_string())?;
    let filename = inbound_file_name(file, message_id, index);
    let size_bytes = file.len.as_deref().and_then(parse_file_size);

    Ok(Some(InboundAttachment {
        id: format!("wechat-file-{}-{}", message_id, index),
        mime_type: infer_file_mime_type(&filename),
        filename: Some(filename),
        size_bytes,
        source_url: Some(build_cdn_download_url(
            &config.cdn_base_url,
            encrypt_query_param,
        )),
        storage_key: None,
        extracted_text: None,
        extras_json: json!({ "wechat_aes_key": aes_key }).to_string(),
    }))
}

fn map_voice_attachment(
    config: &WechatConfig,
    message: &crate::types::WechatMessage,
    item: &MessageItem,
    index: usize,
) -> Result<Option<InboundAttachment>, String> {
    if item.r#type != Some(MESSAGE_ITEM_VOICE) {
        return Ok(None);
    }

    let voice = item.voice_item.as_ref().ok_or_else(|| {
        format!(
            "WeChat voice message {:?} is missing voice_item payload",
            message.message_id
        )
    })?;
    let media = voice.media.as_ref().ok_or_else(|| {
        format!(
            "WeChat voice message {:?} is missing media payload",
            message.message_id
        )
    })?;
    let encrypt_query_param = media.encrypt_query_param.as_deref().ok_or_else(|| {
        format!(
            "WeChat voice message {:?} is missing encrypt_query_param",
            message.message_id
        )
    })?;
    let aes_key = media
        .aes_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "WeChat voice message {:?} is missing aes_key",
                message.message_id
            )
        })?;
    let message_id = message
        .message_id
        .ok_or_else(|| "WeChat voice message is missing message_id".to_string())?;
    let (mime_type, extension) = infer_voice_media_type(voice.encode_type);
    let duration_secs = voice.playtime.map(|millis| (millis / 1000) as u32);

    Ok(Some(InboundAttachment {
        id: format!("wechat-voice-{}-{}", message_id, index),
        mime_type: mime_type.to_string(),
        filename: Some(format!(
            "wechat-voice-{}-{}.{}",
            message_id, index, extension
        )),
        size_bytes: None,
        source_url: Some(build_cdn_download_url(
            &config.cdn_base_url,
            encrypt_query_param,
        )),
        storage_key: None,
        extracted_text: voice
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        extras_json: build_voice_extras_json(aes_key, duration_secs),
    }))
}

fn map_video_attachment(
    config: &WechatConfig,
    message: &crate::types::WechatMessage,
    item: &MessageItem,
    index: usize,
) -> Result<Option<InboundAttachment>, String> {
    if item.r#type != Some(MESSAGE_ITEM_VIDEO) {
        return Ok(None);
    }

    let video = item.video_item.as_ref().ok_or_else(|| {
        format!(
            "WeChat video message {:?} is missing video_item payload",
            message.message_id
        )
    })?;
    let media = video.media.as_ref().ok_or_else(|| {
        format!(
            "WeChat video message {:?} is missing media payload",
            message.message_id
        )
    })?;
    let encrypt_query_param = media.encrypt_query_param.as_deref().ok_or_else(|| {
        format!(
            "WeChat video message {:?} is missing encrypt_query_param",
            message.message_id
        )
    })?;
    let aes_key = media
        .aes_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "WeChat video message {:?} is missing aes_key",
                message.message_id
            )
        })?;
    let message_id = message
        .message_id
        .ok_or_else(|| "WeChat video message is missing message_id".to_string())?;

    Ok(Some(InboundAttachment {
        id: format!("wechat-video-{}-{}", message_id, index),
        mime_type: "video/mp4".to_string(),
        filename: Some(format!("wechat-video-{}-{}.mp4", message_id, index)),
        size_bytes: video.video_size,
        source_url: Some(build_cdn_download_url(
            &config.cdn_base_url,
            encrypt_query_param,
        )),
        storage_key: None,
        extracted_text: None,
        extras_json: json!({ "wechat_aes_key": aes_key }).to_string(),
    }))
}

fn preferred_image_aes_key<'a>(image: &'a ImageItem, media: &'a CdnMedia) -> Option<&'a str> {
    image
        .aeskey
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            media
                .aes_key
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
}

fn inbound_file_name(file: &FileItem, message_id: i64, index: usize) -> String {
    file.file_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("wechat-file-{}-{}.bin", message_id, index))
}

fn parse_file_size(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
}

fn infer_voice_media_type(encode_type: Option<i32>) -> (&'static str, &'static str) {
    match encode_type {
        Some(7) => ("audio/mpeg", "mp3"),
        Some(8) => ("audio/ogg", "ogg"),
        Some(5) => ("audio/amr", "amr"),
        Some(6) => ("audio/silk", "silk"),
        _ => ("audio/silk", "silk"),
    }
}

fn build_voice_extras_json(aes_key: &str, duration_secs: Option<u32>) -> String {
    let mut extras = serde_json::Map::new();
    extras.insert("wechat_aes_key".to_string(), json!(aes_key));
    if let Some(duration_secs) = duration_secs {
        extras.insert("duration_secs".to_string(), json!(duration_secs));
    }
    serde_json::Value::Object(extras).to_string()
}

fn infer_file_mime_type(filename: &str) -> String {
    let extension = filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_ascii_lowercase());

    match extension.as_deref() {
        Some("pdf") => "application/pdf",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("md") => "text/markdown",
        Some("zip") => "application/zip",
        Some("tar") => "application/x-tar",
        Some("gz") => "application/gzip",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn upload_media_attachment(
    config: &WechatConfig,
    to_user_id: &str,
    attachment: &Attachment,
    media_type: i32,
) -> Result<UploadedMedia, String> {
    let plaintext = &attachment.data;
    if plaintext.is_empty() {
        return Err(format!(
            "WeChat attachment '{}' has no data",
            attachment.filename
        ));
    }
    let raw_size = plaintext.len() as u64;
    let raw_md5 = hex_lower(md5_bytes(plaintext));
    let file_size_ciphertext = padded_size(raw_size);
    let filekey = hex_lower(random_bytes(16)?);
    let aes_key = random_bytes(16)?;
    let aes_key_hex = hex_lower(aes_key.clone());

    let upload_url = crate::api::get_upload_url(
        config,
        &crate::types::GetUploadUrlRequest {
            filekey: filekey.clone(),
            media_type,
            to_user_id: to_user_id.to_string(),
            rawsize: raw_size,
            rawfilemd5: raw_md5,
            filesize: file_size_ciphertext,
            no_need_thumb: true,
            aeskey: aes_key_hex,
            base_info: crate::api::base_info(),
        },
    )?;

    let upload_param = upload_url
        .upload_param
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "WeChat getUploadUrl returned no upload_param".to_string())?;

    if upload_url.thumb_upload_param.is_some() {
        channel_host::log(
            channel_host::LogLevel::Debug,
            "WeChat image upload returned thumb_upload_param; ignoring for single-image flow",
        );
    }

    let ciphertext = encrypt_aes_ecb_pkcs7(plaintext, &aes_key)?;
    let upload_response = channel_host::http_request(
        "POST",
        &build_cdn_upload_url(&config.cdn_base_url, upload_param, &filekey),
        r#"{"Content-Type":"application/octet-stream"}"#,
        Some(&ciphertext),
        Some(15_000),
    )
    .map_err(|e| format!("WeChat CDN upload failed: {e}"))?;

    if upload_response.status != 200 {
        let body = String::from_utf8_lossy(&upload_response.body);
        return Err(format!(
            "WeChat CDN upload returned {}: {}",
            upload_response.status, body
        ));
    }

    let headers: std::collections::HashMap<String, String> =
        serde_json::from_str(&upload_response.headers_json)
            .map_err(|e| format!("Failed to parse WeChat CDN upload headers: {e}"))?;
    let download_encrypted_query_param = headers
        .iter()
        .find_map(|(key, value)| {
            if key.eq_ignore_ascii_case("x-encrypted-param") {
                Some(value.clone())
            } else {
                None
            }
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "WeChat CDN upload response missing x-encrypted-param".to_string())?;

    Ok(UploadedMedia {
        download_encrypted_query_param,
        aes_key_base64: base64::engine::general_purpose::STANDARD.encode(aes_key),
        file_size_ciphertext,
        plaintext_size: raw_size,
    })
}

fn normalize_outbound_file_name(attachment: &Attachment) -> String {
    let trimmed = attachment.filename.trim();
    if trimmed.is_empty() {
        "attachment.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_cdn_download_url(cdn_base_url: &str, encrypted_query_param: &str) -> String {
    format!(
        "{}/download?encrypted_query_param={}",
        cdn_base_url.trim_end_matches('/'),
        percent_encode(encrypted_query_param)
    )
}

fn build_cdn_upload_url(cdn_base_url: &str, upload_param: &str, filekey: &str) -> String {
    format!(
        "{}/upload?encrypted_query_param={}&filekey={}",
        cdn_base_url.trim_end_matches('/'),
        percent_encode(upload_param),
        percent_encode(filekey)
    )
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(nibble_to_hex(byte >> 4));
            encoded.push(nibble_to_hex(byte & 0x0F));
        }
    }
    encoded
}

fn nibble_to_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + (nibble - 10)) as char,
        _ => '0',
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0F));
    }
    out
}

fn hex_lower(bytes: Vec<u8>) -> String {
    encode_hex(&bytes).to_ascii_lowercase()
}

fn encrypt_aes_ecb_pkcs7(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    // WeChat's media upload protocol expects AES-128-ECB with PKCS#7 padding for
    // the encrypted payload. This is protocol-mandated compatibility behavior,
    // not a general-purpose encryption choice.
    let cipher = Aes128::new_from_slice(key).map_err(|e| format!("Invalid AES key: {e}"))?;
    let mut padded = plaintext.to_vec();
    let pad_len = AES_BLOCK_SIZE - (padded.len() % AES_BLOCK_SIZE);
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));

    for chunk in padded.chunks_exact_mut(AES_BLOCK_SIZE) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }

    Ok(padded)
}

fn md5_bytes(bytes: &[u8]) -> Vec<u8> {
    Md5::digest(bytes).to_vec()
}

fn random_bytes(len: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    if bytes.iter().all(|byte| *byte == 0) {
        return Err("OS RNG returned all-zero bytes unexpectedly".to_string());
    }
    Ok(bytes)
}

fn padded_size(raw_size: u64) -> u64 {
    ((raw_size / AES_BLOCK_SIZE as u64) + 1) * AES_BLOCK_SIZE as u64
}

#[cfg(test)]
mod tests {
    use super::{
        build_voice_extras_json, classify_outbound_media_kind, encode_hex, encrypt_aes_ecb_pkcs7,
        infer_file_mime_type, infer_voice_media_type, map_file_attachment, map_image_attachment,
        map_video_attachment, map_voice_attachment, OutboundMediaKind, AES_BLOCK_SIZE,
    };
    use crate::types::{
        CdnMedia, FileItem, ImageItem, MessageItem, VideoItem, VoiceItem, WechatConfig,
        WechatMessage, MESSAGE_ITEM_FILE, MESSAGE_ITEM_IMAGE, MESSAGE_ITEM_VIDEO,
        MESSAGE_ITEM_VOICE,
    };

    #[test]
    fn test_encrypt_aes_ecb_pkcs7_is_block_aligned() {
        let key = [0x11u8; 16];
        let plaintext = b"wechat image payload".to_vec();
        let ciphertext = encrypt_aes_ecb_pkcs7(&plaintext, &key).unwrap();
        assert_eq!(ciphertext.len() % AES_BLOCK_SIZE, 0);
        assert_ne!(ciphertext, plaintext);
        assert_eq!(
            encode_hex(&ciphertext).to_ascii_lowercase(),
            "a7464c94a03fb2c5aa783597a1d2f5a461f1cd5d83a7bd92721e8ac1853f881f"
        );
    }

    #[test]
    fn test_map_image_attachment_errors_when_message_id_missing() {
        let config = WechatConfig::default();
        let message = WechatMessage {
            message_id: None,
            from_user_id: Some("user-1".to_string()),
            to_user_id: Some("bot-1".to_string()),
            session_id: None,
            message_type: None,
            context_token: None,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_IMAGE),
                text_item: None,
                image_item: Some(ImageItem {
                    media: Some(CdnMedia {
                        encrypt_query_param: Some("enc".to_string()),
                        aes_key: Some("aes".to_string()),
                        encrypt_type: Some(1),
                    }),
                    aeskey: None,
                    mid_size: Some(128),
                }),
                voice_item: None,
                file_item: None,
                video_item: None,
            }],
        };

        let error = map_image_attachment(&config, &message, &message.item_list[0], 0)
            .expect_err("missing message_id should error");
        assert!(error.contains("missing message_id"));
    }

    #[test]
    fn test_map_file_attachment_uses_filename_and_size_metadata() {
        let config = WechatConfig::default();
        let message = WechatMessage {
            message_id: Some(42),
            from_user_id: Some("user-1".to_string()),
            to_user_id: Some("bot-1".to_string()),
            session_id: None,
            message_type: None,
            context_token: None,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_FILE),
                text_item: None,
                image_item: None,
                voice_item: None,
                file_item: Some(FileItem {
                    media: Some(CdnMedia {
                        encrypt_query_param: Some("enc".to_string()),
                        aes_key: Some("YWJjZGVmZ2hpamtsbW5vcA==".to_string()),
                        encrypt_type: Some(1),
                    }),
                    file_name: Some("report.PDF".to_string()),
                    len: Some("256".to_string()),
                }),
                video_item: None,
            }],
        };

        let attachment = map_file_attachment(&config, &message, &message.item_list[0], 0)
            .expect("file attachment should map")
            .expect("file attachment should be present");
        assert_eq!(attachment.id, "wechat-file-42-0");
        assert_eq!(attachment.mime_type, "application/pdf");
        assert_eq!(attachment.filename.as_deref(), Some("report.PDF"));
        assert_eq!(attachment.size_bytes, Some(256));
        assert!(attachment.extras_json.contains("wechat_aes_key"));
    }

    #[test]
    fn test_map_file_attachment_errors_when_message_id_missing() {
        let config = WechatConfig::default();
        let message = WechatMessage {
            message_id: None,
            from_user_id: Some("user-1".to_string()),
            to_user_id: Some("bot-1".to_string()),
            session_id: None,
            message_type: None,
            context_token: None,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_FILE),
                text_item: None,
                image_item: None,
                voice_item: None,
                file_item: Some(FileItem {
                    media: Some(CdnMedia {
                        encrypt_query_param: Some("enc".to_string()),
                        aes_key: Some("aes".to_string()),
                        encrypt_type: Some(1),
                    }),
                    file_name: Some("report.pdf".to_string()),
                    len: Some("256".to_string()),
                }),
                video_item: None,
            }],
        };

        let error = map_file_attachment(&config, &message, &message.item_list[0], 0)
            .expect_err("missing message_id should error");
        assert!(error.contains("missing message_id"));
    }

    #[test]
    fn test_infer_file_mime_type_defaults_to_octet_stream() {
        assert_eq!(
            infer_file_mime_type("archive.unknown"),
            "application/octet-stream"
        );
        assert_eq!(infer_file_mime_type("README"), "application/octet-stream");
    }

    #[test]
    fn test_infer_voice_media_type_defaults_to_silk() {
        assert_eq!(infer_voice_media_type(Some(6)), ("audio/silk", "silk"));
        assert_eq!(infer_voice_media_type(Some(8)), ("audio/ogg", "ogg"));
        assert_eq!(infer_voice_media_type(None), ("audio/silk", "silk"));
    }

    #[test]
    fn test_build_voice_extras_json_includes_duration() {
        let extras = build_voice_extras_json("aes-key", Some(9));
        assert!(extras.contains("wechat_aes_key"));
        assert!(extras.contains("duration_secs"));
    }

    #[test]
    fn test_map_voice_attachment_sets_audio_metadata() {
        let config = WechatConfig::default();
        let message = WechatMessage {
            message_id: Some(77),
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
                    media: Some(CdnMedia {
                        encrypt_query_param: Some("enc".to_string()),
                        aes_key: Some("YWJjZGVmZ2hpamtsbW5vcA==".to_string()),
                        encrypt_type: Some(1),
                    }),
                    encode_type: Some(8),
                    playtime: Some(4200),
                    text: Some("hello from voice".to_string()),
                }),
                file_item: None,
                video_item: None,
            }],
        };

        let attachment = map_voice_attachment(&config, &message, &message.item_list[0], 0)
            .expect("voice attachment should map")
            .expect("voice attachment should be present");
        assert_eq!(attachment.id, "wechat-voice-77-0");
        assert_eq!(attachment.mime_type, "audio/ogg");
        assert_eq!(
            attachment.filename.as_deref(),
            Some("wechat-voice-77-0.ogg")
        );
        assert_eq!(
            attachment.extracted_text.as_deref(),
            Some("hello from voice")
        );
        assert!(attachment.extras_json.contains("duration_secs"));
    }

    #[test]
    fn test_map_video_attachment_sets_video_metadata() {
        let config = WechatConfig::default();
        let message = WechatMessage {
            message_id: Some(88),
            from_user_id: Some("user-1".to_string()),
            to_user_id: Some("bot-1".to_string()),
            session_id: None,
            message_type: None,
            context_token: None,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_VIDEO),
                text_item: None,
                image_item: None,
                voice_item: None,
                file_item: None,
                video_item: Some(VideoItem {
                    media: Some(CdnMedia {
                        encrypt_query_param: Some("enc".to_string()),
                        aes_key: Some("YWJjZGVmZ2hpamtsbW5vcA==".to_string()),
                        encrypt_type: Some(1),
                    }),
                    video_size: Some(2048),
                    play_length: Some(6_000),
                }),
            }],
        };

        let attachment = map_video_attachment(&config, &message, &message.item_list[0], 0)
            .expect("video attachment should map")
            .expect("video attachment should be present");
        assert_eq!(attachment.id, "wechat-video-88-0");
        assert_eq!(attachment.mime_type, "video/mp4");
        assert_eq!(
            attachment.filename.as_deref(),
            Some("wechat-video-88-0.mp4")
        );
        assert_eq!(attachment.size_bytes, Some(2048));
        assert!(attachment.extras_json.contains("wechat_aes_key"));
    }

    #[test]
    fn test_classify_outbound_media_kind_routes_supported_media_types() {
        assert_eq!(
            classify_outbound_media_kind("image/png"),
            OutboundMediaKind::Image
        );
        assert_eq!(
            classify_outbound_media_kind("video/mp4"),
            OutboundMediaKind::Video
        );
        assert_eq!(
            classify_outbound_media_kind("application/pdf"),
            OutboundMediaKind::File
        );
    }
}
