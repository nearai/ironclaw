use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileWriteAction {
    Wrote,
    Preserved,
    Overwrote,
}

impl std::fmt::Display for FileWriteAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wrote => f.write_str("wrote"),
            Self::Preserved => f.write_str("preserved"),
            Self::Overwrote => f.write_str("overwrote"),
        }
    }
}

pub(crate) fn write_atomic(
    path: &Path,
    contents: &str,
    force: bool,
    label: &'static str,
) -> anyhow::Result<FileWriteAction> {
    if path.exists() && !force {
        anyhow::bail!(
            "{label} already exists at {}; pass --force to overwrite",
            path.display()
        );
    }
    let action = if path.exists() {
        FileWriteAction::Overwrote
    } else {
        FileWriteAction::Wrote
    };
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| anyhow::anyhow!("create temp file in {}: {error}", parent.display()))?;
    tmp.write_all(contents.as_bytes())
        .map_err(|error| anyhow::anyhow!("write {}: {error}", tmp.path().display()))?;
    tmp.flush()
        .map_err(|error| anyhow::anyhow!("flush {}: {error}", tmp.path().display()))?;

    if force {
        tmp.persist(path).map_err(|error| {
            anyhow::anyhow!(
                "persist {} -> {}: {}",
                error.file.path().display(),
                path.display(),
                error.error
            )
        })?;
    } else {
        tmp.persist_noclobber(path).map_err(|error| {
            anyhow::anyhow!(
                "persist {} -> {}: {}",
                error.file.path().display(),
                path.display(),
                error.error
            )
        })?;
    }
    Ok(action)
}
