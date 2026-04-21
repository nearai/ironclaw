use std::time::Duration;

use aes::Aes128;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};
use base64::Engine as _;
use futures::StreamExt;
use md5::{Digest, Md5};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use silk_rs::decode_silk;

use crate::channels::wasm::host::{Attachment, ChannelHostState};

const AES_BLOCK_SIZE: usize = 16;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const WECHAT_CHANNEL_NAME: &str = "wechat";
const WECHAT_SILK_SAMPLE_RATE_HZ: i32 = 24_000;
const WECHAT_OUTBOUND_ENVELOPE_MAGIC: &[u8] = b"ICWXENC1";

#[derive(Debug, Deserialize)]
struct WechatAttachmentExtras {
    wechat_aes_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PreparedWechatUpload {
    raw_size: u64,
    raw_md5: String,
    ciphertext_size: u64,
    filekey: String,
    aes_key_base64: String,
    aes_key_hex: String,
}

pub(crate) async fn hydrate_attachment_for_channel(
    host_state: &mut ChannelHostState,
    attachment: &mut Attachment,
) {
    if !should_hydrate_wechat_attachment(host_state.channel_name(), attachment) {
        return;
    }

    let Some(source_url) = attachment.source_url.as_deref() else {
        return;
    };
    let Some(encoded_aes_key) = wechat_aes_key(&attachment.extras_json) else {
        tracing::warn!(
            channel = %host_state.channel_name(),
            attachment_id = %attachment.id,
            "Skipping WeChat attachment hydration: missing AES key metadata"
        );
        return;
    };

    match download_wechat_attachment_bytes(host_state, source_url).await {
        Ok(ciphertext) => match decrypt_wechat_attachment_bytes(&ciphertext, &encoded_aes_key) {
            Ok(plaintext) => {
                attachment.size_bytes = Some(plaintext.len() as u64);
                attachment.data = plaintext;
                if attachment.mime_type.starts_with("image/") {
                    attachment.mime_type = detect_image_mime(&attachment.data).to_string();
                } else if is_wechat_silk_attachment(attachment)
                    && let Err(error) = maybe_transcode_wechat_silk_attachment(attachment)
                {
                    tracing::warn!(
                        channel = %host_state.channel_name(),
                        attachment_id = %attachment.id,
                        error = %error,
                        "Failed to transcode WeChat SILK attachment; preserving raw SILK"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    channel = %host_state.channel_name(),
                    attachment_id = %attachment.id,
                    error = %error,
                    "Failed to decrypt WeChat attachment"
                );
            }
        },
        Err(error) => {
            tracing::warn!(
                channel = %host_state.channel_name(),
                attachment_id = %attachment.id,
                error = %error,
                "Failed to download WeChat attachment"
            );
        }
    }
}

fn is_wechat_silk_attachment(attachment: &Attachment) -> bool {
    attachment.mime_type.eq_ignore_ascii_case("audio/silk")
        || attachment
            .filename
            .as_deref()
            .and_then(|filename| filename.rsplit_once('.').map(|(_, ext)| ext))
            .is_some_and(|ext| ext.eq_ignore_ascii_case("silk"))
}

fn should_hydrate_wechat_attachment(channel_name: &str, attachment: &Attachment) -> bool {
    channel_name == WECHAT_CHANNEL_NAME
        && attachment.data.is_empty()
        && attachment.source_url.is_some()
}

fn wechat_aes_key(extras_json: &str) -> Option<String> {
    if extras_json.trim().is_empty() {
        return None;
    }

    serde_json::from_str::<WechatAttachmentExtras>(extras_json)
        .ok()
        .and_then(|extras| extras.wechat_aes_key)
        .filter(|value| !value.trim().is_empty())
}

async fn download_wechat_attachment_bytes(
    host_state: &mut ChannelHostState,
    source_url: &str,
) -> Result<Vec<u8>, String> {
    host_state.check_http_allowed(source_url, "GET")?;
    host_state.record_http_request()?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let response = client
        .get(source_url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("WeChat CDN download failed: {e}"))?;

    if response.status() != reqwest::StatusCode::OK {
        return Err(format!(
            "WeChat CDN download returned {}",
            response.status()
        ));
    }
    if let Some(content_length) = response.content_length()
        && content_length > MAX_ATTACHMENT_BYTES as u64
    {
        return Err(format!(
            "WeChat attachment exceeds {MAX_ATTACHMENT_BYTES} bytes"
        ));
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Failed to read WeChat CDN response body: {e}"))?;
        let next_len = bytes.len().saturating_add(chunk.len());
        if next_len > MAX_ATTACHMENT_BYTES {
            return Err(format!(
                "WeChat attachment exceeds {MAX_ATTACHMENT_BYTES} bytes"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }

    if bytes.is_empty() {
        return Err("WeChat CDN download returned an empty body".to_string());
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "WeChat attachment exceeds {MAX_ATTACHMENT_BYTES} bytes"
        ));
    }

    Ok(bytes)
}

fn decrypt_wechat_attachment_bytes(
    ciphertext: &[u8],
    encoded_aes_key: &str,
) -> Result<Vec<u8>, String> {
    let key = parse_aes_key(encoded_aes_key)?;
    decrypt_aes_ecb_pkcs7(ciphertext, &key)
}

fn parse_aes_key(encoded: &str) -> Result<Vec<u8>, String> {
    let decoded = if encoded.len() == 32 && encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        decode_hex(encoded)?
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("Failed to decode WeChat AES key: {e}"))?
    };

    if decoded.len() == AES_BLOCK_SIZE {
        return Ok(decoded);
    }

    if decoded.len() == 32 && decoded.iter().all(|byte| byte.is_ascii_hexdigit()) {
        return decode_hex(
            std::str::from_utf8(&decoded)
                .map_err(|e| format!("WeChat AES key hex payload is not valid UTF-8: {e}"))?,
        );
    }

    Err(format!(
        "WeChat AES key must decode to 16 bytes or a 32-char hex string, got {} bytes",
        decoded.len()
    ))
}

pub(crate) fn prepare_outbound_attachment_for_channel(
    channel_name: &str,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    if channel_name != WECHAT_CHANNEL_NAME || data.is_empty() {
        return Ok(data.to_vec());
    }

    let prepared = prepare_wechat_outbound_attachment(data)?;
    pack_prepared_wechat_upload(&prepared)
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if !input.len().is_multiple_of(2) {
        return Err("hex input length must be even".to_string());
    }
    let mut bytes = Vec::with_capacity(input.len() / 2);
    let chars: Vec<u8> = input.as_bytes().to_vec();
    for idx in (0..chars.len()).step_by(2) {
        let high = from_hex_digit(chars[idx])?;
        let low = from_hex_digit(chars[idx + 1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn from_hex_digit(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(format!("invalid hex digit '{}'", value as char)),
    }
}

fn decrypt_aes_ecb_pkcs7(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    if !ciphertext.len().is_multiple_of(AES_BLOCK_SIZE) {
        return Err("ciphertext length is not a multiple of 16 bytes".to_string());
    }

    let cipher = Aes128::new_from_slice(key).map_err(|e| format!("Invalid AES key: {e}"))?;
    let mut plaintext = ciphertext.to_vec();
    for chunk in plaintext.chunks_exact_mut(AES_BLOCK_SIZE) {
        cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
    }

    let pad_len = *plaintext
        .last()
        .ok_or_else(|| "ciphertext decrypted to an empty buffer".to_string())?
        as usize;
    if pad_len == 0 || pad_len > AES_BLOCK_SIZE || pad_len > plaintext.len() {
        return Err("invalid PKCS7 padding".to_string());
    }
    if !plaintext[plaintext.len() - pad_len..]
        .iter()
        .all(|byte| *byte as usize == pad_len)
    {
        return Err("invalid PKCS7 padding bytes".to_string());
    }
    plaintext.truncate(plaintext.len() - pad_len);
    Ok(plaintext)
}

fn prepare_wechat_outbound_attachment(
    data: &[u8],
) -> Result<(PreparedWechatUpload, Vec<u8>), String> {
    let raw_size = data.len() as u64;
    let raw_md5 = encode_hex(&Md5::digest(data)).to_ascii_lowercase();
    let ciphertext_size = padded_size(raw_size);
    let filekey = encode_hex(&random_bytes(16)?).to_ascii_lowercase();
    let aes_key = random_bytes(16)?;
    let aes_key_hex = encode_hex(&aes_key).to_ascii_lowercase();
    let aes_key_base64 = base64::engine::general_purpose::STANDARD.encode(&aes_key);
    let ciphertext = encrypt_aes_ecb_pkcs7(data, &aes_key)?;
    if ciphertext.len() as u64 != ciphertext_size {
        return Err(format!(
            "WeChat outbound ciphertext size mismatch: expected={} actual={}",
            ciphertext_size,
            ciphertext.len()
        ));
    }

    Ok((
        PreparedWechatUpload {
            raw_size,
            raw_md5,
            ciphertext_size,
            filekey,
            aes_key_base64,
            aes_key_hex,
        },
        ciphertext,
    ))
}

fn pack_prepared_wechat_upload(
    prepared: &(PreparedWechatUpload, Vec<u8>),
) -> Result<Vec<u8>, String> {
    let metadata_json = serde_json::to_vec(&prepared.0)
        .map_err(|e| format!("Failed to serialize WeChat outbound attachment metadata: {e}"))?;
    let metadata_len = u32::try_from(metadata_json.len())
        .map_err(|_| "WeChat outbound attachment metadata exceeds 4 GiB".to_string())?;

    let mut packed = Vec::with_capacity(
        WECHAT_OUTBOUND_ENVELOPE_MAGIC.len() + 4 + metadata_json.len() + prepared.1.len(),
    );
    packed.extend_from_slice(WECHAT_OUTBOUND_ENVELOPE_MAGIC);
    packed.extend_from_slice(&metadata_len.to_le_bytes());
    packed.extend_from_slice(&metadata_json);
    packed.extend_from_slice(&prepared.1);
    Ok(packed)
}

#[cfg(test)]
fn unpack_prepared_wechat_upload(
    data: &[u8],
) -> Result<Option<(PreparedWechatUpload, Vec<u8>)>, String> {
    if !data.starts_with(WECHAT_OUTBOUND_ENVELOPE_MAGIC) {
        return Ok(None);
    }

    let header_len = WECHAT_OUTBOUND_ENVELOPE_MAGIC.len();
    if data.len() < header_len + 4 {
        return Err("WeChat outbound attachment envelope is truncated".to_string());
    }

    let metadata_len = u32::from_le_bytes(
        data[header_len..header_len + 4]
            .try_into()
            .map_err(|_| "Failed to decode WeChat outbound metadata length".to_string())?,
    ) as usize;
    let metadata_start = header_len + 4;
    let metadata_end = metadata_start.saturating_add(metadata_len);
    if metadata_end > data.len() {
        return Err("WeChat outbound attachment envelope metadata is truncated".to_string());
    }

    let metadata =
        serde_json::from_slice::<PreparedWechatUpload>(&data[metadata_start..metadata_end])
            .map_err(|e| format!("Failed to parse WeChat outbound attachment metadata: {e}"))?;
    let ciphertext = data[metadata_end..].to_vec();
    if metadata.ciphertext_size != ciphertext.len() as u64 {
        return Err(format!(
            "WeChat outbound attachment ciphertext size mismatch: metadata={} actual={}",
            metadata.ciphertext_size,
            ciphertext.len()
        ));
    }

    Ok(Some((metadata, ciphertext)))
}

fn detect_image_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

fn maybe_transcode_wechat_silk_attachment(attachment: &mut Attachment) -> Result<(), String> {
    if attachment.data.is_empty() {
        return Err("SILK attachment has no data".to_string());
    }

    let pcm = decode_silk(&attachment.data, WECHAT_SILK_SAMPLE_RATE_HZ)
        .map_err(|error| format!("SILK decode failed: {error}"))?;
    if pcm.is_empty() {
        return Err("SILK decoder returned empty PCM".to_string());
    }

    let wav = pcm_s16le_to_wav(&pcm, WECHAT_SILK_SAMPLE_RATE_HZ as u32)?;
    attachment.data = wav;
    attachment.size_bytes = Some(attachment.data.len() as u64);
    attachment.mime_type = "audio/wav".to_string();
    if let Some(filename) = attachment.filename.as_mut() {
        replace_attachment_extension(filename, "wav");
    }
    Ok(())
}

fn pcm_s16le_to_wav(pcm: &[u8], sample_rate_hz: u32) -> Result<Vec<u8>, String> {
    if !pcm.len().is_multiple_of(2) {
        return Err("PCM buffer length must be even for 16-bit mono audio".to_string());
    }

    let data_len = u32::try_from(pcm.len())
        .map_err(|_| "PCM buffer exceeds WAV container size limits".to_string())?;
    let total_len = 44u32
        .checked_add(data_len)
        .ok_or_else(|| "WAV container size overflowed".to_string())?;
    let byte_rate = sample_rate_hz
        .checked_mul(2)
        .ok_or_else(|| "WAV byte rate overflowed".to_string())?;

    let mut wav = Vec::with_capacity(total_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(total_len - 8).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate_hz.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
}

fn replace_attachment_extension(filename: &mut String, replacement: &str) {
    if let Some((stem, _)) = filename.rsplit_once('.') {
        *filename = format!("{stem}.{replacement}");
    } else {
        filename.push('.');
        filename.push_str(replacement);
    }
}

fn encrypt_aes_ecb_pkcs7(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    // WeChat's CDN upload protocol requires AES-128-ECB with PKCS#7 padding for
    // outbound media payloads. This is compatibility logic for that protocol,
    // not a general recommendation for new encryption schemes.
    let cipher = Aes128::new_from_slice(key).map_err(|e| format!("Invalid AES key: {e}"))?;
    let mut padded = plaintext.to_vec();
    let pad_len = AES_BLOCK_SIZE - (padded.len() % AES_BLOCK_SIZE);
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));

    for chunk in padded.chunks_exact_mut(AES_BLOCK_SIZE) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }

    Ok(padded)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0F));
    }
    out
}

