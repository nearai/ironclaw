//! One-shot boot-time migration of a supported legacy home into the
//! profile-stable canonical layout.
//!
//! The migration is rename-based: nothing is copied and nothing is deleted.
//! The chosen source's files move to their canonical locations on the same
//! volume; every losing candidate stays byte-for-byte in place and is named
//! in the retained `runtime/layout-migration.toml` provenance record. The
//! manifest (`layout.toml`) is published last, so an interrupted migration
//! leaves a home that refuses startup instead of half-opening stores.

use super::*;
use super::{filesystem::*, locks::*, model::*};

/// Automatically migrate one supported legacy source into the canonical
/// layout, guarded by an advisory home lock and a live-writer probe on the
/// embedded database. Fails closed on ambiguity; never merges candidates.
pub(crate) fn migrate_legacy_layout(
    home: &RebornHome,
    requirement: LayoutRequirement,
    policy: StorageMigrationPolicy,
    candidates: Vec<LegacyCandidate>,
) -> anyhow::Result<()> {
    if policy == StorageMigrationPolicy::Manual {
        bail!(
            "legacy durable storage detected but {}={} defers boot-time migration; restart without the override (or with {}={}) after scheduling the migration window",
            StorageMigrationPolicy::ENV,
            StorageMigrationPolicy::MANUAL,
            StorageMigrationPolicy::ENV,
            StorageMigrationPolicy::AUTOMATIC
        );
    }
    let home_path = home.path();
    let paths = RebornStoragePaths::from_home(home);

    let (winner, ignored) = select_migration_source(candidates)?;
    let target_manifest = LayoutManifest::new(requirement).with_memory_provider_app_id(
        ironclaw_config::legacy_memory_provider_app_id(&winner.source_root),
    );
    admit_manifest(&LayoutManifest::new(winner.kind.requirement()), requirement)?;

    let _lock = acquire_named_lock(home_path, MIGRATION_LOCK_FILE, "storage layout migration")?;
    // Classification happened before the lock; re-read every source invariant
    // under it so a competing replica cannot race the selection.
    let relisted = inspect_legacy_candidates(home_path)?;
    let (relisted_winner, _) = select_migration_source(relisted)?;
    if relisted_winner != winner {
        bail!("legacy storage sources changed while acquiring the migration lock; retry startup");
    }
    if winner.is_embedded() {
        ensure_legacy_db_not_in_use(&winner)?;
    }
    if !canonical_layout_is_empty(&paths)? {
        bail!(
            "canonical namespaces under {} already contain data; migration never overwrites or merges canonical state",
            home_path.display()
        );
    }

    for path in paths.canonical_namespace_roots() {
        create_or_validate_direct_child(home_path, path)?;
        sync_directory(path)?;
    }

    let mut record = MigrationRecord {
        schema_version: MIGRATION_RECORD_SCHEMA_VERSION,
        phase: MigrationPhase::InProgress,
        source: winner.kind,
        source_root: winner.source_root.clone(),
        target_manifest: target_manifest.clone(),
        has_legacy_skills: winner.has_legacy_skills,
        ignored,
    };
    write_migration_record(&paths, &record, false)?;

    move_winner_into_canonical_layout(&winner, &paths)?;

    record.phase = MigrationPhase::Complete;
    write_migration_record(&paths, &record, true)?;

    write_manifest_last(home_path, &target_manifest)?;

    tracing::info!(
        source = winner.kind.label(),
        home = %home_path.display(),
        "migrated legacy durable storage into the profile-stable layout; older ironclaw binaries can no longer read this home"
    );
    for ignored in &record.ignored {
        tracing::info!(
            source = ignored.source.label(),
            path = %ignored.source_root.display(),
            "ignored an older populated legacy source; its data was left in place untouched"
        );
    }
    Ok(())
}

