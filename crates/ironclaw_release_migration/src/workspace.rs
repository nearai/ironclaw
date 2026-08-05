use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use ironclaw_host_api::ids::{TenantId, UserId};
use sha2::{Digest, Sha256};

use crate::ReleasePairMigrationError;

const MAX_WORKSPACE_ENTRIES: usize = 1_000_000;
const MAX_WORKSPACE_BYTES: u64 = 1_099_511_627_776;

/// Explicit operator-provided source for the rc1 shared workspace snapshot.
///
/// rc1 exposed the physical workspace root directly. The 1.1 serve surface
/// maps `/workspace` to a tenant/user subtree, so the old bytes must be copied
/// into that exact subtree before runtime writers start. The source is retained
/// unchanged as the rollback authority.
#[derive(Debug, Clone)]
pub struct LegacyWorkspaceMigrationInput {
    pub source: PathBuf,
    pub workspace_root: PathBuf,
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LegacyWorkspaceMigrationReport {
    pub directories_verified: usize,
    pub files_migrated: usize,
    pub files_unchanged: usize,
    pub bytes_verified: u64,
}

/// Copy an explicitly snapshotted rc1 workspace into the 1.1 per-caller
/// workspace, without overwriting any divergent destination file.
///
/// The operation is restart-safe: files are verified by SHA-256, new files are
/// published create-only from same-filesystem staging files, and the source is
/// never modified. Symlinks and other special files fail closed because their
/// ownership and escape semantics cannot be inferred safely during startup.
pub async fn migrate_legacy_workspace_snapshot(
    input: LegacyWorkspaceMigrationInput,
) -> Result<LegacyWorkspaceMigrationReport, ReleasePairMigrationError> {
    tokio::task::spawn_blocking(move || migrate_legacy_workspace_snapshot_blocking(&input))
        .await
        .map_err(|error| ReleasePairMigrationError::Domain {
            domain: "workspace artifacts",
            reason: format!("workspace migration worker failed: {error}"),
        })?
}

fn migrate_legacy_workspace_snapshot_blocking(
    input: &LegacyWorkspaceMigrationInput,
) -> Result<LegacyWorkspaceMigrationReport, ReleasePairMigrationError> {
    let domain_error = |reason: String| ReleasePairMigrationError::Domain {
        domain: "workspace artifacts",
        reason,
    };
    let source_metadata = std::fs::symlink_metadata(&input.source).map_err(|error| {
        domain_error(format!("legacy snapshot source is not readable: {error}"))
    })?;
    if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
        return Err(domain_error(
            "legacy snapshot source must be a real directory, not a symlink or special file"
                .to_string(),
        ));
    }
    let source = input.source.canonicalize().map_err(|error| {
        domain_error(format!("legacy snapshot source is not readable: {error}"))
    })?;
    std::fs::create_dir_all(&input.workspace_root)
        .map_err(|error| domain_error(format!("workspace root could not be created: {error}")))?;
    let workspace_root = input
        .workspace_root
        .canonicalize()
        .map_err(|error| domain_error(format!("workspace root is not accessible: {error}")))?;
    if source.starts_with(&workspace_root) || workspace_root.starts_with(&source) {
        return Err(domain_error(
            "legacy snapshot source and live workspace root must not overlap".to_string(),
        ));
    }
    let target_relative = PathBuf::from("tenants")
        .join(input.tenant_id.as_str())
        .join("users")
        .join(input.user_id.as_str());
    let target = ensure_relative_directory(&workspace_root, &target_relative)
        .map_err(|error| domain_error(format!("scoped workspace target is unsafe: {error}")))?;
    let staging =
        ensure_relative_directory(&workspace_root, Path::new(".ironclaw-migration-staging"))
            .map_err(|error| {
                domain_error(format!("workspace staging directory is unsafe: {error}"))
            })?;

