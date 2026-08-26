use super::*;
use ironclaw_filesystem::{CasExpectation, DiskDirectoryCapability};

pub(in super::super) fn write_manifest_last(
    home: &Path,
    manifest: &LayoutManifest,
) -> anyhow::Result<()> {
    let manifest_path = home.join(LAYOUT_MANIFEST_FILE);
    if manifest_path.exists() {
        return converge_concurrent_manifest(&manifest_path, manifest);
    }
    let contents = toml::to_string(manifest).context("serialize durable layout manifest")?;
    match write_atomic_synced(&manifest_path, &contents, false) {
        Ok(()) => Ok(()),
        Err(create_error) => converge_concurrent_manifest(&manifest_path, manifest)
            .with_context(|| format!("initial manifest publication failed: {create_error:#}")),
    }
}

fn converge_concurrent_manifest(path: &Path, target: &LayoutManifest) -> anyhow::Result<()> {
    let existing = read_manifest(path)?;
    if existing == *target {
        return Ok(());
    }
    admit_manifest(&existing, target.requirement())?;
    let strengthened = existing
        .clone()
        .with_stronger_workspace_access_floor(target.requirement().security.workspace_access_floor);
    if strengthened != *target {
        bail!(
            "refusing to replace existing layout manifest at {}",
            path.display()
        );
    }
    replace_manifest(path, &strengthened)?;
    let persisted = read_manifest(path)?;
    if persisted != strengthened {
        bail!(
            "concurrent layout manifest update at {} did not preserve the strongest admitted workspace floor",
            path.display()
        );
    }
    Ok(())
}

pub(in super::super) fn admit_and_upgrade_manifest(
    path: &Path,
    requirement: LayoutRequirement,
) -> anyhow::Result<LayoutManifest> {
    let existing = read_manifest(path)?;
    admit_manifest(&existing, requirement)?;
    let strengthened = existing
        .clone()
        .with_stronger_workspace_access_floor(requirement.security.workspace_access_floor);
    if strengthened == existing {
        return Ok(existing);
    }
    // WorkspaceAccessFloor has one strengthening edge. Every writer reaching
    // this branch therefore publishes the same stronger value, so atomic
    // replacement cannot lose a concurrent update or reintroduce the weak
    // floor; a racing weak admission never writes.
    replace_manifest(path, &strengthened)?;
    let persisted = read_manifest(path)?;
    admit_manifest(&persisted, requirement)?;
    Ok(persisted)
}

fn replace_manifest(path: &Path, manifest: &LayoutManifest) -> anyhow::Result<()> {
    let contents = toml::to_string(manifest).context("serialize durable layout manifest")?;
    write_atomic_synced(path, &contents, true)
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
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", path.display()))?;
    let capability = DiskDirectoryCapability::admit_existing(parent)
        .with_context(|| format!("admit parent directory for {}", path.display()))?;
    write_atomic_synced_at(&capability, Path::new(file_name), path, contents, replace)
}

pub(in super::super) fn write_atomic_synced_at(
    parent: &DiskDirectoryCapability,
    relative: &Path,
    display_path: &Path,
    contents: &str,
    replace: bool,
) -> anyhow::Result<()> {
    let cas = if replace {
        CasExpectation::Any
    } else {
        CasExpectation::Absent
    };
    parent
        .write_file_atomic_synced(relative, contents.as_bytes(), cas)
        .with_context(|| format!("atomically publish {}", display_path.display()))
}
