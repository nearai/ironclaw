use super::*;

pub(in super::super) fn canonical_layout_is_empty(
    paths: &RebornStoragePaths,
) -> anyhow::Result<bool> {
    for path in paths.canonical_namespace_roots() {
        if path.exists() && !directory_is_empty(path)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(in super::super) fn create_or_validate_direct_child(
    parent: &Path,
    child: &Path,
) -> anyhow::Result<()> {
    if child.parent() != Some(parent) {
        bail!(
            "refusing to create non-direct child {} beneath {}",
            child.display(),
            parent.display()
        );
    }
    require_ordinary_directory(parent)?;
    if child.exists() {
        return require_ordinary_directory(child);
    }
    fs::create_dir(child)
        .with_context(|| format!("create canonical directory {}", child.display()))?;
    sync_directory(parent)
}
