use super::*;

pub(in super::super) fn write_manifest_last(
    home: &Path,
    manifest: &LayoutManifest,
) -> anyhow::Result<()> {
    let manifest_path = home.join(LAYOUT_MANIFEST_FILE);
    if manifest_path.exists() {
        let existing = read_manifest(&manifest_path)?;
        if existing == *manifest {
            return Ok(());
        }
        bail!(
            "refusing to replace existing layout manifest at {}",
            manifest_path.display()
        );
    }
    let contents = toml::to_string(manifest).context("serialize durable layout manifest")?;
    match write_atomic_synced(&manifest_path, &contents, false) {
        Ok(()) => Ok(()),
        Err(create_error) => match read_manifest(&manifest_path) {
            Ok(existing) if existing == *manifest => Ok(()),
            _ => Err(create_error),
        },
    }
}

pub(in super::super) fn read_manifest(path: &Path) -> anyhow::Result<LayoutManifest> {
    let contents = read_utf8_file_no_follow(path)?;
    toml::from_str(&contents)
        .map_err(|error| anyhow!("parse durable layout manifest {}: {error}", path.display()))
}

pub(in super::super) fn admit_manifest(
    manifest: &LayoutManifest,
    requirement: LayoutRequirement,
) -> anyhow::Result<()> {
    match manifest.admit(requirement) {
        ProfileTransitionAdmission::Allowed => Ok(()),
        ProfileTransitionAdmission::Rejected { reason } => {
            bail!("stored durable layout rejects this profile transition: {reason}")
        }
    }
}

pub(in super::super) fn write_atomic_synced(
    path: &Path,
    contents: &str,
    replace: bool,
) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    require_ordinary_directory(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary file beside {}", path.display()))?;
    temp.write_all(contents.as_bytes())
        .with_context(|| format!("write temporary file for {}", path.display()))?;
    temp.as_file()
        .sync_all()
        .with_context(|| format!("sync temporary file for {}", path.display()))?;
    if replace {
        temp.persist(path).map_err(|error| {
            anyhow!(
                "atomically replace {} with {}: {}",
                path.display(),
                error.file.path().display(),
                error.error
            )
        })?;
    } else {
        temp.persist_noclobber(path).map_err(|error| {
            anyhow!(
                "atomically create {} from {}: {}",
                path.display(),
                error.file.path().display(),
                error.error
            )
        })?;
    }
    sync_directory(parent)
}
