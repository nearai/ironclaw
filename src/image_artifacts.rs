//! Helpers for storing image bytes as reusable local artifacts.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_common::attachment::{AttachmentKind, IncomingAttachment};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub(crate) const MAX_IMAGE_ARTIFACT_BYTES: usize = 10 * 1024 * 1024;

pub(crate) fn default_image_artifact_root() -> PathBuf {
    crate::bootstrap::ironclaw_base_dir().join("image-artifacts")
}

fn normalize_image_media_type(media_type: &str) -> Result<&'static str, String> {
    let media_type = media_type
        .split(';')
        .next()
        .unwrap_or(media_type)
        .trim()
        .to_ascii_lowercase();
    match media_type.as_str() {
        "image/png" => Ok("image/png"),
        "image/jpeg" | "image/jpg" => Ok("image/jpeg"),
        "image/gif" => Ok("image/gif"),
        "image/webp" => Ok("image/webp"),
        other => Err(format!("unsupported image media type: {other}")),
    }
}

fn image_artifact_extension(media_type: &str) -> Result<&'static str, String> {
    match normalize_image_media_type(media_type)? {
        "image/png" => Ok("png"),
        "image/jpeg" => Ok("jpg"),
        "image/gif" => Ok("gif"),
        "image/webp" => Ok("webp"),
        normalized => Err(format!("unsupported image media type: {normalized}")),
    }
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .take(96)
        .collect::<String>();
    let sanitized = sanitized.trim_matches('.').trim();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "unknown".to_string()
    } else {
        sanitized.to_string()
    }
}

fn image_artifact_dir(root: &Path, user_id: &str, thread_id: Uuid) -> PathBuf {
    root.join(sanitize_path_segment(user_id))
        .join(thread_id.to_string())
}

fn image_artifact_filename(artifact_id: &str, attempt: usize, ext: &str) -> String {
    let stem = sanitize_path_segment(artifact_id);
    if attempt == 0 {
        format!("{stem}.{ext}")
    } else {
        format!("{stem}-{attempt}.{ext}")
    }
}

fn image_attachment_artifact_id(message_id: Uuid, attachment_id: &str, index: usize) -> String {
    if attachment_id.trim().is_empty() {
        format!("{message_id}-attachment-{index}")
    } else {
        format!("{message_id}-{attachment_id}")
    }
}

fn max_base64_encoded_len(decoded_limit: usize) -> usize {
    decoded_limit.div_ceil(3) * 4
}

fn ensure_base64_decoded_len_within_limit(
    encoded: &str,
    decoded_limit: usize,
) -> Result<(), String> {
    let max_encoded_len = max_base64_encoded_len(decoded_limit);
    if encoded.len() > max_encoded_len {
        return Err(format!(
            "image data URL exceeds {} byte limit",
            decoded_limit
        ));
    }

    let trailing_padding = encoded
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    if trailing_padding > 2 {
        return Err("invalid image data URL base64 padding".to_string());
    }
    if trailing_padding > 0 && !encoded.len().is_multiple_of(4) {
        return Err("invalid image data URL base64 padding".to_string());
    }

    let decoded_upper_bound = match encoded.len() % 4 {
        0 => encoded.len() / 4 * 3 - trailing_padding,
        2 => encoded.len() / 4 * 3 + 1,
        3 => encoded.len() / 4 * 3 + 2,
        _ => encoded.len() / 4 * 3 + 3,
    };
    if decoded_upper_bound > decoded_limit {
        return Err(format!(
            "image data URL exceeds {} byte limit",
            decoded_limit
        ));
    }

    Ok(())
}