/// Rank populated candidates by most recent use and pick the winner. Losers
/// are reported, never touched. An unorderable tie between the two candidates
/// that actually compete fails closed instead of guessing.
///
/// A bare-home candidate is the byproduct of a fixed historical resolver bug
/// rather than any released layout, so it ranks strictly below every real
/// profile directory regardless of timestamps. Recency orders within each
/// class. Keeping the classes partitioned — instead of demoting a bare-home
/// entry only when it happened to tie — is what lets the tie guard run against
/// the real neighbours: an earlier version swapped the top two on a tie and
/// never re-checked, so a newer bare-home artifact beside two genuinely
/// unorderable profile directories silently selected one of them.
pub(super) fn select_migration_source(
    candidates: Vec<LegacyCandidate>,
) -> anyhow::Result<(LegacyCandidate, Vec<IgnoredCandidate>)> {
    const TIE_WINDOW_SECS: u64 = 2;
    if candidates.is_empty() {
        bail!("no supported populated legacy source found to migrate");
    }
    let mut ranked = candidates
        .into_iter()
        .map(|candidate| {
            let last_used = candidate_last_used(&candidate)?;
            Ok((candidate, last_used))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    // Real profile directories first, then bare-home artifacts; most recent
    // first within each class.
    ranked.sort_by_key(|(candidate, last_used)| {
        (
            candidate.kind == LegacySourceKind::BareHome,
            std::cmp::Reverse(*last_used),
        )
    });
    // The winner competes only with the next candidate of its own class, so a
    // bare-home artifact never suppresses a tie between two profile roots.
    let competing_neighbours = ranked.len() > 1
        && (ranked[0].0.kind == LegacySourceKind::BareHome)
            == (ranked[1].0.kind == LegacySourceKind::BareHome);
    if competing_neighbours {
        let gap = ranked[0].1.duration_since(ranked[1].1).unwrap_or_default();
        if gap.as_secs() < TIE_WINDOW_SECS {
            bail!(
                "multiple populated legacy sources were last used at nearly the same time; recency cannot pick one safely. Inspect and archive all but one of: {}",
                candidate_paths(&ranked.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>())
            );
        }
    }
    let mut ranked = ranked.into_iter();
    let (winner, _) = ranked
        .next()
        .ok_or_else(|| anyhow!("candidate ranking cannot be empty"))?;
    let ignored = ranked
        .map(|(candidate, last_used)| IgnoredCandidate {
            source: candidate.kind,
            source_root: candidate.source_root,
            last_used_epoch_secs: last_used
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
        .collect();
    Ok((winner, ignored))
}

/// Most recent write signal for a candidate: the newest mtime across its
/// database unit and master key, falling back to the source directory itself
/// for sources (hosted PostgreSQL) that keep no embedded files.
pub(super) fn candidate_last_used(
    candidate: &LegacyCandidate,
) -> anyhow::Result<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    let mut observe = |path: &Path| -> anyhow::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                let modified = metadata
                    .modified()
                    .with_context(|| format!("read mtime of {}", path.display()))?;
                if newest.is_none_or(|current| modified > current) {
                    newest = Some(modified);
                }
                Ok(())
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("inspect {} for recency", path.display()))
            }
        }
    };
    for file in &candidate.db_files {
        observe(&candidate.source_root.join(file))?;
    }
    if candidate.has_master_key {
        observe(&candidate.source_root.join(MASTER_KEY_FILE))?;
    }
    match newest {
        Some(newest) => Ok(newest),
        None => fs::symlink_metadata(&candidate.source_root)
            .with_context(|| {
                format!(
                    "inspect legacy source {} for recency",
                    candidate.source_root.display()
                )
            })?
            .modified()
            .with_context(|| {
                format!(
                    "read mtime of legacy source {}",
                    candidate.source_root.display()
                )
            }),
    }
}

