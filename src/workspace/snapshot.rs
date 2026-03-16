//! Workspace snapshot and hydration for disaster recovery.
//!
//! Periodically exports core workspace documents (identity files + MEMORY.md
//! and context/\*\*) to a structured Markdown file on disk. On startup, if the
//! workspace database is empty and a snapshot exists, documents are restored.
//!
//! The snapshot format uses byte-length–prefixed sections so that document
//! content containing any HTML comments, Markdown headers, or other markup
//! round-trips exactly (byte-level fidelity).
//!
//! # Security
//!
//! The snapshot file contains sensitive workspace content in plaintext,
//! including identity files (IDENTITY.md, SOUL.md, USER.md, AGENTS.md),
//! memory (MEMORY.md), heartbeat configuration (HEARTBEAT.md), tool notes
//! (TOOLS.md), and all context/\*\* documents. On Unix systems, snapshot and
//! state files are automatically written with 0600 permissions (owner
//! read/write only). The snapshot path should be in a user-controlled
//! directory with appropriate access restrictions.
//!
//! ```text
//! ┌───────────────────────────────────────────────┐
//! │            Snapshot Pass                       │
//! │                                               │
//! │  0. Acquire SNAPSHOT_RUNNING guard             │
//! │  1. Check cadence (skip if ran recently)       │
//! │  2. Validate user_id marker safety             │
//! │  3. Collect allowlist docs + context/**         │
//! │  4. Render length-prefixed Markdown             │
//! │  5. Atomic write (tmp + rename)                │
//! │  6. Update cadence state (only on success)     │
//! └───────────────────────────────────────────────┘
//! ```

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::WorkspaceError;
use crate::workspace::Workspace;
use crate::workspace::document::paths;

/// Root documents included in snapshot (exact match).
const SNAPSHOT_DOCS: &[&str] = &[
    paths::MEMORY,
    paths::IDENTITY,
    paths::SOUL,
    paths::AGENTS,
    paths::USER,
    paths::HEARTBEAT,
    paths::TOOLS,
];

/// Directory prefixes included in snapshot (prefix match on list_all results).
const SNAPSHOT_PREFIXES: &[&str] = &["context/"];

/// Current snapshot format version.
const SNAPSHOT_VERSION: &str = "v1";

/// Global guard preventing concurrent snapshot passes.
/// Independent of hygiene's `RUNNING` guard.
static SNAPSHOT_RUNNING: AtomicBool = AtomicBool::new(false);

/// Regex matching well-formed begin markers with numeric length.
static BEGIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    // safety: hardcoded regex literal, cannot fail
    Regex::new(r"<!-- begin: (.+) length:(\d+) -->").expect("BEGIN_RE")
});

/// Regex matching begin markers with any (possibly non-numeric) length value.
/// Used to detect malformed markers that `BEGIN_RE` would silently skip.
static MALFORMED_BEGIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    // safety: hardcoded regex literal, cannot fail
    Regex::new(r"<!-- begin: .+ length:\S+ -->").expect("MALFORMED_BEGIN_RE")
});

/// Workspace-side snapshot configuration (paths already resolved for a user).
#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    pub enabled: bool,
    pub cadence_hours: u32,
    pub snapshot_path: PathBuf,
    pub state_path: PathBuf,
}

/// Report from a snapshot pass.
#[derive(Debug)]
pub struct SnapshotReport {
    /// Number of documents exported.
    pub documents_exported: u32,
    /// Path of the snapshot file written (None if skipped).
    pub snapshot_path: Option<PathBuf>,
    /// True if the pass was skipped (disabled, cadence, or concurrent guard).
    pub skipped: bool,
}

/// Report from a hydration pass.
#[derive(Debug)]
pub struct HydrationReport {
    /// Number of documents restored from snapshot.
    pub restored: u32,
    /// Number of documents skipped (already exist in workspace).
    pub skipped: u32,
    /// Number of documents rejected (outside allowlist or invalid).
    pub rejected: u32,
}