pub(crate) async fn persist_image_artifact(
    root: Option<&Path>,
    bytes: &[u8],
    media_type: &str,
    user_id: &str,
    thread_id: Uuid,
    artifact_id: &str,
) -> Result<String, String> {
    let normalized_media_type = normalize_image_media_type(media_type)?;
    if bytes.is_empty() {
        return Err("image artifact is empty".to_string());
    }
    if bytes.len() > MAX_IMAGE_ARTIFACT_BYTES {
        return Err(format!(
            "image artifact exceeds {} byte limit",
            MAX_IMAGE_ARTIFACT_BYTES
        ));
    }

    let default_root;
    let root = match root {
        Some(root) => root,
        None => {
            default_root = default_image_artifact_root();
            default_root.as_path()
        }
    };
    let ext = image_artifact_extension(normalized_media_type)?;
    let parent = image_artifact_dir(root, user_id, thread_id);
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|e| format!("failed to create image artifact directory: {e}"))?;

    for attempt in 0..1000 {
        let path = parent.join(image_artifact_filename(artifact_id, attempt, ext));
        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("failed to create image artifact: {err}")),
        };

        if let Err(err) = file.write_all(bytes).await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(format!("failed to write image artifact: {err}"));
        }
        if let Err(err) = file.flush().await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(format!("failed to flush image artifact: {err}"));
        }

        let display_path = tokio::fs::canonicalize(&path).await.unwrap_or(path);
        return Ok(display_path.to_string_lossy().into_owned());
    }

    Err("failed to allocate unique image artifact path".to_string())
}

pub(crate) async fn persist_incoming_image_attachment_artifact(
    root: Option<&Path>,
    attachment: &IncomingAttachment,
    user_id: &str,
    thread_id: Uuid,
    message_id: Uuid,
    index: usize,
) -> Result<Option<String>, String> {
    if attachment.kind != AttachmentKind::Image || attachment.data.is_empty() {
        return Ok(None);
    }

    persist_image_artifact(
        root,
        &attachment.data,
        &attachment.mime_type,
        user_id,
        thread_id,
        &image_attachment_artifact_id(message_id, &attachment.id, index),
    )
    .await
    .map(Some)
}

pub(crate) async fn load_image_artifact_data_url(path: &str) -> Result<String, String> {
    load_image_artifact_data_url_at(None, path).await
}

pub(crate) async fn load_image_artifact_data_url_at(
    root: Option<&Path>,
    path: &str,
) -> Result<String, String> {
    let default_root;
    let root = match root {
        Some(root) => root,
        None => {
            default_root = default_image_artifact_root();
            default_root.as_path()
        }
    };
    let resolved = crate::tools::builtin::path_utils::validate_path(path, Some(root))
        .map_err(|e| e.to_string())?;
    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("failed to read image artifact: {e}"))?;
    if bytes.is_empty() {
        return Err("image artifact is empty".to_string());
    }
    if bytes.len() > MAX_IMAGE_ARTIFACT_BYTES {
        return Err(format!(
            "image artifact exceeds {} byte limit",
            MAX_IMAGE_ARTIFACT_BYTES
        ));
    }
    let guessed = mime_guess::from_path(&resolved)
        .first_raw()
        .ok_or_else(|| "failed to determine image artifact media type".to_string())?;
    let media_type = normalize_image_media_type(guessed)?;
    Ok(format!(
        "data:{media_type};base64,{}",
        BASE64_STANDARD.encode(bytes)
    ))
}

