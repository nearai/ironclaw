//! DingTalk media download: handles inbound voice, image, video, and file messages.
//!
//! DingTalk delivers multimedia messages with a `downloadCode` rather than a URL.
//! This module exchanges the code for a signed download URL, fetches the file,
//! and saves it under a UUID-based filename to prevent path-traversal attacks.

use std::path::{Path, PathBuf};

use reqwest::Client;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::error::ChannelError;

/// Maximum allowed file size for downloaded media (50 MiB).
const MAX_MEDIA_BYTES: u64 = 50 * 1024 * 1024;

/// Exchange a DingTalk `downloadCode` for a signed URL, then download the file.
///
/// The file is saved under `$TMPDIR/ironclaw-dingtalk-media/<uuid>.<ext>`.
/// Returns `(file_path, mime_type)`.
///
/// # Security
/// The caller-supplied `filename` hint is used only for extension detection;
/// it is never used as a path component.
pub async fn download_media(
    client: &Client,
    token: &str,
    download_code: &str,
    filename: Option<&str>,
) -> Result<(PathBuf, String), ChannelError> {
    // ── Step 1: exchange downloadCode for a signed URL ────────────────────────
    let exchange_url = "https://api.dingtalk.com/v1.0/robot/messageFiles/download";
    let body = serde_json::json!({
        "downloadCode": download_code,
        "robotCode": "",
    });

    let exchange_resp = client
        .post(exchange_url)
        .header("x-acs-dingtalk-access-token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ChannelError::Http(format!("media download exchange: {e}")))?;

    if !exchange_resp.status().is_success() {
        let status = exchange_resp.status();
        let body_text = exchange_resp.text().await.unwrap_or_default();
        return Err(ChannelError::Http(format!(
            "media download exchange returned {status}: {body_text}"
        )));
    }

    let exchange_json: serde_json::Value = exchange_resp
        .json()
        .await
        .map_err(|e| ChannelError::Http(format!("parse media exchange response: {e}")))?;

    let signed_url = exchange_json
        .get("downloadUrl")
        .or_else(|| exchange_json.get("url"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ChannelError::Http(format!(
                "no downloadUrl in media exchange response: {exchange_json}"
            ))
        })?
        .to_string();

    tracing::debug!(
        download_code,
        "DingTalk: got signed media URL, downloading..."
    );

    // ── Step 2: download the file from the signed URL ─────────────────────────
    let file_resp = client
        .get(&signed_url)
        .send()
        .await
        .map_err(|e| ChannelError::Http(format!("media file fetch: {e}")))?;

    if !file_resp.status().is_success() {
        let status = file_resp.status();
        return Err(ChannelError::Http(format!(
            "media file fetch returned {status}"
        )));
    }

    // Enforce size limit from Content-Length header before streaming.
    if let Some(content_length) = file_resp.content_length() {
        if content_length > MAX_MEDIA_BYTES {
            return Err(ChannelError::InvalidMessage(format!(
                "media file too large: {content_length} bytes (limit: {MAX_MEDIA_BYTES})"
            )));
        }
    }

    // Determine MIME type from response headers.
    let mime_type = file_resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| {
            // Strip parameters (e.g. "image/jpeg; charset=utf-8" → "image/jpeg")
            ct.split(';')
                .next()
                .unwrap_or(ct)
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());

    // Detect extension from filename hint first, then content-type.
    let ext = detect_extension(filename, Some(&mime_type));

    // ── Step 3: prepare temp directory ───────────────────────────────────────
    let temp_dir = std::env::temp_dir().join("ironclaw-dingtalk-media");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| ChannelError::Http(format!("create temp dir: {e}")))?;

    // UUID-based filename — never uses the user-supplied filename.
    let file_path = temp_dir.join(format!("{}.{}", Uuid::new_v4(), ext));

    // ── Step 4: stream body to disk with size enforcement ─────────────────────
    let mut file = tokio::fs::File::create(&file_path)
        .await
        .map_err(|e| ChannelError::Http(format!("create temp file: {e}")))?;

    let mut body_stream = file_resp;
    let mut bytes_written: u64 = 0;

    // Stream chunks using bytes() — reqwest's Bytes stream
    while let Some(chunk) = body_stream
        .chunk()
        .await
        .map_err(|e| ChannelError::Http(format!("read media chunk: {e}")))?
    {
        bytes_written += chunk.len() as u64;
        if bytes_written > MAX_MEDIA_BYTES {
            // Remove the partial file to avoid leaving junk on disk.
            drop(file);
            let _ = tokio::fs::remove_file(&file_path).await;
            return Err(ChannelError::InvalidMessage(format!(
                "media file exceeded {MAX_MEDIA_BYTES} bytes during download"
            )));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| ChannelError::Http(format!("write media chunk: {e}")))?;
    }

    file.flush()
        .await
        .map_err(|e| ChannelError::Http(format!("flush media file: {e}")))?;

    tracing::debug!(
        path = %file_path.display(),
        bytes = bytes_written,
        mime = %mime_type,
        "DingTalk: media file downloaded"
    );

    Ok((file_path, mime_type))
}

