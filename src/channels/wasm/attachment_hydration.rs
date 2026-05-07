use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use aes::Aes128;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};
use base64::Engine as _;
use futures::StreamExt;
use md5::{Digest, Md5};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::channels::wasm::host::{Attachment, ChannelHostState};

const AES_BLOCK_SIZE: usize = 16;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const WECHAT_CHANNEL_NAME: &str = "wechat";
const WECHAT_SILK_SAMPLE_RATE_HZ: u32 = 24_000;
const WECHAT_OUTBOUND_ENVELOPE_MAGIC: &[u8] = b"ICWXENC1";

/// Cap on output WAV bytes accepted from the SILK decoder subprocess. SILK→PCM
/// expansion is ~25× at 24 kHz mono; 60 s of voice is ~3 MiB. 50 MiB matches
/// the cap inside the decoder binary and prevents a runaway child from
/// pushing unbounded data into the host.
const MAX_DECODED_WAV_BYTES: usize = 50 * 1024 * 1024;
const SILK_DECODER_TIMEOUT: Duration = Duration::from_secs(15);
const SILK_DECODER_BIN_NAME: &str = "ironclaw-silk-decoder";
const SILK_DECODER_ENV_VAR: &str = "IRONCLAW_SILK_DECODER";

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
                    && let Err(error) = maybe_transcode_wechat_silk_attachment(attachment).await
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

async fn maybe_transcode_wechat_silk_attachment(attachment: &mut Attachment) -> Result<(), String> {
    if attachment.data.is_empty() {
        return Err("SILK attachment has no data".to_string());
    }
    let decoder_path = resolve_silk_decoder_command().ok_or_else(|| {
        format!(
            "{SILK_DECODER_BIN_NAME} not found (set {SILK_DECODER_ENV_VAR}, install on PATH, or place beside the ironclaw binary)"
        )
    })?;

    let wav = run_silk_decoder(&decoder_path, &attachment.data, WECHAT_SILK_SAMPLE_RATE_HZ).await?;
    if wav.is_empty() {
        return Err("SILK decoder returned empty WAV".to_string());
    }
    if !wav.starts_with(b"RIFF") {
        return Err("SILK decoder did not produce a RIFF/WAVE stream".to_string());
    }

    attachment.size_bytes = Some(wav.len() as u64);
    attachment.data = wav;
    attachment.mime_type = "audio/wav".to_string();
    if let Some(filename) = attachment.filename.as_mut() {
        replace_attachment_extension(filename, "wav");
    }
    Ok(())
}

/// Locate the optional SILK decoder helper binary. Lookup order:
///
/// 1. `IRONCLAW_SILK_DECODER` env var, used verbatim as a path.
/// 2. Sibling of the running executable (`<exe-dir>/ironclaw-silk-decoder[.exe]`).
/// 3. Bare `ironclaw-silk-decoder` for `$PATH` resolution by `Command`.
///
/// Returns `None` only when no candidate looks viable; callers fall back to
/// preserving raw SILK and logging that the decoder is not configured.
fn resolve_silk_decoder_command() -> Option<OsString> {
    if let Some(path) = std::env::var_os(SILK_DECODER_ENV_VAR)
        && !path.is_empty()
    {
        return Some(path);
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        let mut candidate: PathBuf = parent.to_path_buf();
        if cfg!(windows) {
            candidate.push(format!("{SILK_DECODER_BIN_NAME}.exe"));
        } else {
            candidate.push(SILK_DECODER_BIN_NAME);
        }
        if candidate.is_file() {
            return Some(candidate.into_os_string());
        }
    }

    Some(OsString::from(SILK_DECODER_BIN_NAME))
}