fn nibble_to_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'A' + (nibble - 10)) as char,
        _ => '0',
    }
}

fn padded_size(raw_size: u64) -> u64 {
    ((raw_size / AES_BLOCK_SIZE as u64) + 1) * AES_BLOCK_SIZE as u64
}

fn random_bytes(len: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    if bytes.iter().all(|byte| *byte == 0) {
        return Err("OS RNG returned all-zero bytes unexpectedly".to_string());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        AES_BLOCK_SIZE, Attachment, decrypt_wechat_attachment_bytes, detect_image_mime,
        encrypt_aes_ecb_pkcs7, hydrate_attachment_for_channel,
        maybe_transcode_wechat_silk_attachment, pcm_s16le_to_wav,
        prepare_outbound_attachment_for_channel, should_hydrate_wechat_attachment,
        unpack_prepared_wechat_upload,
    };
    use crate::channels::wasm::{ChannelCapabilities, ChannelHostState};
    use crate::tools::wasm::{Capabilities, EndpointPattern, HttpCapability};
    use base64::Engine as _;

    fn make_attachment() -> Attachment {
        Attachment {
            id: "wechat-image-1".to_string(),
            mime_type: "image/jpeg".to_string(),
            filename: Some("wechat-image.jpg".to_string()),
            size_bytes: None,
            source_url: Some(
                "https://novac2c.cdn.weixin.qq.com/c2c/download?encrypted_query_param=test"
                    .to_string(),
            ),
            storage_key: None,
            local_path: None,
            extracted_text: None,
            extras_json: String::new(),
            data: Vec::new(),
            duration_secs: None,
        }
    }

    fn encode_test_extras_json(aes_key: &str) -> String {
        serde_json::json!({ "wechat_aes_key": aes_key }).to_string()
    }

    #[test]
    fn decrypt_wechat_image_bytes_round_trips() {
        let key = [7u8; 16];
        let plaintext = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x11];
        let ciphertext = encrypt_aes_ecb_pkcs7(&plaintext, &key).unwrap();
        let encoded_key = base64::engine::general_purpose::STANDARD.encode(key);
        let decrypted = decrypt_wechat_attachment_bytes(&ciphertext, &encoded_key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wechat_outbound_attachment_preparation_round_trips() {
        let plaintext = b"wechat outbound image".to_vec();
        let packed =
            prepare_outbound_attachment_for_channel("wechat", &plaintext).expect("prepare");
        assert_ne!(packed, plaintext);

        let (metadata, ciphertext) = unpack_prepared_wechat_upload(&packed)
            .expect("parse envelope")
            .expect("wechat envelope");
        assert_eq!(metadata.raw_size, plaintext.len() as u64);
        assert_eq!(metadata.ciphertext_size, ciphertext.len() as u64);
        assert_eq!(metadata.ciphertext_size % AES_BLOCK_SIZE as u64, 0);

        let decrypted = decrypt_wechat_attachment_bytes(&ciphertext, &metadata.aes_key_base64)
            .expect("decrypt host-prepared ciphertext");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn non_wechat_outbound_attachment_preparation_is_passthrough() {
        let plaintext = b"plain attachment".to_vec();
        let prepared =
            prepare_outbound_attachment_for_channel("telegram", &plaintext).expect("prepare");
        assert_eq!(prepared, plaintext);
    }

    #[test]
    fn detect_image_mime_prefers_magic_bytes() {
        assert_eq!(detect_image_mime(&[0xFF, 0xD8, 0xFF, 0x00]), "image/jpeg");
        assert_eq!(
            detect_image_mime(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            "image/png"
        );
    }

    #[test]
    fn wechat_attachment_hydration_applies_to_wechat_encrypted_media() {
        let mut attachment = make_attachment();
        attachment.extras_json = encode_test_extras_json("ZmFrZS1rZXk=");
        assert!(should_hydrate_wechat_attachment("wechat", &attachment));
        assert!(!should_hydrate_wechat_attachment("telegram", &attachment));

        attachment.mime_type = "application/pdf".to_string();
        assert!(should_hydrate_wechat_attachment("wechat", &attachment));
    }

    #[tokio::test]
    async fn hydration_skips_when_metadata_is_missing() {
        let mut attachment = make_attachment();
        let caps = ChannelCapabilities::for_channel("wechat");
        let mut host_state = ChannelHostState::new("wechat", caps);
        hydrate_attachment_for_channel(&mut host_state, &mut attachment).await;
        assert!(attachment.data.is_empty());
        assert_eq!(attachment.size_bytes, None);
    }

    #[test]
    fn wechat_attachment_downloads_consume_host_http_budget() {
        let caps = ChannelCapabilities::for_channel("wechat").with_tool_capabilities(
            Capabilities::default().with_http(HttpCapability::new(vec![
                EndpointPattern::host("novac2c.cdn.weixin.qq.com")
                    .with_path_prefix("/c2c/download")
                    .with_methods(vec!["GET".to_string()]),
            ])),
        );
        let mut host_state = ChannelHostState::new("wechat", caps);
        let url = "https://novac2c.cdn.weixin.qq.com/c2c/download?encrypted_query_param=test";

        for _ in 0..50 {
            host_state
                .check_http_allowed(url, "GET")
                .expect("allowlisted request");
            host_state
                .record_http_request()
                .expect("request budget available");
        }

        let error = host_state
            .record_http_request()
            .expect_err("51st request should exceed per-execution budget");
        assert!(error.contains("Too many HTTP requests in single execution"));
    }

    #[test]
    fn pcm_s16le_to_wav_wraps_pcm_with_expected_header() {
        let wav = pcm_s16le_to_wav(&[0x00, 0x00, 0x01, 0x00], 24_000).expect("wav wrapping");
        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[40..44], &(4u32).to_le_bytes());
        assert_eq!(&wav[44..], &[0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn silk_transcode_failure_preserves_raw_silk_path_for_callers() {
        let mut attachment = Attachment {
            id: "wechat-voice-1".to_string(),
            mime_type: "audio/silk".to_string(),
            filename: Some("wechat-voice-1.silk".to_string()),
            size_bytes: Some(3),
            source_url: None,
            storage_key: None,
            local_path: None,
            extracted_text: None,
            extras_json: encode_test_extras_json("ZmFrZS1rZXk="),
            data: vec![1, 2, 3],
            duration_secs: Some(1),
        };

        let original = attachment.data.clone();
        let error =
            maybe_transcode_wechat_silk_attachment(&mut attachment).expect_err("invalid SILK");
        assert!(error.contains("SILK decode failed"));
        assert_eq!(attachment.mime_type, "audio/silk");
        assert_eq!(attachment.filename.as_deref(), Some("wechat-voice-1.silk"));
        assert_eq!(attachment.data, original);
    }
}
