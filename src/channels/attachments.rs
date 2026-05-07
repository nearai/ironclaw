//! Shared attachment helpers for channel ingestion and persistence.

use std::path::{Path, PathBuf};

use crate::channels::{AttachmentKind, IncomingAttachment};

/// Maximum decoded size per inline attachment.
pub(crate) const MAX_INLINE_ATTACHMENT_BYTES: usize = 7 * 1024 * 1024;
/// Maximum total decoded size across all inline attachments in a message.
pub(crate) const MAX_INLINE_TOTAL_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
/// Maximum number of inline attachments in a single message.
pub(crate) const MAX_INLINE_ATTACHMENTS: usize = 5;

/// Sanitize a path segment (user_id, message_id, project_id, filename) so
/// it can't escape the attachments directory or produce a hidden / control-
/// character path. Used by every persistence call site.
pub(crate) fn sanitize_attachment_segment(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('.');
    if sanitized.is_empty() {
        "attachment".to_string()
    } else {
        sanitized.to_string()
    }
}

pub(crate) fn fallback_attachment_filename(index: usize, mime_type: &str) -> String {
    let ext = attachment_extension_for_mime(mime_type);
    format!("attachment-{}.{}", index + 1, ext)
}

/// Write a single attachment's decoded bytes to disk and stamp its
/// `local_path` so downstream `format_attachment` can emit a `project_path`
/// attribute. No-op when the attachment is empty (already-stored or
/// metadata-only) or already has `local_path` set.
///
/// Returns `Ok(true)` when bytes were written, `Ok(false)` for a deliberate
/// skip, and an error for actual IO failures.
pub(crate) async fn persist_attachment_at(
    absolute_path: &Path,
    relative_path: &str,
    attachment: &mut IncomingAttachment,
) -> Result<bool, std::io::Error> {
    if attachment.data.is_empty() || attachment.local_path.is_some() {
        return Ok(false);
    }
    let Some(parent) = absolute_path.parent() else {
        return Ok(false);
    };
    tokio::fs::create_dir_all(parent).await?;
    tokio::fs::write(absolute_path, &attachment.data).await?;
    attachment.local_path = Some(relative_path.to_string());
    Ok(true)
}

/// Subdirectory under `<owner>/` for attachments uploaded through the v1
/// dispatcher (no project context). Distinct from v2's
/// `<owner>/<project_id>/<date>/...` so the same `/api/attachments/` route
/// can serve both layouts without ambiguity.
const LEGACY_SUBDIR: &str = ".legacy";

/// HOME-relative root used by every persistence call site. v2's
/// `persist_project_attachments` and v1's `persist_legacy_image_attachments`
/// both produce paths beginning with this prefix so the HTTP route at
/// `/api/attachments/<rest>` can map any URL back to a single on-disk tree.
pub(crate) const ATTACHMENT_PATH_PREFIX: &str = ".ironclaw/attachments";

/// HOME directory (parent of `~/.ironclaw`). Both v1 and v2 join their
/// HOME-relative paths against this root.
pub(crate) fn attachment_storage_root() -> PathBuf {
    let base_dir = crate::bootstrap::ironclaw_base_dir();
    base_dir.parent().map(PathBuf::from).unwrap_or(base_dir)
}

/// Build the on-disk relative path for a v1-uploaded attachment. The path is
/// HOME-relative and matches v2's `.ironclaw/attachments/<owner>/...`
/// layout so `/api/attachments/<owner>/<rest>` works for both.
pub(crate) fn legacy_attachment_relative_path(
    user_id: &str,
    message_id: &str,
    attachment_index: usize,
    attachment: &IncomingAttachment,
) -> String {
    let owner = sanitize_attachment_segment(user_id);
    let msg_id = sanitize_attachment_segment(message_id);
    let filename = attachment
        .filename
        .as_deref()
        .map(sanitize_attachment_segment)
        .unwrap_or_else(|| fallback_attachment_filename(attachment_index, &attachment.mime_type));
    format!(
        "{}/{}/{}/{}/{}",
        ATTACHMENT_PATH_PREFIX, owner, LEGACY_SUBDIR, msg_id, filename
    )
}

/// Persist user-uploaded image attachments to disk under the legacy v1
/// subtree and stamp `local_path` so `/api/attachments/...` can serve them
/// after a refresh. v2 has its own project-aware persistence path in
/// `bridge::router::persist_project_attachments`.
///
/// Only image attachments are persisted today — that matches the user-facing
/// regression we're fixing (#1341 follow-up). Non-image inline attachments
/// have their text extracted into the XML body and don't need disk recovery.
///
/// IO errors are logged at `debug!` and swallowed: a persistence failure
/// should not block the LLM call. The XML still carries metadata; the worst
/// the user sees on refresh is the existing file-card fallback.
pub(crate) async fn persist_legacy_image_attachments(
    user_id: &str,
    message_id: &str,
    attachments: &mut [IncomingAttachment],
) {
    let storage_root = attachment_storage_root();
    for (index, attachment) in attachments.iter_mut().enumerate() {
        if attachment.kind != AttachmentKind::Image {
            continue;
        }
        let relative_path = legacy_attachment_relative_path(user_id, message_id, index, attachment);
        let absolute_path = storage_root.join(Path::new(&relative_path));
        if let Err(e) = persist_attachment_at(&absolute_path, &relative_path, attachment).await {
            // debug! (not warn!) per src/agent/CLAUDE.md: warn! corrupts the
            // REPL/TUI display, and a persist failure here is recoverable —
            // the user just sees a file-card fallback on refresh.
            tracing::debug!(
                user_id = %user_id,
                message_id = %message_id,
                error = %e,
                "v1: failed to persist user image attachment"
            );
        }
    }
}