async fn run_silk_decoder(
    program: &OsString,
    silk_bytes: &[u8],
    sample_rate_hz: u32,
) -> Result<Vec<u8>, String> {
    let mut command = Command::new(program);
    command
        .arg("--sample-rate")
        .arg(sample_rate_hz.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|e| {
        format!("failed to spawn {SILK_DECODER_BIN_NAME}: {e} (is the helper binary installed?)")
    })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("failed to capture stdin for {SILK_DECODER_BIN_NAME}"))?;
    let input = silk_bytes.to_vec();
    let writer = tokio::spawn(async move {
        stdin
            .write_all(&input)
            .await
            .map_err(|e| format!("failed to send SILK bytes to decoder: {e}"))?;
        stdin
            .shutdown()
            .await
            .map_err(|e| format!("failed to close decoder stdin: {e}"))
    });

    let output_future = child.wait_with_output();
    let output = match tokio::time::timeout(SILK_DECODER_TIMEOUT, output_future).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("{SILK_DECODER_BIN_NAME} failed: {error}")),
        Err(_) => {
            return Err(format!(
                "{SILK_DECODER_BIN_NAME} timed out after {}s",
                SILK_DECODER_TIMEOUT.as_secs()
            ));
        }
    };
    writer
        .await
        .map_err(|e| format!("decoder stdin task panicked: {e}"))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(format!(
            "{SILK_DECODER_BIN_NAME} exited with {} (stderr: {})",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            if stderr.is_empty() { "<empty>" } else { stderr }
        ));
    }

    if output.stdout.len() > MAX_DECODED_WAV_BYTES {
        return Err(format!(
            "{SILK_DECODER_BIN_NAME} produced {} bytes, exceeds {MAX_DECODED_WAV_BYTES} cap",
            output.stdout.len()
        ));
    }
    Ok(output.stdout)
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
        AES_BLOCK_SIZE, Attachment, MAX_DECODED_WAV_BYTES, SILK_DECODER_BIN_NAME,
        SILK_DECODER_ENV_VAR, decrypt_wechat_attachment_bytes, detect_image_mime,
        encrypt_aes_ecb_pkcs7, hydrate_attachment_for_channel,
        maybe_transcode_wechat_silk_attachment, prepare_outbound_attachment_for_channel,
        resolve_silk_decoder_command, should_hydrate_wechat_attachment,
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
    fn resolve_silk_decoder_command_prefers_env_var() {
        // Serialize against any other env-mutating test in the workspace.
        // Without this, the cargo-test default thread pool can interleave
        // this test with the EnvGuard-protected stub-binary tests below,
        // producing flaky results.
        let _env_lock = crate::config::helpers::lock_env();
        let previous = std::env::var_os(SILK_DECODER_ENV_VAR);
        // SAFETY: env-mutation under the global env-test lock; cargo-test
        // workers contending on this var are blocked until the guard drops.
        unsafe {
            std::env::set_var(SILK_DECODER_ENV_VAR, "/opt/custom/decoder");
        }
        let resolved = resolve_silk_decoder_command();
        // SAFETY: restore prior state before releasing the lock.
        unsafe {
            match previous {
                Some(value) => std::env::set_var(SILK_DECODER_ENV_VAR, value),
                None => std::env::remove_var(SILK_DECODER_ENV_VAR),
            }
        }
        assert_eq!(
            resolved,
            Some(std::ffi::OsString::from("/opt/custom/decoder"))
        );
    }

    #[test]
    fn resolve_silk_decoder_command_falls_back_to_path_lookup() {
        let _env_lock = crate::config::helpers::lock_env();
        let previous = std::env::var_os(SILK_DECODER_ENV_VAR);
        // SAFETY: env-mutation under the global env-test lock.
        unsafe {
            std::env::remove_var(SILK_DECODER_ENV_VAR);
        }
        let resolved = resolve_silk_decoder_command();
        // SAFETY: restore prior state before releasing the lock.
        unsafe {
            if let Some(value) = previous {
                std::env::set_var(SILK_DECODER_ENV_VAR, value);
            }
        }
        // Without env var and (almost certainly) no sibling binary in the
        // cargo-test runner directory, the resolver should still hand back
        // the bare program name for $PATH resolution. This guarantees
        // graceful "binary not installed" handling rather than `None`.
        let value = resolved.expect("resolver should always offer a candidate");
        assert!(
            value
                .to_str()
                .is_some_and(|s| s.ends_with(SILK_DECODER_BIN_NAME))
        );
    }

    #[cfg(unix)]
    fn write_unix_stub(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;

        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("create stub");
        write!(file, "#!/usr/bin/env bash\nset -eu\n{body}\n").expect("write stub");
        let mut perms = std::fs::metadata(&path).expect("stat stub").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod stub");
        path
    }

    #[cfg(unix)]
    struct EnvGuard {
        key: &'static str,
        prior: Option<std::ffi::OsString>,
        // Serializes env-var mutation across cargo-test's parallel thread
        // pool. Held until Drop so the test that set the var also owns the
        // process-wide env state for its full duration. Released last so
        // the restoration writes happen before another test takes the lock.
        _env_lock: std::sync::MutexGuard<'static, ()>,
    }

    #[cfg(unix)]
    impl EnvGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let env_lock = crate::config::helpers::lock_env();
            let prior = std::env::var_os(key);
            // SAFETY: env-mutation under the global env-test lock — held in
            // the returned guard for the full lifetime of the test.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                prior,
                _env_lock: env_lock,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see EnvGuard::set; lock is still held via _env_lock.
            unsafe {
                match self.prior.take() {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    fn fake_silk_attachment() -> Attachment {
        Attachment {
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
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silk_transcoder_consumes_decoder_output_and_updates_attachment() {
        let temp = tempfile::tempdir().expect("tempdir");
        // Stub: emit a minimal valid RIFF/WAVE header (44 bytes is the well-formed
        // empty WAV). Discards stdin to keep the pipe alive for the writer task.
        let body = "cat >/dev/null\n\
            printf 'RIFF\\x24\\x00\\x00\\x00WAVEfmt \\x10\\x00\\x00\\x00\\x01\\x00\\x01\\x00\\xc0]\\x00\\x00\\x80\\xbb\\x00\\x00\\x02\\x00\\x10\\x00data\\x00\\x00\\x00\\x00'";
        let stub = write_unix_stub(temp.path(), "stub-silk-decoder", body);
        let _guard = EnvGuard::set(SILK_DECODER_ENV_VAR, &stub);

        let mut attachment = fake_silk_attachment();
        maybe_transcode_wechat_silk_attachment(&mut attachment)
            .await
            .expect("stub decoder should succeed");

        assert_eq!(attachment.mime_type, "audio/wav");
        assert_eq!(attachment.filename.as_deref(), Some("wechat-voice-1.wav"));
        assert!(attachment.data.starts_with(b"RIFF"));
        assert_eq!(attachment.size_bytes, Some(attachment.data.len() as u64));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silk_transcoder_propagates_decoder_failure_for_caller_fallback() {
        let temp = tempfile::tempdir().expect("tempdir");
        let body = "cat >/dev/null\n\
            echo 'fake decoder failure' >&2\n\
            exit 3";
        let stub = write_unix_stub(temp.path(), "stub-silk-decoder", body);
        let _guard = EnvGuard::set(SILK_DECODER_ENV_VAR, &stub);

        let mut attachment = fake_silk_attachment();
        let original = attachment.data.clone();

        let error = maybe_transcode_wechat_silk_attachment(&mut attachment)
            .await
            .expect_err("stub decoder failure should bubble");
        assert!(
            error.contains(SILK_DECODER_BIN_NAME),
            "error mentions decoder name: {error}"
        );
        assert!(
            error.contains("fake decoder failure"),
            "error includes captured stderr: {error}"
        );

        // Caller-level invariant: the attachment must still be raw SILK so
        // hydrate_attachment_for_channel's outer warn-and-preserve branch is
        // valid. Regression coverage for the fallback path.
        assert_eq!(attachment.mime_type, "audio/silk");
        assert_eq!(attachment.filename.as_deref(), Some("wechat-voice-1.silk"));
        assert_eq!(attachment.data, original);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silk_transcoder_rejects_non_riff_output() {
        let temp = tempfile::tempdir().expect("tempdir");
        let body = "cat >/dev/null\n\
            printf 'NOT_A_WAV'";
        let stub = write_unix_stub(temp.path(), "stub-silk-decoder", body);
        let _guard = EnvGuard::set(SILK_DECODER_ENV_VAR, &stub);

        let mut attachment = fake_silk_attachment();
        let error = maybe_transcode_wechat_silk_attachment(&mut attachment)
            .await
            .expect_err("non-RIFF output should be rejected");
        assert!(error.contains("RIFF"), "error mentions RIFF: {error}");
        assert_eq!(attachment.mime_type, "audio/silk");
    }

    #[tokio::test]
    async fn silk_transcoder_errors_when_attachment_data_empty() {
        let mut attachment = fake_silk_attachment();
        attachment.data = Vec::new();
        let error = maybe_transcode_wechat_silk_attachment(&mut attachment)
            .await
            .expect_err("empty input should not invoke the decoder");
        assert!(error.contains("no data"), "error: {error}");
    }

    // Compile-time check that the decoded-WAV cap is at least as large as
    // the inbound-attachment cap. SILK→PCM expansion is ~25× at 24 kHz
    // mono; 60 s of voice is ~3 MiB. A 50 MiB output cap leaves generous
    // headroom over the 20 MiB input cap. const_assert form keeps the
    // invariant near the constants without spending a runtime test slot.
    const _: () = assert!(MAX_DECODED_WAV_BYTES >= super::MAX_ATTACHMENT_BYTES);
}