/// Errors specific to snapshot operations.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("IO error: {reason}")]
    Io { reason: String },
    #[error("workspace error: {0}")]
    Workspace(#[from] WorkspaceError),
    #[error("invalid snapshot format: {reason}")]
    Format { reason: String },
    #[error("user ID mismatch: snapshot has '{snapshot}', workspace has '{workspace}'")]
    UserMismatch { snapshot: String, workspace: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotState {
    last_run: DateTime<Utc>,
}

// ─── Public API ──────────────────────────────────────────────────────

/// Run a snapshot pass if the cadence has elapsed.
///
/// Best-effort for individual documents: read failures are logged and skipped.
/// `list_all()` and file I/O failures propagate as errors.
///
/// Cadence state is updated ONLY after the snapshot file is successfully
/// written (atomic rename). This differs from hygiene's pre-write strategy
/// because missing a backup window is costlier than repeating a cleanup.
pub async fn snapshot_if_due(
    workspace: &Workspace,
    config: &SnapshotConfig,
) -> Result<SnapshotReport, SnapshotError> {
    let skipped = || {
        Ok(SnapshotReport {
            documents_exported: 0,
            snapshot_path: None,
            skipped: true,
        })
    };

    if !config.enabled {
        return skipped();
    }

    // Acquire process-level concurrency guard.
    if SNAPSHOT_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        tracing::debug!("workspace snapshot: skipping (another pass is running)");
        return skipped();
    }
    let _guard = SnapshotRunningGuard;

    // Check cadence.
    if let Some(state) = load_state(&config.state_path).await {
        let elapsed = Utc::now().signed_duration_since(state.last_run);
        let cadence = chrono::Duration::hours(i64::from(config.cadence_hours));
        if elapsed < cadence {
            tracing::debug!(
                hours_since_last = elapsed.num_hours(),
                cadence_hours = config.cadence_hours,
                "workspace snapshot: skipping (cadence not elapsed)"
            );
            return skipped();
        }
    }

    // Validate user_id for marker safety (fail the entire snapshot if unsafe).
    // Whitespace in user_id would break metadata parsing (split_whitespace).
    validate_marker_safe(workspace.user_id(), "user_id")?;
    if workspace.user_id().contains(char::is_whitespace) {
        return Err(SnapshotError::Format {
            reason: format!(
                "user_id contains whitespace, which is unsafe for snapshot metadata: {:?}",
                workspace.user_id()
            ),
        });
    }

    // Collect documents.
    let mut documents: Vec<(String, String)> = Vec::new();

    // Root allowlist documents (exact match).
    for &path in SNAPSHOT_DOCS {
        match workspace.read(path).await {
            Ok(doc) => {
                if let Err(e) = validate_marker_safe(path, "path") {
                    tracing::warn!(path, "snapshot: skipping, unsafe path: {e}");
                    continue;
                }
                documents.push((path.to_string(), doc.content));
            }
            Err(WorkspaceError::DocumentNotFound { .. }) => {}
            Err(e) => tracing::warn!(path, "snapshot: read failed, skipping: {e}"),
        }
    }

    // context/** via list_all + prefix filter (covers nested dirs).
    let all_paths = workspace.list_all().await?;
    for path in all_paths
        .iter()
        .filter(|p| SNAPSHOT_PREFIXES.iter().any(|pfx| p.starts_with(pfx)))
    {
        if let Err(e) = validate_marker_safe(path, "path") {
            tracing::warn!(path, "snapshot: skipping, unsafe path: {e}");
            continue;
        }
        match workspace.read(path).await {
            Ok(doc) => documents.push((path.clone(), doc.content)),
            Err(e) => tracing::warn!(path, "snapshot: read failed, skipping: {e}"),
        }
    }

    let count = documents.len() as u32;

    // Render snapshot.
    let snapshot_text = render_snapshot(workspace.user_id(), &documents);

    // Atomic write: .tmp → rename.
    let tmp_path = config.snapshot_path.with_extension("md.tmp");
    if let Some(dir) = config.snapshot_path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| SnapshotError::Io {
                reason: format!("create dir {}: {e}", dir.display()),
            })?;
    }
    tokio::fs::write(&tmp_path, &snapshot_text)
        .await
        .map_err(|e| SnapshotError::Io {
            reason: format!("write {}: {e}", tmp_path.display()),
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(&tmp_path, perms)
            .await
            .map_err(|e| SnapshotError::Io {
                reason: format!("set permissions {}: {e}", tmp_path.display()),
            })?;
    }

    if let Err(e) = tokio::fs::rename(&tmp_path, &config.snapshot_path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(SnapshotError::Io {
            reason: format!("rename to {}: {e}", config.snapshot_path.display()),
        });
    }

    // Update cadence ONLY after successful write.
    save_state(&config.state_path).await;

    tracing::info!(
        documents_exported = count,
        path = %config.snapshot_path.display(),
        "workspace snapshot exported"
    );

    Ok(SnapshotReport {
        documents_exported: count,
        snapshot_path: Some(config.snapshot_path.clone()),
        skipped: false,
    })
}

