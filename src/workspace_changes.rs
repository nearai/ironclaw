use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder, Header};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum WorkspaceChangesError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceFileSnapshot {
    pub digest: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceSnapshot {
    pub files: BTreeMap<String, WorkspaceFileSnapshot>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceChangeEntry {
    pub path: String,
    pub kind: WorkspaceChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceChangesSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    pub changes: Vec<WorkspaceChangeEntry>,
}

impl WorkspaceChangesSummary {
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceChangesPayload {
    pub summary: WorkspaceChangesSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_gzip_base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppliedWorkspaceChanges {
    pub applied_paths: Vec<String>,
    pub deleted_paths: Vec<String>,
}

pub fn capture_workspace_snapshot(root: &Path) -> Result<WorkspaceSnapshot, WorkspaceChangesError> {
    let mut files = BTreeMap::new();
    scan_workspace(root, root, &mut files)?;
    Ok(WorkspaceSnapshot { files })
}

fn scan_workspace(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<String, WorkspaceFileSnapshot>,
) -> Result<(), WorkspaceChangesError> {
    if !current.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            scan_workspace(root, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }

        let rel = normalize_relative_path(path.strip_prefix(root).map_err(|e| {
            WorkspaceChangesError::Message(format!(
                "failed to normalize workspace path {}: {e}",
                path.display()
            ))
        })?)?;
        let bytes = fs::read(&path)?;
        files.insert(
            rel,
            WorkspaceFileSnapshot {
                digest: sha256_bytes(&bytes),
                size_bytes: bytes.len() as u64,
            },
        );
    }

    Ok(())
}

pub fn build_workspace_changes_payload(
    root: &Path,
    baseline: &WorkspaceSnapshot,
    snapshot_generated_at: Option<String>,
    snapshot_source: Option<String>,
    project_dir: Option<String>,
) -> Result<Option<WorkspaceChangesPayload>, WorkspaceChangesError> {
    let current = capture_workspace_snapshot(root)?;
    let mut changes = Vec::new();
    let mut changed_file_paths = Vec::new();

    for (path, current_file) in &current.files {
        match baseline.files.get(path) {
            None => {
                changes.push(WorkspaceChangeEntry {
                    path: path.clone(),
                    kind: WorkspaceChangeKind::Added,
                    previous_digest: None,
                    current_digest: Some(current_file.digest.clone()),
                    size_bytes: Some(current_file.size_bytes),
                });
                changed_file_paths.push(path.clone());
            }
            Some(before) if before.digest != current_file.digest => {
                changes.push(WorkspaceChangeEntry {
                    path: path.clone(),
                    kind: WorkspaceChangeKind::Modified,
                    previous_digest: Some(before.digest.clone()),
                    current_digest: Some(current_file.digest.clone()),
                    size_bytes: Some(current_file.size_bytes),
                });
                changed_file_paths.push(path.clone());
            }
            Some(_) => {}
        }
    }

    for (path, before) in &baseline.files {
        if !current.files.contains_key(path) {
            changes.push(WorkspaceChangeEntry {
                path: path.clone(),
                kind: WorkspaceChangeKind::Deleted,
                previous_digest: Some(before.digest.clone()),
                current_digest: None,
                size_bytes: None,
            });
        }
    }

    if changes.is_empty() {
        return Ok(None);
    }

    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changed_file_paths.sort();

    let bundle_gzip_base64 = if changed_file_paths.is_empty() {
        None
    } else {
        let bundle = build_changed_files_bundle(root, &changed_file_paths)?;
        Some(base64::engine::general_purpose::STANDARD.encode(bundle))
    };

    Ok(Some(WorkspaceChangesPayload {
        summary: WorkspaceChangesSummary {
            snapshot_generated_at,
            snapshot_source,
            project_dir,
            changes,
        },
        bundle_gzip_base64,
    }))
}

pub fn build_workspace_changes_payload_from_archive(
    archive_gz: &[u8],
    baseline: &WorkspaceSnapshot,
    snapshot_generated_at: Option<String>,
    snapshot_source: Option<String>,
    project_dir: Option<String>,
) -> Result<Option<WorkspaceChangesPayload>, WorkspaceChangesError> {
    let temp = std::env::temp_dir().join(format!("ironclaw-workspace-changes-{}", Uuid::new_v4()));
    fs::create_dir_all(&temp)?;
    let result = (|| {
        unpack_workspace_archive(&temp, archive_gz)?;
        build_workspace_changes_payload(
            &temp,
            baseline,
            snapshot_generated_at,
            snapshot_source,
            project_dir,
        )
    })();
    let cleanup = fs::remove_dir_all(&temp);
    match (result, cleanup) {
        (Ok(payload), Ok(())) => Ok(payload),
        (Ok(_), Err(err)) => Err(err.into()),
        (Err(err), _) => Err(err),
    }
}

fn build_changed_files_bundle(
    root: &Path,
    changed_file_paths: &[String],
) -> Result<Vec<u8>, WorkspaceChangesError> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);

    for rel_path in changed_file_paths {
        let abs_path = root.join(rel_path);
        let bytes = fs::read(&abs_path)?;
        let mut header = Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, rel_path, Cursor::new(bytes))?;
    }

    let encoder = builder.into_inner()?;
    Ok(encoder.finish()?)
}

