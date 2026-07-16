//! Resolve a project-scoped path for an attachment and write its bytes through
//! the filesystem authority.

use ironclaw_filesystem::{FilesystemError, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{HostApiError, ResourceScope, ScopedPath};

/// Subdirectory, under the project mount, where landed attachments live.
pub const ATTACHMENTS_DIR: &str = "attachments";

/// Defensive ceiling on the size of a single landed attachment (25 MiB).
///
/// This is a sane default callers may pass to [`land_attachment`], not a policy
/// authority — channel adapters that know their own provider limits should pass
/// a tighter bound. It exists so the leaf write routine always has *some* cap
/// rather than persisting an unbounded `Vec<u8>`.
pub const DEFAULT_MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// Failure landing an attachment into the project filesystem.
#[derive(Debug, thiserror::Error)]
pub enum AttachmentLandingError {
    /// The resolved attachment path was not a valid scoped path (e.g. the
    /// project alias was malformed). Sanitization plus [`ScopedPath`] parsing
    /// make attacker-controlled traversal unreachable, so this is a
    /// configuration error, not a user-input error.
    #[error("invalid attachment path: {0}")]
    InvalidPath(#[from] HostApiError),
    /// The attachment exceeds the caller-supplied `max_bytes`. Rejected before
    /// any write so an oversized upload cannot grow project storage without
    /// bound — the write-side counterpart to `read_bytes_bounded`.
    #[error("attachment is {size} bytes, over the {max} byte limit")]
    TooLarge { size: usize, max: usize },
    /// Writing through the scoped filesystem failed. Notably
    /// [`FilesystemError::PermissionDenied`] when the project `MountView` lacks
    /// a write grant — landing fails closed rather than escaping the authority.
    #[error("failed to write attachment bytes: {0}")]
    Write(#[from] FilesystemError),
}

/// Identifying metadata for one attachment being landed.
///
/// Every user-influenced field is sanitized into a single safe path segment
/// before it reaches the filesystem; see [`sanitize_attachment_segment`].
#[derive(Debug, Clone, Copy)]
pub struct AttachmentLanding<'a> {
    /// Stable id of the message the attachment belongs to.
    pub message_id: &'a str,
    /// Zero-based index of this attachment within its message. Always rendered
    /// into the landed path (1-based) so two attachments on one message never
    /// collide, even when they share a filename, and it supplies the fallback
    /// filename when `filename` is absent.
    pub index: usize,
    /// Original filename, when the source provided one.
    pub filename: Option<&'a str>,
    /// Canonical extension (no dot) used to synthesize a filename when
    /// `filename` is absent — typically resolved from the attachment format
    /// registry by the caller. Falls back to `bin` when empty.
    pub fallback_extension: &'a str,
}

/// Collapse a raw, possibly attacker-controlled string into one safe path
/// segment: keep ASCII alphanumerics and `.`/`-`/`_`, replace everything else
/// with `_`, then trim leading/trailing dots (which neutralizes `..` and
/// hidden-file segments). An empty result becomes `attachment`.
fn sanitize_attachment_segment(raw: &str) -> String {
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

/// Filename portion of the landed path: the sanitized original name, or a
/// synthesized `attachment.{ext}` when the source provided none. Uniqueness
/// across attachments is carried by the index prefix in the path, not here.
fn attachment_filename(landing: &AttachmentLanding<'_>) -> String {
    match landing.filename {
        Some(name) => sanitize_attachment_segment(name),
        None => {
            let ext = sanitize_attachment_segment(landing.fallback_extension);
            let ext = if ext == "attachment" {
                "bin".to_string()
            } else {
                ext
            };
            format!("attachment.{ext}")
        }
    }
}

/// Build the [`ScopedPath`] an attachment lands at:
/// `{project_alias}/attachments/{date}/{message_id}-{index}-{filename}`, where
/// `index` is the 1-based attachment index so two attachments on one message
/// never collide even when they share a filename.
///
/// Every segment derived from the message or the upload is sanitized, and
/// [`ScopedPath::new`] additionally rejects path traversal and raw host paths,
/// so the result is always contained under `project_alias`.
pub fn attachment_scoped_path(
    project_alias: &str,
    date: &str,
    landing: &AttachmentLanding<'_>,
) -> Result<ScopedPath, AttachmentLandingError> {
    let date = sanitize_attachment_segment(date);
    let message_id = sanitize_attachment_segment(landing.message_id);
    let index = landing.index + 1;
    let filename = attachment_filename(landing);
    let full = format!(
        "{}/{ATTACHMENTS_DIR}/{date}/{message_id}-{index}-{filename}",
        project_alias.trim_end_matches('/')
    );
    ScopedPath::new(full).map_err(AttachmentLandingError::InvalidPath)
}

/// Land an attachment's bytes into the project filesystem and return the
/// [`ScopedPath`] they were written to — the value to record as the
/// attachment's storage key.
///
/// The write goes through `filesystem`, which resolves the path against the
/// scope's `MountView` and enforces its [`MountPermissions`] before touching
/// any backend. A read-only mount therefore fails closed with
/// [`FilesystemError::PermissionDenied`]. Because the agent's file tools
/// resolve through the same `MountView`, the returned `ScopedPath` is readable
/// by `file_read`/`list_dir` in this and later turns with no extra wiring.
///
/// `bytes` is rejected with [`AttachmentLandingError::TooLarge`] before any
/// write when it exceeds `max_bytes` (see [`DEFAULT_MAX_ATTACHMENT_BYTES`]).
/// The bytes are already materialized at this boundary; callers that can should
/// enforce the same bound on a streaming reader *before* buffering the full
/// `Vec<u8>`, so an oversized upload is rejected without full materialization.
///
/// [`MountPermissions`]: ironclaw_host_api::MountPermissions
pub async fn land_attachment<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    project_alias: &str,
    date: &str,
    landing: &AttachmentLanding<'_>,
    bytes: Vec<u8>,
    max_bytes: usize,
) -> Result<ScopedPath, AttachmentLandingError>
where
    F: RootFilesystem,
{
    if bytes.len() > max_bytes {
        return Err(AttachmentLandingError::TooLarge {
            size: bytes.len(),
            max: max_bytes,
        });
    }
    let path = attachment_scoped_path(project_alias, date, landing)?;
    filesystem.write_bytes(scope, &path, bytes).await?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, TenantId,
        UserId, VirtualPath,
    };

    const PROJECT_TARGET: &str = "/projects/workspace";
    // Test-only project mount alias. Production callers pass the alias read off
    // the request's `MountView`; the crate intentionally owns no default so it
    // can't drift from the composition layer that builds the mount.
    const PROJECT_ALIAS: &str = "/workspace";

    fn project_mount_view(permissions: MountPermissions) -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new(PROJECT_ALIAS).unwrap(),
            VirtualPath::new(PROJECT_TARGET).unwrap(),
            permissions,
        )])
        .unwrap()
    }

    fn scoped(
        backend: Arc<InMemoryBackend>,
        permissions: MountPermissions,
    ) -> ScopedFilesystem<InMemoryBackend> {
        ScopedFilesystem::with_fixed_view(backend, project_mount_view(permissions))
    }

    fn test_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-test").unwrap(),
            user_id: UserId::new("user-test").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn landing<'a>(message_id: &'a str, filename: Option<&'a str>) -> AttachmentLanding<'a> {
        AttachmentLanding {
            message_id,
            index: 0,
            filename,
            fallback_extension: "png",
        }
    }

    #[test]
    fn sanitize_neutralizes_separators_dots_and_empties() {
        assert_eq!(sanitize_attachment_segment("report.pdf"), "report.pdf");
        assert_eq!(sanitize_attachment_segment("a b/c"), "a_b_c");
        // Separators become `_`; only *leading/trailing* dots are trimmed, so an
        // interior `..` survives as literal filename chars (harmless — it is not
        // a `/../` path segment) while a segment that is *only* dots collapses.
        assert_eq!(sanitize_attachment_segment("../../etc"), "_.._etc");
        assert_eq!(sanitize_attachment_segment(".."), "attachment");
        assert_eq!(sanitize_attachment_segment(""), "attachment");
        assert_eq!(sanitize_attachment_segment("résumé.txt"), "r_sum_.txt");
    }

    #[test]
    fn uuid_message_ids_are_sanitization_stable_so_distinct_messages_never_alias() {
        // In production `message_id` is a v4 UUID (Display = lowercase hex +
        // hyphens). `sanitize_attachment_segment` keeps `[alnum.-_]`, so a UUID
        // passes through unchanged — two distinct message ids can never collapse
        // to one path segment. Path uniqueness is carried by (message_id, index);
        // the filename segment is cosmetic and its lossy sanitization cannot
        // cause an overwrite because no two attachments share that prefix.
        let uuid = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
        assert_eq!(sanitize_attachment_segment(uuid), uuid);
        let other = "a1b2c3d4-0000-4000-8000-000000000000";
        assert_ne!(
            sanitize_attachment_segment(uuid),
            sanitize_attachment_segment(other),
            "distinct uuid message ids must not alias to one path segment"
        );
    }

    #[test]
    fn scoped_path_is_built_under_the_project_mount() {
        let path = attachment_scoped_path(
            "/workspace",
            "2026-06-09",
            &landing("msg1", Some("report.pdf")),
        )
        .unwrap();
        assert_eq!(
            path.as_str(),
            "/workspace/attachments/2026-06-09/msg1-1-report.pdf"
        );
    }

    #[test]
    fn scoped_path_synthesizes_filename_when_absent() {
        let mut meta = landing("msg1", None);
        meta.index = 2;
        meta.fallback_extension = "jpg";
        let path = attachment_scoped_path("/workspace", "2026-06-09", &meta).unwrap();
        assert_eq!(
            path.as_str(),
            "/workspace/attachments/2026-06-09/msg1-3-attachment.jpg"
        );
    }

    #[test]
    fn same_named_attachments_on_one_message_never_collide() {
        let mut first = landing("msg1", Some("report.pdf"));
        first.index = 0;
        let mut second = landing("msg1", Some("report.pdf"));
        second.index = 1;
        let first = attachment_scoped_path("/workspace", "2026-06-09", &first).unwrap();
        let second = attachment_scoped_path("/workspace", "2026-06-09", &second).unwrap();
        assert_eq!(
            first.as_str(),
            "/workspace/attachments/2026-06-09/msg1-1-report.pdf"
        );
        assert_eq!(
            second.as_str(),
            "/workspace/attachments/2026-06-09/msg1-2-report.pdf"
        );
        assert_ne!(
            first.as_str(),
            second.as_str(),
            "same-named attachments must land at distinct paths"
        );
    }

    #[test]
    fn scoped_path_contains_traversal_inside_the_mount() {
        // A filename and message id trying to escape are sanitized to safe
        // segments and the path stays under the project alias.
        let path = attachment_scoped_path(
            "/workspace",
            "2026-06-09",
            &landing("../../escape", Some("../../../etc/passwd")),
        )
        .unwrap();
        assert!(
            path.as_str().starts_with("/workspace/attachments/"),
            "path escaped the mount: {}",
            path.as_str()
        );
        // What matters for traversal is that no path *segment* is `..`; an
        // interior `..` inside the filename segment is just literal bytes.
        assert!(
            !path.as_str().split('/').any(|segment| segment == ".."),
            "path retained a `..` traversal segment: {}",
            path.as_str()
        );
    }

    #[tokio::test]
    async fn land_then_read_round_trips_through_the_same_mount() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = scoped(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();
        let bytes = b"%PDF-1.7 hello".to_vec();

        let stored = land_attachment(
            &writer,
            &scope,
            PROJECT_ALIAS,
            "2026-06-09",
            &landing("msg1", Some("report.pdf")),
            bytes.clone(),
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("write succeeds through a read-write mount");
        assert_eq!(
            stored.as_str(),
            "/workspace/attachments/2026-06-09/msg1-1-report.pdf"
        );

        // A separate scoped filesystem over the same backend — standing in for
        // the agent's file tools in a later turn — reads the bytes back at the
        // same scoped path. Writer and reader share one authority.
        let reader = scoped(backend, MountPermissions::read_only());
        let got = reader
            .get(&scope, &stored)
            .await
            .expect("read succeeds")
            .expect("attachment is present");
        assert_eq!(got.entry.body, bytes);
    }

    #[tokio::test]
    async fn land_fails_closed_on_a_read_only_mount() {
        let backend = Arc::new(InMemoryBackend::new());
        let read_only = scoped(backend, MountPermissions::read_only());

        let err = land_attachment(
            &read_only,
            &test_scope(),
            PROJECT_ALIAS,
            "2026-06-09",
            &landing("msg1", Some("report.pdf")),
            b"bytes".to_vec(),
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect_err("write must be rejected without a write grant");

        assert!(
            matches!(
                err,
                AttachmentLandingError::Write(FilesystemError::PermissionDenied { .. })
            ),
            "expected fail-closed PermissionDenied, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn land_rejects_oversized_attachment_before_writing() {
        let backend = Arc::new(InMemoryBackend::new());
        let writer = scoped(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();

        let err = land_attachment(
            &writer,
            &scope,
            PROJECT_ALIAS,
            "2026-06-09",
            &landing("msg1", Some("big.bin")),
            vec![0u8; 9],
            8,
        )
        .await
        .expect_err("over-limit attachment must be rejected");
        assert!(
            matches!(err, AttachmentLandingError::TooLarge { size: 9, max: 8 }),
            "expected TooLarge, got: {err:?}"
        );

        // The rejection happens before any write, so nothing landed.
        let reader = scoped(backend, MountPermissions::read_only());
        let path = attachment_scoped_path(
            PROJECT_ALIAS,
            "2026-06-09",
            &landing("msg1", Some("big.bin")),
        )
        .unwrap();
        assert!(
            reader
                .get(&scope, &path)
                .await
                .expect("read succeeds")
                .is_none(),
            "oversized attachment must not have been written"
        );
    }

    #[tokio::test]
    async fn same_named_attachments_land_without_clobbering() {
        // Two attachments in one message sharing a filename must keep their own
        // bytes — the index prefix gives them distinct paths, so the second land
        // does not overwrite the first.
        let backend = Arc::new(InMemoryBackend::new());
        let writer = scoped(Arc::clone(&backend), MountPermissions::read_write());
        let scope = test_scope();

        let mut first = landing("msg1", Some("photo.jpg"));
        first.index = 0;
        let mut second = landing("msg1", Some("photo.jpg"));
        second.index = 1;

        let first_path = land_attachment(
            &writer,
            &scope,
            PROJECT_ALIAS,
            "2026-06-09",
            &first,
            b"first-bytes".to_vec(),
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("first land succeeds");
        let second_path = land_attachment(
            &writer,
            &scope,
            PROJECT_ALIAS,
            "2026-06-09",
            &second,
            b"second-bytes".to_vec(),
            DEFAULT_MAX_ATTACHMENT_BYTES,
        )
        .await
        .expect("second land succeeds");

        assert_ne!(first_path.as_str(), second_path.as_str());
        let reader = scoped(backend, MountPermissions::read_only());
        let first_back = reader
            .get(&scope, &first_path)
            .await
            .expect("read succeeds")
            .expect("first attachment present");
        let second_back = reader
            .get(&scope, &second_path)
            .await
            .expect("read succeeds")
            .expect("second attachment present");
        assert_eq!(first_back.entry.body, b"first-bytes");
        assert_eq!(second_back.entry.body, b"second-bytes");
    }
}