/// Restore workspace documents from a snapshot file.
///
/// Idempotent: documents that already exist in the workspace are skipped.
/// Validates snapshot version and user_id ownership before restoring.
/// Paths outside the snapshot allowlist are rejected.
pub async fn hydrate_from_snapshot(
    workspace: &Workspace,
    path: &Path,
) -> Result<HydrationReport, SnapshotError> {
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| SnapshotError::Io {
            reason: format!("read {}: {e}", path.display()),
        })?;

    // Parse and validate metadata.
    let first_line = text.lines().next().ok_or_else(|| SnapshotError::Format {
        reason: "empty snapshot file".into(),
    })?;
    let (version, snapshot_user_id, _created) = parse_metadata(first_line)?;

    if version != SNAPSHOT_VERSION {
        return Err(SnapshotError::Format {
            reason: format!("unsupported version '{version}', expected '{SNAPSHOT_VERSION}'"),
        });
    }
    if snapshot_user_id != workspace.user_id() {
        return Err(SnapshotError::UserMismatch {
            snapshot: snapshot_user_id,
            workspace: workspace.user_id().to_string(),
        });
    }

    // Parse documents (strict: any format error → SnapshotError::Format).
    let sections = parse_snapshot_documents(&text)?;

    // Restore each document.
    let mut report = HydrationReport {
        restored: 0,
        skipped: 0,
        rejected: 0,
    };
    for (doc_path, content) in sections {
        // Validate allowlist.
        if !is_snapshot_path(&doc_path) {
            tracing::warn!(
                path = doc_path.as_str(),
                "snapshot hydration: path outside allowlist, rejecting"
            );
            report.rejected += 1;
            continue;
        }
        // Skip if already exists (idempotent).
        match workspace.exists(&doc_path).await {
            Ok(true) => {
                report.skipped += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(
                    path = doc_path.as_str(),
                    "snapshot hydration: exists check failed: {e}"
                );
                report.rejected += 1;
                continue;
            }
        }
        // Write to workspace.
        match workspace.write(&doc_path, &content).await {
            Ok(_) => report.restored += 1,
            Err(e) => {
                tracing::warn!(
                    path = doc_path.as_str(),
                    "snapshot hydration: write failed: {e}"
                );
                report.rejected += 1;
            }
        }
    }

    Ok(report)
}

// ─── Internal helpers ────────────────────────────────────────────────

/// RAII guard that clears `SNAPSHOT_RUNNING` on drop.
struct SnapshotRunningGuard;