pub fn decode_bundle_entries(
    bundle_gzip_base64: &str,
) -> Result<HashMap<String, Vec<u8>>, WorkspaceChangesError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bundle_gzip_base64)
        .map_err(|e| WorkspaceChangesError::Message(format!("invalid base64 bundle: {e}")))?;
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    let mut entries = HashMap::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?;
        let rel = normalize_relative_path(path.as_ref())?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        entries.insert(rel, buf);
    }

    Ok(entries)
}

pub fn unpack_workspace_archive(
    root: &Path,
    archive_gz: &[u8],
) -> Result<(), WorkspaceChangesError> {
    let decoder = GzDecoder::new(Cursor::new(archive_gz));
    let mut archive = Archive::new(decoder);

    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }

        let path = entry.path()?;
        let rel = normalize_relative_path(path.as_ref())?;
        let target = safe_target_path(root, &rel)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(target)?;
        std::io::copy(&mut entry, &mut file)?;
        file.flush()?;
    }

    Ok(())
}

pub fn apply_workspace_changes(
    root: &Path,
    payload: &WorkspaceChangesPayload,
) -> Result<AppliedWorkspaceChanges, WorkspaceChangesError> {
    let bundle_entries = match payload.bundle_gzip_base64.as_deref() {
        Some(bundle) => decode_bundle_entries(bundle)?,
        None => HashMap::new(),
    };

    for change in &payload.summary.changes {
        let target = safe_target_path(root, &change.path)?;
        match change.kind {
            WorkspaceChangeKind::Added => {
                if target.exists() {
                    return Err(WorkspaceChangesError::Message(format!(
                        "cannot apply returned change for {} because the file now exists on the host",
                        change.path
                    )));
                }
                if !bundle_entries.contains_key(&change.path) {
                    return Err(WorkspaceChangesError::Message(format!(
                        "bundle is missing added file {}",
                        change.path
                    )));
                }
            }
            WorkspaceChangeKind::Modified => {
                let expected = change.previous_digest.as_deref().ok_or_else(|| {
                    WorkspaceChangesError::Message(format!(
                        "modified file {} is missing previous_digest",
                        change.path
                    ))
                })?;
                let current = digest_path_if_exists(&target)?;
                if current.as_deref() != Some(expected) {
                    return Err(WorkspaceChangesError::Message(format!(
                        "cannot apply returned change for {} because the host file changed since the snapshot",
                        change.path
                    )));
                }
                if !bundle_entries.contains_key(&change.path) {
                    return Err(WorkspaceChangesError::Message(format!(
                        "bundle is missing modified file {}",
                        change.path
                    )));
                }
            }
            WorkspaceChangeKind::Deleted => {
                let expected = change.previous_digest.as_deref().ok_or_else(|| {
                    WorkspaceChangesError::Message(format!(
                        "deleted file {} is missing previous_digest",
                        change.path
                    ))
                })?;
                let current = digest_path_if_exists(&target)?;
                if current.as_deref() != Some(expected) {
                    return Err(WorkspaceChangesError::Message(format!(
                        "cannot delete {} because the host file changed since the snapshot",
                        change.path
                    )));
                }
            }
        }
    }

    let mut applied_paths = Vec::new();
    let mut deleted_paths = Vec::new();

    for change in &payload.summary.changes {
        let target = safe_target_path(root, &change.path)?;
        match change.kind {
            WorkspaceChangeKind::Added | WorkspaceChangeKind::Modified => {
                let bytes = bundle_entries.get(&change.path).ok_or_else(|| {
                    WorkspaceChangesError::Message(format!(
                        "bundle is missing file contents for {}",
                        change.path
                    ))
                })?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&target, bytes)?;
                applied_paths.push(change.path.clone());
            }
            WorkspaceChangeKind::Deleted => {
                fs::remove_file(&target)?;
                deleted_paths.push(change.path.clone());
            }
        }
    }

    Ok(AppliedWorkspaceChanges {
        applied_paths,
        deleted_paths,
    })
}