pub(crate) fn decode_image_data_url(data_url: &str) -> Result<(String, Vec<u8>), String> {
    let Some(rest) = data_url.strip_prefix("data:") else {
        return Err("image data URL must start with data:".to_string());
    };
    let (metadata, data) = rest
        .split_once(',')
        .ok_or_else(|| "image data URL is missing comma separator".to_string())?;
    let mut metadata_parts = metadata.split(';');
    let media_type = metadata_parts
        .next()
        .ok_or_else(|| "image data URL is missing media type".to_string())?;
    if !metadata_parts.any(|part| part.eq_ignore_ascii_case("base64")) {
        return Err("image data URL must be base64 encoded".to_string());
    }
    let normalized_media_type = normalize_image_media_type(media_type)?;
    ensure_base64_decoded_len_within_limit(data, MAX_IMAGE_ARTIFACT_BYTES)?;
    let bytes = BASE64_STANDARD
        .decode(data)
        .map_err(|e| format!("invalid image data URL base64: {e}"))?;
    if bytes.len() > MAX_IMAGE_ARTIFACT_BYTES {
        return Err(format!(
            "image data URL exceeds {} byte limit",
            MAX_IMAGE_ARTIFACT_BYTES
        ));
    }
    Ok((normalized_media_type.to_string(), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incoming_attachment(
        kind: AttachmentKind,
        id: &str,
        mime_type: &str,
        data: &[u8],
    ) -> IncomingAttachment {
        IncomingAttachment {
            id: id.to_string(),
            kind,
            mime_type: mime_type.to_string(),
            filename: None,
            size_bytes: Some(data.len() as u64),
            source_url: None,
            storage_key: None,
            local_path: None,
            extracted_text: None,
            data: data.to_vec(),
            duration_secs: None,
        }
    }

    #[tokio::test]
    async fn persists_image_artifact_under_sanitized_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let thread_id = Uuid::new_v4();

        let path = persist_image_artifact(
            Some(dir.path()),
            b"image-bytes",
            "image/png",
            "../alice@example.com",
            thread_id,
            "call/img:0",
        )
        .await
        .expect("persisted");

        assert!(path.ends_with(".png"));
        assert!(path.contains("alice_example.com"));
        assert!(path.contains("call_img_0.png"));
        assert_eq!(tokio::fs::read(path).await.expect("read"), b"image-bytes");
    }

    #[tokio::test]
    async fn persist_image_artifact_does_not_overwrite_existing_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let thread_id = Uuid::new_v4();

        let first = persist_image_artifact(
            Some(dir.path()),
            b"first",
            "image/png",
            "alice",
            thread_id,
            "call_img_0",
        )
        .await
        .expect("first persisted");
        let second = persist_image_artifact(
            Some(dir.path()),
            b"second",
            "image/png",
            "alice",
            thread_id,
            "call_img_0",
        )
        .await
        .expect("second persisted");

        assert_ne!(first, second);
        assert!(first.ends_with("call_img_0.png"));
        assert!(second.ends_with("call_img_0-1.png"));
        assert_eq!(tokio::fs::read(first).await.expect("read first"), b"first");
        assert_eq!(
            tokio::fs::read(second).await.expect("read second"),
            b"second"
        );
    }

    #[tokio::test]
    async fn persists_incoming_image_attachment_artifact_with_shared_id_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let thread_id = Uuid::new_v4();
        let message_id = Uuid::new_v4();
        let attachment =
            incoming_attachment(AttachmentKind::Image, "channel/file:1", "image/png", b"png");

        let path = persist_incoming_image_attachment_artifact(
            Some(dir.path()),
            &attachment,
            "alice",
            thread_id,
            message_id,
            2,
        )
        .await
        .expect("persisted")
        .expect("image attachment should persist");

        assert!(path.ends_with(".png"));
        assert!(path.contains(&message_id.to_string()));
        assert!(path.contains("channel_file_1"));
        assert_eq!(tokio::fs::read(path).await.expect("read"), b"png");
    }

    #[tokio::test]
    async fn skips_non_image_incoming_attachment_artifacts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let attachment = incoming_attachment(
            AttachmentKind::Document,
            "doc",
            "application/pdf",
            b"not an image",
        );

        let path = persist_incoming_image_attachment_artifact(
            Some(dir.path()),
            &attachment,
            "alice",
            Uuid::new_v4(),
            Uuid::new_v4(),
            0,
        )
        .await
        .expect("non-image attachments should be ignored");

        assert!(path.is_none());
    }

    #[test]
    fn decodes_image_data_url() {
        let data_url = format!("data:image/png;base64,{}", BASE64_STANDARD.encode(b"png"));

        let (media_type, bytes) = decode_image_data_url(&data_url).expect("decoded");

        assert_eq!(media_type, "image/png");
        assert_eq!(bytes, b"png");
    }

    #[test]
    fn rejects_base64_that_can_decode_past_limit_before_decode() {
        let encoded = "A".repeat(max_base64_encoded_len(1));

        let err = ensure_base64_decoded_len_within_limit(&encoded, 1).expect_err("rejected");

        assert!(err.contains("1 byte limit"));
    }

    #[test]
    fn accepts_base64_at_limit_when_padding_keeps_decoded_size_in_bounds() {
        ensure_base64_decoded_len_within_limit("AA==", 1).expect("within limit");
    }

    #[test]
    fn rejects_non_image_data_url() {
        let data_url = format!(
            "data:text/plain;base64,{}",
            BASE64_STANDARD.encode(b"hello")
        );

        let err = decode_image_data_url(&data_url).expect_err("rejected");

        assert!(err.contains("unsupported image media type"));
    }
}