fn base_mime_type(mime: &str) -> &str {
    mime.split(';').next().unwrap_or(mime).trim()
}

pub(crate) fn attachment_extension_for_mime(mime: &str) -> &'static str {
    match base_mime_type(mime) {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/markdown" => "md",
        "text/csv" => "csv",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        other if other.starts_with("image/") => "jpg",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::IncomingAttachment;

    #[test]
    fn attachment_extension_handles_common_types_and_parameters() {
        assert_eq!(
            super::attachment_extension_for_mime("text/plain; charset=utf-8"),
            "txt"
        );
        assert_eq!(
            super::attachment_extension_for_mime(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            "docx"
        );
        assert_eq!(
            super::attachment_extension_for_mime(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            ),
            "xlsx"
        );
        assert_eq!(super::attachment_extension_for_mime("audio/x-wav"), "wav");
    }

    fn image_attachment(filename: &str, bytes: &[u8]) -> IncomingAttachment {
        IncomingAttachment {
            id: "a".to_string(),
            kind: AttachmentKind::Image,
            mime_type: "image/png".to_string(),
            filename: Some(filename.to_string()),
            size_bytes: Some(bytes.len() as u64),
            source_url: None,
            storage_key: None,
            local_path: None,
            extracted_text: None,
            data: bytes.to_vec(),
            duration_secs: None,
        }
    }

    #[test]
    fn legacy_relative_path_uses_legacy_subdir_and_sanitizes_segments() {
        let att = image_attachment("photo.png", b"\x89PNG");
        let path = legacy_attachment_relative_path("alice", "msg-123", 0, &att);
        // Layout: <prefix>/<owner>/.legacy/<msg>/<file> — same as v2's
        // `<prefix>/<owner>/<project>/<date>/<file>` so the same HTTP route
        // can serve both.
        assert_eq!(
            path,
            ".ironclaw/attachments/alice/.legacy/msg-123/photo.png"
        );
    }

    #[test]
    fn legacy_relative_path_synthesizes_filename_when_missing() {
        let mut att = image_attachment("ignored", b"\x89PNG");
        att.filename = None;
        let path = legacy_attachment_relative_path("alice", "msg-1", 2, &att);
        // Index is offset by 1 in fallback_attachment_filename → "attachment-3.png".
        assert!(
            path.ends_with("/.legacy/msg-1/attachment-3.png"),
            "unexpected synthesized path: {path}",
        );
    }

    #[test]
    fn legacy_relative_path_rejects_path_traversal_in_user_id() {
        let att = image_attachment("photo.png", b"\x89PNG");
        let path = legacy_attachment_relative_path("../etc", "msg-1", 0, &att);
        // Sanitize maps `/` to `_` and `trim_matches('.')` strips the leading
        // `..`, leaving a benign directory name. The key security property
        // is that no `..` segment survives — we don't care that the result
        // is exactly `_etc`, only that it can't escape the attachments tree.
        assert!(!path.contains("../"), "path traversal leaked: {path}");
        assert!(
            !path.split('/').any(|seg| seg == ".."),
            "literal `..` segment survived: {path}",
        );
    }

    #[tokio::test]
    async fn persist_legacy_writes_image_and_stamps_local_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let absolute = dir.path().join("subdir").join("img.png");
        let mut att = image_attachment("img.png", b"\x89PNG\r\n\x1a\n");
        let written = persist_attachment_at(&absolute, "rel/path", &mut att)
            .await
            .expect("persist ok");
        assert!(written, "expected persist to write the file");
        assert_eq!(att.local_path.as_deref(), Some("rel/path"));
        let on_disk = tokio::fs::read(&absolute).await.expect("read back");
        assert_eq!(on_disk, b"\x89PNG\r\n\x1a\n");
    }

    #[tokio::test]
    async fn persist_attachment_at_skips_when_data_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let absolute = dir.path().join("img.png");
        let mut att = image_attachment("img.png", b"");
        let written = persist_attachment_at(&absolute, "rel", &mut att)
            .await
            .expect("ok");
        assert!(!written, "empty data should be a deliberate skip");
        assert!(att.local_path.is_none(), "skip must not stamp local_path");
        assert!(!absolute.exists(), "skip must not create the file");
    }

    #[tokio::test]
    async fn persist_attachment_at_skips_when_local_path_already_set() {
        // Defends against double-persist: a second pass must not overwrite
        // the path or rewrite the file.
        let dir = tempfile::tempdir().expect("tempdir");
        let absolute = dir.path().join("img.png");
        let mut att = image_attachment("img.png", b"\x89PNG");
        att.local_path = Some("preexisting".to_string());
        let written = persist_attachment_at(&absolute, "ignored", &mut att)
            .await
            .expect("ok");
        assert!(!written);
        assert_eq!(att.local_path.as_deref(), Some("preexisting"));
        assert!(!absolute.exists());
    }
}