fn digest_path_if_exists(path: &Path) -> Result<Option<String>, WorkspaceChangesError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok(Some(sha256_bytes(&bytes)))
}

fn safe_target_path(root: &Path, rel: &str) -> Result<PathBuf, WorkspaceChangesError> {
    let rel_path = Path::new(rel);
    normalize_relative_path(rel_path)?;
    Ok(root.join(rel_path))
}

fn normalize_relative_path(path: &Path) -> Result<String, WorkspaceChangesError> {
    if path.is_absolute() {
        return Err(WorkspaceChangesError::Message(format!(
            "absolute paths are not allowed: {}",
            path.display()
        )));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(WorkspaceChangesError::Message(format!(
                    "parent traversal is not allowed: {}",
                    path.display()
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceChangesError::Message(format!(
                    "absolute paths are not allowed: {}",
                    path.display()
                )));
            }
        }
    }

    let text = normalized.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        return Err(WorkspaceChangesError::Message(
            "empty relative path is not allowed".to_string(),
        ));
    }
    Ok(text)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_apply_workspace_changes_roundtrip() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let root = temp.path();
        fs::create_dir_all(root.join("src")).expect("src dir should exist");
        fs::write(root.join("src").join("lib.rs"), "hello").expect("baseline file should write");
        fs::write(root.join("README.md"), "before").expect("baseline readme should write");

        let baseline = capture_workspace_snapshot(root).expect("baseline snapshot should work");

        fs::write(root.join("src").join("lib.rs"), "hello world")
            .expect("modified file should write");
        fs::write(root.join("src").join("new.rs"), "new").expect("new file should write");
        fs::remove_file(root.join("README.md")).expect("readme should delete");

        let payload = build_workspace_changes_payload(
            root,
            &baseline,
            Some("2026-04-15T00:00:00Z".to_string()),
            Some("project-dir".to_string()),
            Some(root.display().to_string()),
        )
        .expect("payload should build")
        .expect("payload should exist");

        let target = tempfile::tempdir().expect("target tempdir should exist");
        fs::create_dir_all(target.path().join("src")).expect("target src dir should exist");
        fs::write(target.path().join("src").join("lib.rs"), "hello")
            .expect("target baseline file should write");
        fs::write(target.path().join("README.md"), "before").expect("target readme should write");

        let applied =
            apply_workspace_changes(target.path(), &payload).expect("apply should succeed");
        assert_eq!(applied.applied_paths.len(), 2);
        assert_eq!(applied.deleted_paths, vec!["README.md".to_string()]);
        assert_eq!(
            fs::read_to_string(target.path().join("src").join("lib.rs"))
                .expect("modified file should exist"),
            "hello world"
        );
        assert_eq!(
            fs::read_to_string(target.path().join("src").join("new.rs"))
                .expect("new file should exist"),
            "new"
        );
        assert!(!target.path().join("README.md").exists());
    }

    #[test]
    fn apply_rejects_conflicting_host_file() {
        let source = tempfile::tempdir().expect("source tempdir should exist");
        fs::write(source.path().join("a.txt"), "before").expect("source file should write");
        let baseline =
            capture_workspace_snapshot(source.path()).expect("source baseline should work");
        fs::write(source.path().join("a.txt"), "after").expect("source file should update");
        let payload = build_workspace_changes_payload(source.path(), &baseline, None, None, None)
            .expect("payload should build")
            .expect("payload should exist");

        let target = tempfile::tempdir().expect("target tempdir should exist");
        fs::write(target.path().join("a.txt"), "different").expect("target file should write");

        let err =
            apply_workspace_changes(target.path(), &payload).expect_err("apply should reject");
        assert!(err.to_string().contains("host file changed"));
    }
}