    let mut report = LegacyWorkspaceMigrationReport::default();
    let mut entries_seen = 0_usize;
    let mut pending = vec![source.clone()];
    while let Some(directory) = pending.pop() {
        let relative = directory.strip_prefix(&source).map_err(|error| {
            domain_error(format!("workspace source path escaped its root: {error}"))
        })?;
        ensure_relative_directory(&target, relative)
            .map_err(|error| domain_error(format!("workspace directory is unsafe: {error}")))?;
        report.directories_verified = report.directories_verified.saturating_add(1);

        let read_dir = std::fs::read_dir(&directory).map_err(|error| {
            domain_error(format!("workspace directory could not be read: {error}"))
        })?;
        let mut entries = Vec::new();
        for entry in read_dir {
            entries_seen = entries_seen.saturating_add(1);
            if entries_seen > MAX_WORKSPACE_ENTRIES {
                return Err(domain_error(
                    "legacy workspace entry bound exceeded".to_string(),
                ));
            }
            entries.push(entry.map_err(|error| {
                domain_error(format!("workspace entry could not be read: {error}"))
            })?);
        }
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries.into_iter().rev() {
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
                domain_error(format!(
                    "workspace entry metadata could not be read: {error}"
                ))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(domain_error(
                    "legacy workspace contains a symlink; copy it to a regular in-root file or remove it from the snapshot before retrying".to_string(),
                ));
            }
            if metadata.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !metadata.is_file() {
                return Err(domain_error(
                    "legacy workspace contains an unsupported special file".to_string(),
                ));
            }

            let relative = entry
                .path()
                .strip_prefix(&source)
                .map_err(|error| {
                    domain_error(format!("workspace source path escaped its root: {error}"))
                })?
                .to_path_buf();
            let destination = target.join(relative);
            let (source_hash, source_bytes) = hash_file(&entry.path()).map_err(&domain_error)?;
            report.bytes_verified = report
                .bytes_verified
                .checked_add(source_bytes)
                .filter(|bytes| *bytes <= MAX_WORKSPACE_BYTES)
                .ok_or_else(|| domain_error("legacy workspace byte bound exceeded".to_string()))?;
            match std::fs::symlink_metadata(&destination) {
                Ok(destination_metadata) => {
                    if !destination_metadata.is_file() {
                        return Err(domain_error(
                            "legacy workspace destination conflicts with a non-file entry"
                                .to_string(),
                        ));
                    }
                    let (destination_hash, destination_bytes) =
                        hash_file(&destination).map_err(&domain_error)?;
                    if source_bytes != destination_bytes || source_hash != destination_hash {
                        return Err(domain_error(
                            "legacy workspace destination contains divergent content; no files were overwritten".to_string(),
                        ));
                    }
                    report.files_unchanged = report.files_unchanged.saturating_add(1);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    copy_file_create_only(
                        &entry.path(),
                        &destination,
                        &staging,
                        &source_hash,
                        source_bytes,
                    )
                    .map_err(&domain_error)?;
                    report.files_migrated = report.files_migrated.saturating_add(1);
                }
                Err(error) => {
                    return Err(domain_error(format!(
                        "workspace destination metadata could not be read: {error}"
                    )));
                }
            }
        }
    }
    let _ = std::fs::remove_dir(&staging);
    Ok(report)
}

fn hash_file(path: &Path) -> Result<([u8; 32], u64), String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("workspace file could not be opened: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("workspace file could not be read: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes = bytes.saturating_add(read as u64);
    }
    Ok((digest.finalize().into(), bytes))
}

fn ensure_relative_directory(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut directory = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err("workspace directory contains a non-normal path component".to_string());
        };
        directory.push(segment);
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("workspace destination directory is a symlink".to_string());
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err("workspace destination directory is not a directory".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&directory).map_err(|error| {
                    format!("workspace destination directory could not be created: {error}")
                })?;
                let metadata = std::fs::symlink_metadata(&directory).map_err(|error| {
                    format!("workspace destination directory could not be verified: {error}")
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(
                        "workspace destination directory changed during creation".to_string()
                    );
                }
            }
            Err(error) => {
                return Err(format!(
                    "workspace destination directory metadata could not be read: {error}"
                ));
            }
        }
    }
    Ok(directory)
}

fn copy_file_create_only(
    source: &Path,
    destination: &Path,
    staging: &Path,
    expected_hash: &[u8; 32],
    expected_bytes: u64,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "workspace destination has no parent directory".to_string())?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("workspace destination parent could not be read: {error}"))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err("workspace destination parent is unsafe".to_string());
    }
    let temporary = staging.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut source_file = std::fs::File::open(source)
            .map_err(|error| format!("workspace source file could not be opened: {error}"))?;
        let mut temporary_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("workspace staging file could not be created: {error}"))?;
        std::io::copy(&mut source_file, &mut temporary_file)
            .map_err(|error| format!("workspace staging copy failed: {error}"))?;
        temporary_file
            .flush()
            .map_err(|error| format!("workspace staging file could not be flushed: {error}"))?;
        temporary_file
            .sync_all()
            .map_err(|error| format!("workspace staging file could not be synced: {error}"))?;
        std::fs::set_permissions(
            &temporary,
            source_file
                .metadata()
                .map_err(|error| {
                    format!("workspace source permissions could not be read: {error}")
                })?
                .permissions(),
        )
        .map_err(|error| format!("workspace staging permissions could not be set: {error}"))?;
        let (actual_hash, actual_bytes) = hash_file(&temporary)?;
        if actual_bytes != expected_bytes || &actual_hash != expected_hash {
            return Err("workspace staging verification failed".to_string());
        }
        std::fs::hard_link(&temporary, destination).map_err(|error| {
            format!("workspace destination could not be published create-only: {error}")
        })?;
        let (published_hash, published_bytes) = hash_file(destination)?;
        if published_bytes != expected_bytes || &published_hash != expected_hash {
            return Err("workspace destination read-back verification failed".to_string());
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}