impl Drop for SnapshotRunningGuard {
    fn drop(&mut self) {
        SNAPSHOT_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Check if a path is in the snapshot allowlist.
fn is_snapshot_path(path: &str) -> bool {
    SNAPSHOT_DOCS.contains(&path)
        || SNAPSHOT_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

/// Reject values that would break HTML comment syntax or line-based parsing.
///
/// The ` length:` check is a format-level constraint: the length-prefixed
/// snapshot format uses ` length:` as the separator between path and byte
/// count in begin markers, so paths containing this substring create
/// ambiguity that neither greedy nor lazy regex matching can resolve.
fn validate_marker_safe(value: &str, label: &str) -> Result<(), SnapshotError> {
    if value.contains("-->")
        || value.contains('\n')
        || value.contains('\r')
        || value.contains('\0')
        || value.chars().any(|c| c.is_control())
    {
        return Err(SnapshotError::Format {
            reason: format!("{label} contains characters unsafe for snapshot markers: {value:?}"),
        });
    }
    if value.contains(" length:") {
        return Err(SnapshotError::Format {
            reason: format!(
                "{label} contains ' length:' which is ambiguous in the length-prefixed snapshot format: {value:?}"
            ),
        });
    }
    Ok(())
}

async fn load_state(path: &Path) -> Option<SnapshotState> {
    let data = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&data).ok()
}

/// Save cadence state using atomic write (temp + rename).
async fn save_state(path: &Path) {
    let state = SnapshotState {
        last_run: Utc::now(),
    };
    if let Some(dir) = path.parent()
        && let Err(e) = tokio::fs::create_dir_all(dir).await
    {
        tracing::warn!("snapshot: failed to create state dir: {e}");
        return;
    }
    let Ok(json) = serde_json::to_string_pretty(&state) else {
        return;
    };
    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = tokio::fs::write(&tmp_path, &json).await {
        tracing::warn!("snapshot: failed to write temp state: {e}");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = tokio::fs::set_permissions(&tmp_path, perms).await {
            tracing::warn!("snapshot: failed to set state file permissions: {e}");
        }
    }
    if let Err(e) = tokio::fs::rename(&tmp_path, path).await {
        tracing::warn!("snapshot: failed to rename state file: {e}");
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }
}

/// Render snapshot with length-prefixed sections.
fn render_snapshot(user_id: &str, documents: &[(String, String)]) -> String {
    let mut out = format!(
        "<!-- ironclaw-snapshot {} user_id={} created={} -->\n",
        SNAPSHOT_VERSION,
        user_id,
        Utc::now().to_rfc3339()
    );
    for (path, content) in documents {
        let byte_len = content.len();
        out.push_str(&format!("\n<!-- begin: {} length:{} -->\n", path, byte_len));
        out.push_str(content);
        out.push_str(&format!("\n<!-- end: {} -->\n", path));
    }
    out
}

/// Parse metadata from the first line of a snapshot file.
///
/// Expected format:
/// `<!-- ironclaw-snapshot v1 user_id=alice created=2026-03-15T12:00:00Z -->`
fn parse_metadata(line: &str) -> Result<(String, String, DateTime<Utc>), SnapshotError> {
    let inner = line
        .strip_prefix("<!-- ironclaw-snapshot ")
        .and_then(|s| s.strip_suffix(" -->"))
        .ok_or_else(|| SnapshotError::Format {
            reason: format!("invalid metadata line: {line}"),
        })?;

    let mut version = None;
    let mut user_id = None;
    let mut created = None;

    for token in inner.split_whitespace() {
        if let Some(val) = token.strip_prefix("user_id=") {
            user_id = Some(val.to_string());
        } else if let Some(val) = token.strip_prefix("created=") {
            created = Some(
                DateTime::parse_from_rfc3339(val)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| SnapshotError::Format {
                        reason: format!("invalid created timestamp: {e}"),
                    })?,
            );
        } else if version.is_none() {
            version = Some(token.to_string());
        }
    }

    Ok((
        version.ok_or_else(|| SnapshotError::Format {
            reason: "missing version".into(),
        })?,
        user_id.ok_or_else(|| SnapshotError::Format {
            reason: "missing user_id".into(),
        })?,
        created.ok_or_else(|| SnapshotError::Format {
            reason: "missing created timestamp".into(),
        })?,
    ))
}

/// Parse documents by byte-length framing.
///
/// Strict: all format errors produce `SnapshotError::Format`.
fn parse_snapshot_documents(text: &str) -> Result<Vec<(String, String)>, SnapshotError> {
    let mut results = Vec::new();
    let mut pos = 0;

    // Scan forward through text, using length to skip content regions so
    // that fake markers embedded inside document content are never matched.
    while pos < text.len() {
        let Some(cap) = BEGIN_RE.captures(&text[pos..]) else {
            // No more strict matches. Check for malformed begin markers in
            // remaining text — if any exist, the snapshot is damaged.
            if MALFORMED_BEGIN_RE.is_match(&text[pos..]) {
                return Err(SnapshotError::Format {
                    reason: "malformed begin marker with non-numeric length".into(),
                });
            }
            break;
        };
        let full_match = cap.get(0).ok_or_else(|| SnapshotError::Format {
            reason: "regex match without capture group 0".into(),
        })?;
        let path = cap[1].to_string();
        let length: usize = cap[2].parse().map_err(|e| SnapshotError::Format {
            reason: format!("invalid length for '{path}': {e}"),
        })?;

        // Validate path marker safety.
        validate_marker_safe(&path, "path")?;

        // Content starts after the newline following the begin marker.
        let marker_end_abs = pos + full_match.end();
        let content_start = marker_end_abs + 1; // +1 for the \n
        if content_start > text.len() {
            return Err(SnapshotError::Format {
                reason: format!("unexpected EOF after begin marker for '{path}'"),
            });
        }

        let content_end = content_start + length;
        if content_end > text.len() {
            return Err(SnapshotError::Format {
                reason: format!(
                    "content truncated for '{path}': expected {length} bytes, \
                     but only {} available",
                    text.len() - content_start
                ),
            });
        }

        // Verify char boundaries (defensive check for corrupted files).
        if !text.is_char_boundary(content_start) {
            return Err(SnapshotError::Format {
                reason: format!(
                    "content start offset {content_start} is not on a UTF-8 char boundary for '{path}'"
                ),
            });
        }
        if !text.is_char_boundary(content_end) {
            return Err(SnapshotError::Format {
                reason: format!(
                    "content end offset {content_end} is not on a UTF-8 char boundary for '{path}'"
                ),
            });
        }

        let content = &text[content_start..content_end];
        results.push((path, content.to_string()));

        // Advance past the content and the end marker line.
        pos = content_end;
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Format rendering / parsing ─────────────────────────────────

    #[test]
    fn render_and_parse_round_trip() {
        let docs = vec![
            (
                "MEMORY.md".to_string(),
                "# Memory\n\nSome notes.".to_string(),
            ),
            (
                "IDENTITY.md".to_string(),
                "# Identity\n\n- **Name:** Test".to_string(),
            ),
            (
                "context/vision.md".to_string(),
                "## Vision\n\nOur vision.".to_string(),
            ),
        ];

        let snapshot = render_snapshot("alice", &docs);
        let parsed = parse_snapshot_documents(&snapshot).expect("parse should succeed");

        assert_eq!(parsed.len(), 3);
        for (i, (path, content)) in parsed.iter().enumerate() {
            assert_eq!(path, &docs[i].0);
            assert_eq!(content, &docs[i].1, "content mismatch for {path}");
        }
    }

    #[test]
    fn round_trip_content_with_h2_headers() {
        let docs = vec![(
            "MEMORY.md".to_string(),
            "## Header inside content\n\n## Another header".to_string(),
        )];
        let snapshot = render_snapshot("test", &docs);
        let parsed = parse_snapshot_documents(&snapshot).expect("parse");
        assert_eq!(parsed[0].1, docs[0].1);
    }

    #[test]
    fn round_trip_content_with_fake_begin_end_markers() {
        let content = "<!-- begin: MEMORY.md length:99 -->\nfake content\n<!-- end: MEMORY.md -->"
            .to_string();
        let docs = vec![("IDENTITY.md".to_string(), content.clone())];
        let snapshot = render_snapshot("test", &docs);
        let parsed = parse_snapshot_documents(&snapshot).expect("parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "IDENTITY.md");
        assert_eq!(parsed[0].1, content);
    }

    #[test]
    fn round_trip_empty_document() {
        let docs = vec![("MEMORY.md".to_string(), String::new())];
        let snapshot = render_snapshot("test", &docs);
        let parsed = parse_snapshot_documents(&snapshot).expect("parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1, "");
    }

    #[test]
    fn round_trip_leading_trailing_newlines() {
        let content = "\n\nsome content\n\n".to_string();
        let docs = vec![("MEMORY.md".to_string(), content.clone())];
        let snapshot = render_snapshot("test", &docs);
        let parsed = parse_snapshot_documents(&snapshot).expect("parse");
        assert_eq!(
            parsed[0].1, content,
            "leading/trailing newlines must be preserved"
        );
    }

    #[test]
    fn round_trip_unicode_content() {
        let content = "中文内容 🎉 café résumé naïve 组合变音符: a\u{0300}\u{0301}".to_string();
        let docs = vec![("context/notes.md".to_string(), content.clone())];
        let snapshot = render_snapshot("test", &docs);
        let parsed = parse_snapshot_documents(&snapshot).expect("parse");
        assert_eq!(parsed[0].1, content);
    }

    #[test]
    fn round_trip_context_non_md_file() {
        let docs = vec![("context/notes.txt".to_string(), "plain text".to_string())];
        let snapshot = render_snapshot("test", &docs);
        let parsed = parse_snapshot_documents(&snapshot).expect("parse");
        assert_eq!(parsed[0].0, "context/notes.txt");
        assert_eq!(parsed[0].1, "plain text");
    }

    // ─── Metadata parsing ───────────────────────────────────────────

    #[test]
    fn parse_metadata_valid() {
        let line = "<!-- ironclaw-snapshot v1 user_id=alice created=2026-03-15T12:00:00+00:00 -->";
        let (version, user_id, _created) = parse_metadata(line).expect("parse");
        assert_eq!(version, "v1");
        assert_eq!(user_id, "alice");
    }

    #[test]
    fn parse_metadata_rejects_corrupted() {
        assert!(parse_metadata("not a valid header").is_err());
        assert!(parse_metadata("<!-- ironclaw-snapshot -->").is_err());
    }

    // ─── Parse strictness ───────────────────────────────────────────

    #[test]
    fn parse_rejects_non_numeric_length() {
        let text = "<!-- ironclaw-snapshot v1 user_id=a created=2026-01-01T00:00:00+00:00 -->\n\n<!-- begin: MEMORY.md length:abc -->\ncontent\n<!-- end: MEMORY.md -->\n";
        assert!(parse_snapshot_documents(text).is_err());
    }

    #[test]
    fn parse_rejects_truncated_content() {
        let text = "<!-- ironclaw-snapshot v1 user_id=a created=2026-01-01T00:00:00+00:00 -->\n\n<!-- begin: MEMORY.md length:9999 -->\nshort\n<!-- end: MEMORY.md -->\n";
        assert!(parse_snapshot_documents(text).is_err());
    }

    // ─── Marker safety ──────────────────────────────────────────────

    #[test]
    fn marker_rejects_path_with_arrow() {
        assert!(validate_marker_safe("path-->bad", "path").is_err());
    }

    #[test]
    fn marker_rejects_path_with_newline() {
        assert!(validate_marker_safe("path\nwith\nnewline", "path").is_err());
    }

    #[test]
    fn marker_allows_space_in_path() {
        // Spaces are valid in document paths (e.g. "context/project notes.md").
        assert!(validate_marker_safe("context/project notes.md", "path").is_ok());
    }

    #[test]
    fn marker_rejects_tab() {
        // Tabs are control chars and break both metadata and marker parsing.
        assert!(validate_marker_safe("alice\tbob", "user_id").is_err());
    }

    #[test]
    fn marker_rejects_control_chars() {
        assert!(validate_marker_safe("path\x00null", "path").is_err());
        assert!(validate_marker_safe("path\x01soh", "path").is_err());
    }

    #[test]
    fn marker_allows_normal_paths() {
        assert!(validate_marker_safe("MEMORY.md", "path").is_ok());
        assert!(validate_marker_safe("context/projects/deep/notes.txt", "path").is_ok());
        assert!(validate_marker_safe("context/中文.md", "path").is_ok());
    }

    // ─── Allowlist ──────────────────────────────────────────────────

    #[test]
    fn is_snapshot_path_allows_identity_docs() {
        assert!(is_snapshot_path("MEMORY.md"));
        assert!(is_snapshot_path("IDENTITY.md"));
        assert!(is_snapshot_path("SOUL.md"));
        assert!(is_snapshot_path("AGENTS.md"));
        assert!(is_snapshot_path("USER.md"));
        assert!(is_snapshot_path("HEARTBEAT.md"));
        assert!(is_snapshot_path("TOOLS.md"));
    }

    #[test]
    fn is_snapshot_path_allows_context() {
        assert!(is_snapshot_path("context/vision.md"));
        assert!(is_snapshot_path("context/projects/deep/notes.txt"));
    }

    #[test]
    fn is_snapshot_path_rejects_non_allowlist() {
        assert!(!is_snapshot_path("README.md"));
        assert!(!is_snapshot_path("BOOTSTRAP.md"));
        assert!(!is_snapshot_path("daily/2026-01-01.md"));
        assert!(!is_snapshot_path("conversations/chat.md"));
    }

    // ─── State persistence ──────────────────────────────────────────

    #[tokio::test]
    async fn save_and_load_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot_state.json");

        save_state(&path).await;
        let state = load_state(&path).await.expect("state should be loadable");
        let elapsed = Utc::now().signed_duration_since(state.last_run);
        assert!(elapsed.num_seconds() < 2);
    }

    #[tokio::test]
    async fn save_state_atomic_no_tmp_residue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let tmp = dir.path().join("state.json.tmp");

        save_state(&path).await;
        assert!(path.exists());
        assert!(!tmp.exists(), "temp file should be cleaned up");
    }

    #[tokio::test]
    async fn save_state_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("state.json");
        save_state(&path).await;
        assert!(path.exists());
    }

    // ─── Cadence / concurrency ──────────────────────────────────────

    #[test]
    fn running_guard_releases_on_drop() {
        SNAPSHOT_RUNNING.store(false, Ordering::SeqCst);
        {
            assert!(
                SNAPSHOT_RUNNING
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            );
            let _guard = SnapshotRunningGuard;
            assert!(
                SNAPSHOT_RUNNING
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
            );
        }
        // After guard dropped, should be acquirable again.
        assert!(
            SNAPSHOT_RUNNING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        );
        SNAPSHOT_RUNNING.store(false, Ordering::SeqCst);
    }

    // ─── Hydration behaviour (unit-level) ───────────────────────────

    #[test]
    fn hydration_rejects_wrong_version() {
        let text =
            "<!-- ironclaw-snapshot v99 user_id=test created=2026-01-01T00:00:00+00:00 -->\n";
        // We can only test parse_metadata + version check at unit level.
        let (version, _, _) = parse_metadata(text.lines().next().unwrap()).unwrap();
        assert_ne!(version, SNAPSHOT_VERSION);
    }

    // ─── I4: marker rejects ` length:` in paths ─────────────────────

    #[test]
    fn marker_rejects_path_with_length_keyword() {
        let result = validate_marker_safe("context/ length:42 notes.md", "path");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("length:"),
            "error should mention 'length:': {err}"
        );
    }

    // ─── I2: file permissions ────────────────────────────────────────

    #[cfg(unix)]
    #[tokio::test]
    async fn save_state_sets_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        save_state(&path).await;

        let metadata = std::fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "state file should have 0600 permissions");
    }
}