/// Detect an appropriate file extension from a filename hint and/or MIME type.
///
/// Priority: filename extension > MIME type mapping > "bin" default.
pub fn detect_extension(filename: Option<&str>, content_type: Option<&str>) -> String {
    // Try to get extension from the filename hint.
    // Only extract if there is actually a '.' in the name (not just the base name itself).
    if let Some(name) = filename {
        if name.contains('.') {
            if let Some(ext) = name.rsplit('.').next() {
                let ext = ext.trim().to_ascii_lowercase();
                if !ext.is_empty() && ext.len() <= 10 && ext.chars().all(|c| c.is_alphanumeric()) {
                    return ext;
                }
            }
        }
    }

    // Fall back to MIME type mapping.
    if let Some(ct) = content_type {
        let ct = ct
            .split(';')
            .next()
            .unwrap_or(ct)
            .trim()
            .to_ascii_lowercase();
        let ext = match ct.as_str() {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/png" => "png",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/bmp" => "bmp",
            "image/tiff" => "tiff",
            "image/svg+xml" => "svg",
            "audio/amr" => "amr",
            "audio/mpeg" | "audio/mp3" => "mp3",
            "audio/ogg" => "ogg",
            "audio/wav" | "audio/x-wav" => "wav",
            "audio/aac" => "aac",
            "audio/mp4" => "m4a",
            "video/mp4" => "mp4",
            "video/mpeg" => "mpeg",
            "video/webm" => "webm",
            "video/quicktime" => "mov",
            "video/x-msvideo" => "avi",
            "application/pdf" => "pdf",
            "application/zip" => "zip",
            "application/x-tar" => "tar",
            "application/gzip" => "gz",
            "application/msword" => "doc",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
            "application/vnd.ms-excel" => "xls",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
            "application/vnd.ms-powerpoint" => "ppt",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
            "text/plain" => "txt",
            "text/html" => "html",
            "text/csv" => "csv",
            _ => "bin",
        };
        return ext.to_string();
    }

    "bin".to_string()
}

/// Classify a file path into a DingTalk media type string.
///
/// Returns one of `"image"`, `"voice"`, `"video"`, or `"file"`.
pub fn detect_media_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    match ext.as_deref() {
        Some("jpg") | Some("jpeg") | Some("png") | Some("gif") | Some("bmp") | Some("webp") => {
            "image"
        }
        Some("mp3") | Some("amr") | Some("wav") | Some("ogg") | Some("m4a") => "voice",
        Some("mp4") | Some("avi") | Some("mov") | Some("mkv") | Some("webm") => "video",
        _ => "file",
    }
}

/// Upload a local file to DingTalk's media endpoint.
///
/// Uses the legacy `oapi.dingtalk.com/media/upload` endpoint (the v1.0 API does
/// not expose an equivalent upload path as of the time of writing).
///
/// Returns the `media_id` string from the JSON response.
pub async fn upload_media(
    client: &Client,
    token: &str,
    file_path: &Path,
    media_type: &str,
) -> Result<String, ChannelError> {
    let url =
        format!("https://oapi.dingtalk.com/media/upload?access_token={token}&type={media_type}");

    // Read file bytes
    let file_bytes = tokio::fs::read(file_path)
        .await
        .map_err(|e| ChannelError::Http(format!("read upload file: {e}")))?;

    // Derive a filename for the multipart part
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("media")
        .to_string();

    let part = reqwest::multipart::Part::bytes(file_bytes).file_name(filename);
    let form = reqwest::multipart::Form::new().part("media", part);

    let resp = client
        .post(&url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| ChannelError::Http(format!("media upload request: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ChannelError::Http(format!(
            "media upload returned {status}: {body_text}"
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ChannelError::Http(format!("parse media upload response: {e}")))?;

    let media_id = json
        .get("media_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ChannelError::Http(format!("no media_id in upload response: {json}")))?
        .to_string();

    tracing::debug!(
        path = %file_path.display(),
        media_type,
        media_id = %media_id,
        "DingTalk: media uploaded"
    );

    Ok(media_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_extension_from_filename() {
        assert_eq!(detect_extension(Some("photo.jpg"), None), "jpg");
        assert_eq!(detect_extension(Some("document.PDF"), None), "pdf");
        assert_eq!(detect_extension(Some("voice.AMR"), None), "amr");
        assert_eq!(detect_extension(Some("clip.mp4"), None), "mp4");
    }

    #[test]
    fn detect_extension_from_content_type() {
        assert_eq!(detect_extension(None, Some("image/jpeg")), "jpg");
        assert_eq!(detect_extension(None, Some("image/png")), "png");
        assert_eq!(detect_extension(None, Some("audio/amr")), "amr");
        assert_eq!(detect_extension(None, Some("video/mp4")), "mp4");
        assert_eq!(detect_extension(None, Some("application/pdf")), "pdf");
    }

    #[test]
    fn detect_extension_content_type_with_params() {
        // Content-Type with charset should still match correctly
        assert_eq!(
            detect_extension(None, Some("image/jpeg; charset=utf-8")),
            "jpg"
        );
    }

    #[test]
    fn detect_extension_filename_takes_priority_over_content_type() {
        // filename extension wins over content-type
        assert_eq!(
            detect_extension(Some("file.png"), Some("image/jpeg")),
            "png"
        );
    }

    #[test]
    fn detect_extension_default_bin() {
        assert_eq!(detect_extension(None, None), "bin");
        assert_eq!(detect_extension(None, Some("application/unknown")), "bin");
    }

    #[test]
    fn detect_extension_edge_cases() {
        // File with no extension falls back to content-type or bin
        assert_eq!(detect_extension(Some("file"), Some("image/jpeg")), "jpg");
        assert_eq!(detect_extension(Some("file"), None), "bin");
        // Trailing dot — empty extension → ignored, fall through
        assert_eq!(detect_extension(Some("file."), None), "bin");
        // Non-alphanumeric characters in ext → fall through to bin
        assert_eq!(
            detect_extension(Some("file.tar.gz"), Some("application/gzip")),
            "gz"
        );
    }

    #[test]
    fn detect_extension_no_path_components() {
        // A "filename" that is actually a path should not leak path segments
        // rsplit('.').next() gives "sh" from "../../evil.sh" — which is fine (it's just the ext)
        assert_eq!(detect_extension(Some("../../evil.sh"), None), "sh");
        // We never use the filename as a path, only extract the extension from it.
    }
}