/// Refuse migration while another process still has the legacy embedded
/// database open. SQLite/libSQL coordinates through POSIX record locks, so a
/// conflicting-lock probe on the database's lock byte range and the WAL
/// index's dead-man-switch range detects live readers, writers, and idle open
/// WAL connections without acquiring anything.
fn ensure_legacy_db_not_in_use(candidate: &LegacyCandidate) -> anyhow::Result<()> {
    let db_path = candidate.source_root.join(DB_FILE);
    if legacy_db_probe(&db_path, 0x4000_0000, 512)? {
        bail!(
            "another ironclaw process is using the legacy database at {}; stop it, then restart to migrate",
            db_path.display()
        );
    }
    let shm_path = candidate.source_root.join(format!("{DB_FILE}-shm"));
    if legacy_db_probe(&shm_path, 120, 8)? {
        bail!(
            "another ironclaw process holds the legacy database WAL index at {}; stop it, then restart to migrate",
            shm_path.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn legacy_db_probe(path: &Path, start: i64, len: i64) -> anyhow::Result<bool> {
    use std::os::unix::io::AsRawFd as _;

    if !path.exists() {
        return Ok(false);
    }
    let file = open_file_no_follow(path)?;
    // SAFETY: `flock` is a plain-old-data struct; zero is a valid initial
    // state for every field before the probe parameters are assigned.
    let mut probe: libc::flock = unsafe { std::mem::zeroed() };
    probe.l_type = libc::F_WRLCK as libc::c_short;
    probe.l_whence = libc::SEEK_SET as libc::c_short;
    probe.l_start = start;
    probe.l_len = len;
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut probe) };
    if rc == -1 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("probe advisory locks on {}", path.display()));
    }
    Ok(probe.l_type != libc::F_UNLCK as libc::c_short)
}

#[cfg(not(unix))]
fn legacy_db_probe(_path: &Path, _start: i64, _len: i64) -> anyhow::Result<bool> {
    // Non-unix builds fall back to the advisory migration lock alone.
    Ok(false)
}

fn move_winner_into_canonical_layout(
    winner: &LegacyCandidate,
    paths: &RebornStoragePaths,
) -> anyhow::Result<()> {
    for file in &winner.db_files {
        rename_into(
            &winner.source_root.join(file),
            &paths.state_root().join(file),
        )?;
    }
    if winner.has_master_key {
        rename_into(
            &winner.source_root.join(MASTER_KEY_FILE),
            &paths.state_root().join(MASTER_KEY_FILE),
        )?;
    }
    if winner.has_system_content {
        let system_source = winner.source_root.join("system");
        for directory in SYSTEM_CONTENT_DIRS {
            let source = system_source.join(directory);
            if source.exists() {
                rename_into(&source, &paths.system_root().join(directory))?;
            }
        }
    }
    if winner.has_legacy_skills {
        let staging_root = winner.kind.snapshot_root(paths);
        fs::create_dir_all(&staging_root).with_context(|| {
            format!(
                "create legacy skill staging root {}",
                staging_root.display()
            )
        })?;
        for tree in ["skills", "tenants"] {
            let source = winner.source_root.join(tree);
            if source.exists() {
                rename_into(&source, &staging_root.join(tree))?;
            }
        }
        sync_directory(&staging_root)?;
    }
    sync_directory(paths.state_root())?;
    sync_directory(paths.system_root())?;
    sync_directory(&winner.source_root)?;
    Ok(())
}

fn rename_into(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if destination.exists() {
        bail!(
            "migration destination {} already exists; refusing to overwrite",
            destination.display()
        );
    }
    fs::rename(source, destination).with_context(|| {
        format!(
            "move legacy entry {} to {}",
            source.display(),
            destination.display()
        )
    })
}

pub(super) fn migration_record_path(paths: &RebornStoragePaths) -> PathBuf {
    paths.runtime_root().join(MIGRATION_RECORD_FILE)
}

fn write_migration_record(
    paths: &RebornStoragePaths,
    record: &MigrationRecord,
    replace: bool,
) -> anyhow::Result<()> {
    let contents = toml::to_string(record).context("serialize layout migration record")?;
    write_atomic_synced(&migration_record_path(paths), &contents, replace)
}

pub(super) fn read_migration_record(path: &Path) -> anyhow::Result<MigrationRecord> {
    let contents = read_utf8_file_no_follow(path)?;
    let record: MigrationRecord = toml::from_str(&contents)
        .map_err(|error| anyhow!("parse layout migration record {}: {error}", path.display()))?;
    if record.schema_version != MIGRATION_RECORD_SCHEMA_VERSION {
        bail!(
            "unsupported layout migration record schema_version {}; expected {}",
            record.schema_version,
            MIGRATION_RECORD_SCHEMA_VERSION
        );
    }
    Ok(record)
}
