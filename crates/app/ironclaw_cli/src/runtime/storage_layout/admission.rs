use super::*;
use super::{filesystem::*, model::*, mover::*};

/// Validate a ready layout, initialize a genuinely fresh home, or classify the
/// populated legacy sources for boot-time migration.
///
/// This never performs migration work: it does not select a source, move a
/// file, or write the provenance record.
pub(crate) fn admit_startup_layout(
    home: &RebornHome,
    requirement: LayoutRequirement,
) -> anyhow::Result<StartupLayoutAdmission> {
    let home_path = home.path();
    let paths = RebornStoragePaths::from_home(home);
    let manifest_path = home_path.join(LAYOUT_MANIFEST_FILE);
    if manifest_path.exists() {
        admit_existing_manifest(&manifest_path, &paths, requirement)?;
        return Ok(StartupLayoutAdmission::Ready(paths));
    }

    let record_path = migration_record_path(&paths);
    if record_path.exists() {
        let record = read_migration_record(&record_path)?;
        if record.phase != MigrationPhase::Complete {
            // Source renames may be incomplete. Recovery is the operator's
            // backup, never a reconstruction from ambient filesystem state.
            bail!(
                "layout migration record exists at {} but no layout manifest was published; the previous migration was interrupted. Restore the pre-migration backup of this home, then restart",
                record_path.display()
            );
        }
        admit_manifest(&record.target_manifest, requirement)?;
        validate_ready_namespace_roots(&paths)?;
        write_manifest_last(home_path, &record.target_manifest)?;
        return Ok(StartupLayoutAdmission::Ready(paths));
    }

    let candidates = inspect_legacy_candidates(home_path)?;
    if candidates.is_empty() && canonical_layout_is_empty(&paths)? {
        initialize_fresh_layout(home_path, &paths, requirement)?;
        return Ok(StartupLayoutAdmission::Ready(paths));
    }
    if !candidates.is_empty() {
        return Ok(StartupLayoutAdmission::MigrationRequired(candidates));
    }

    bail!(
        "canonical durable layout is incomplete or unrecognized at {}; refusing to open stores without a valid layout.toml. Inspect the home contents manually",
        home_path.display()
    )
}

/// Validate a ready layout or initialize a genuinely fresh home for stateful
/// CLI commands outside runtime startup. Commands never migrate: a legacy
/// home is reported with the startup remedy instead.
pub(crate) fn ensure_ready_layout(
    home: &RebornHome,
    requirement: LayoutRequirement,
) -> anyhow::Result<RebornStoragePaths> {
    match admit_startup_layout(home, requirement)? {
        StartupLayoutAdmission::Ready(paths) => Ok(paths),
        StartupLayoutAdmission::MigrationRequired(_) => bail!(
            "legacy durable storage detected; start the Reborn runtime (`ironclaw serve`) once to migrate it into the profile-stable layout, then rerun this command"
        ),
    }
}

/// Validate a ready canonical layout without creating any directories,
/// records, or manifests. This is the migration-dry-run admission path: it
/// may report an unsafe deployment, but it must not change it.
pub(crate) fn inspect_ready_layout(
    home: &RebornHome,
    requirement: LayoutRequirement,
) -> anyhow::Result<RebornStoragePaths> {
    let paths = RebornStoragePaths::from_home(home);
    let manifest_path = home.path().join(LAYOUT_MANIFEST_FILE);
    if !manifest_path.exists() {
        bail!(
            "canonical durable layout is not ready at {}; migration dry-run will not initialize it",
            home.path().display()
        );
    }
    admit_existing_manifest(&manifest_path, &paths, requirement)?;
    Ok(paths)
}

/// Admit a published layout only when its migration record (if one exists)
/// completed and every canonical namespace remains safe to open.
fn admit_existing_manifest(
    manifest_path: &Path,
    paths: &RebornStoragePaths,
    requirement: LayoutRequirement,
) -> anyhow::Result<()> {
    let manifest = read_manifest(manifest_path)?;
    let record_path = migration_record_path(paths);
    if record_path.exists() {
        let record = read_migration_record(&record_path)?;
        if record.phase != MigrationPhase::Complete || record.target_manifest != manifest {
            bail!(
                "ready layout manifest and migration record disagree at {}; refusing to open durable state",
                record_path.display()
            );
        }
    }
    admit_manifest(&manifest, requirement)?;
    validate_ready_namespace_roots(paths)
}

fn validate_ready_namespace_roots(paths: &RebornStoragePaths) -> anyhow::Result<()> {
    for namespace in paths.canonical_namespace_roots() {
        require_ordinary_directory(namespace)?;
    }
    Ok(())
}

/// Return the fixed legacy skill staging source retained by a completed
/// migration. Composition receives this enum, never a caller-selected host
/// path, and derives the staging location itself.
pub(crate) fn ready_legacy_skill_snapshot_source(
    home: &RebornHome,
) -> anyhow::Result<Option<LegacyStorageSource>> {
    let paths = RebornStoragePaths::from_home(home);
    let mut found = None;
    for kind in [
        LegacySourceKind::LocalDev,
        LegacySourceKind::HostedSingleTenant,
        LegacySourceKind::HostedSingleTenantVolume,
        LegacySourceKind::BareHome,
    ] {
        let staging_root = kind.snapshot_root(&paths);
        if !staging_root.exists() {
            continue;
        }
        require_ordinary_directory(&staging_root)?;
        if !directory_has_content(&staging_root)? {
            continue;
        }
        if found.is_some() {
            bail!(
                "multiple retained legacy skill staging roots exist under {}; refusing to select one",
                paths.runtime_root().display()
            );
        }
        found = Some(kind);
    }
    Ok(found)
}

/// Read the durable external-memory namespace committed by fresh
/// initialization or legacy migration.
pub(crate) fn ready_memory_provider_app_id(home: &RebornHome) -> anyhow::Result<Option<String>> {
    let manifest = read_manifest(&home.path().join(LAYOUT_MANIFEST_FILE))?;
    Ok(manifest.memory_provider_app_id().map(str::to_owned))
}

pub(super) fn initialize_fresh_layout(
    home: &Path,
    paths: &RebornStoragePaths,
    requirement: LayoutRequirement,
) -> anyhow::Result<()> {
    fs::create_dir_all(home).with_context(|| format!("create Reborn home {}", home.display()))?;
    for path in paths.canonical_namespace_roots() {
        create_or_validate_direct_child(home, path)?;
        sync_directory(path)?;
    }
    let manifest = LayoutManifest::new(requirement).with_memory_provider_app_id(
        ironclaw_config::canonical_memory_provider_app_id(paths.installation_root()),
    );
    write_manifest_last(home, &manifest)
}
