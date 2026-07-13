//! v1 / engine-v2 → Reborn state migration.
//!
//! The public lifecycle is plan → apply/resume → verify. Planning opens only a
//! non-migrating source reader and never constructs a Reborn target writer.

pub mod error;
pub mod manifest;
pub mod options;
pub mod report;

mod inventory;
mod mounts;
mod source;
mod target;
mod v2_model;

mod convert;
mod extension_ownership;

pub use extension_ownership::{
    ExtensionOwnershipMigrationOptions, ExtensionOwnershipMigrationReport,
    run_extension_ownership_migration,
};

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use ironclaw_common::hashing::sha256_hex;
use secrecy::ExposeSecret as _;

pub use error::MigrationError;
pub use manifest::{
    Disposition, DomainCheckpoint, InventoryEntry, InventorySourceKind, MANIFEST_SCHEMA_VERSION,
    MIGRATION_PROTOCOL_VERSION, MigrationManifest, MigrationStatus, RedactedStoreDescriptor,
    ResolvedScope, SourceFingerprint, StoreBackend,
};
pub use options::{
    ApplyAcknowledgements, MigrationOptions, MigrationSecretInputs, SourceDb, TargetStore,
};
pub use report::{Domain, LossReason, LossyItem, MigrationReport, MigrationStats};

/// Compare a currently resolved target with a manifest without serializing or
/// returning the target locator. Intended for `status`/`doctor` read paths.
pub fn manifest_target_matches(target: &TargetStore, manifest: &MigrationManifest) -> bool {
    let current = target_descriptor(target);
    current.backend == manifest.target.backend
        && current.locator_fingerprint == manifest.target.locator_fingerprint
}

/// Inspect a v1 snapshot and build a redacted, versioned migration plan.
///
/// This function does not open the target or create target directories/files.
pub async fn plan_migration(
    options: &MigrationOptions,
) -> Result<MigrationManifest, MigrationError> {
    validate_distinct_stores(&options.source, &options.target)?;
    let source = source::V1Source::open(&options.source).await?;
    let source_schema_version = source.schema_version().await?;
    let inventory = collect_inventory(options, &source).await?;
    // Fingerprint after every planning read has completed. libSQL may update
    // connection-local WAL bookkeeping while opening a snapshot; apply runs
    // the same read sequence before comparing the sealed fingerprint.
    let source_fingerprint = source.fingerprint(&options.source).await?;
    let source_inventory_checksum = inventory_checksum(&inventory)?;

    let mut domains = BTreeMap::new();
    for item in &inventory {
        let checkpoint = domains
            .entry(item.domain)
            .or_insert_with(DomainCheckpoint::default);
        checkpoint.planned = checkpoint.planned.saturating_add(item.count);
        if let Some(blocker) = &item.blocker {
            checkpoint.blockers.push(blocker.clone());
        }
        if let Some(warning) = &item.warning {
            checkpoint.warnings.push(warning.clone());
        }
    }

    let target = target_descriptor(&options.target);
    // Local existence is observable without opening a target. PostgreSQL
    // emptiness is deliberately deferred until apply so planning never makes
    // a target connection.
    let target_empty = match &options.target {
        TargetStore::LibSql { path } => Some(!path.exists()),
        TargetStore::Postgres { .. } => None,
    };
    let mut manifest = MigrationManifest {
        manifest_schema_version: MANIFEST_SCHEMA_VERSION,
        migration_protocol_version: MIGRATION_PROTOCOL_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        release_version: env!("CARGO_PKG_VERSION").to_string(),
        run_id: uuid::Uuid::new_v4(),
        status: MigrationStatus::Planned,
        source: source_descriptor(&options.source),
        target,
        source_schema_version,
        source_fingerprint,
        source_inventory_checksum,
        plan_hash: String::new(),
        scope: ResolvedScope {
            profile: options.profile.clone(),
            tenant_id: options.tenant_id.to_string(),
            agent_id: options.agent_id.to_string(),
            source_home_fingerprint: options
                .source_home
                .as_deref()
                .map(canonicalish)
                .as_deref()
                .map(locator_hash),
            user_mapping: BTreeMap::new(),
            target_empty,
        },
        inventory,
        domains,
        operator_acknowledgements: Vec::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    manifest.seal()?;
    Ok(manifest)
}

/// Apply a previously sealed plan after validating its source snapshot.
///
/// Source and target keys are deliberately resolved independently. The target
/// is not opened until every acknowledgement and source fingerprint check has
/// passed.
pub async fn apply_migration(
    options: MigrationOptions,
    manifest: &MigrationManifest,
    secrets: MigrationSecretInputs,
    acknowledgements: ApplyAcknowledgements,
) -> Result<MigrationReport, MigrationError> {
    apply_migration_inner(options, manifest, secrets, acknowledgements, false).await
}

async fn apply_migration_inner(
    options: MigrationOptions,
    manifest: &MigrationManifest,
    secrets: MigrationSecretInputs,
    acknowledgements: ApplyAcknowledgements,
    is_resume: bool,
) -> Result<MigrationReport, MigrationError> {
    validate_apply_preconditions(&options, manifest, acknowledgements, is_resume).await?;

    let source_options = MigrationOptions {
        secret_master_key: secrets.source_master_key,
        dry_run: false,
        ..options.clone()
    };
    let target_options = MigrationOptions {
        secret_master_key: secrets.target_master_key,
        dry_run: false,
        ..options
    };

    let mut applied_manifest = manifest.transition(MigrationStatus::Applying)?;
    applied_manifest.operator_acknowledgements = vec![
        "source_is_stopped".to_string(),
        "source_is_snapshot".to_string(),
    ];
    applied_manifest.seal()?;
    target::write_shared_migration_state(&target_options.target, &applied_manifest).await?;

    // Re-open only after fingerprint validation, still without running v1
    // migrations. The converter surface exposes reads only.
    let source = source::V1Source::open(&source_options.source).await?;
    let mut target = target::RebornTarget::open(&target_options).await?;
    let mut report = MigrationReport::new(false);
    // Converter ledger keys may read the sealed installation fingerprint from
    // the in-progress report without threading another context parameter
    // through every domain converter.
    report.manifest = Some(applied_manifest.clone());

    let result = run_converters(&source, &mut target, &source_options, &mut report).await;
    match result {
        Ok(()) => {
            record_applied_checkpoints(&mut applied_manifest, &report.stats);
            applied_manifest.status = MigrationStatus::Applied;
        }
        Err(error) => {
            applied_manifest.status = MigrationStatus::Failed;
            applied_manifest.updated_at = Utc::now();
            applied_manifest.seal()?;
            target::write_shared_migration_state(&target_options.target, &applied_manifest).await?;
            report.manifest = Some(applied_manifest);
            return Err(error);
        }
    }
    applied_manifest.updated_at = Utc::now();
    applied_manifest.seal()?;
    target::write_shared_migration_state(&target_options.target, &applied_manifest).await?;
    report.manifest = Some(applied_manifest);
    Ok(report)
}

/// Resume uses the same compare-and-apply path as apply. Converter-level
/// deterministic ledgers are supplied by the next migration phase.
pub async fn resume_migration(
    options: MigrationOptions,
    manifest: &MigrationManifest,
    secrets: MigrationSecretInputs,
    acknowledgements: ApplyAcknowledgements,
) -> Result<MigrationReport, MigrationError> {
    apply_migration_inner(options, manifest, secrets, acknowledgements, true).await
}

/// Validate the sealed manifest and source snapshot, then read supported state
/// back from the cold production stores before marking the run verified.
pub async fn verify_migration(
    options: &MigrationOptions,
    manifest: &MigrationManifest,
) -> Result<MigrationManifest, MigrationError> {
    validate_manifest_source(options, manifest).await?;
    match manifest.status {
        MigrationStatus::Verified => Ok(manifest.clone()),
        MigrationStatus::Applied | MigrationStatus::Verifying => {
            let mut verifying = if manifest.status == MigrationStatus::Applied {
                manifest.transition(MigrationStatus::Verifying)?
            } else {
                manifest.clone()
            };
            target::write_shared_migration_state(&options.target, &verifying).await?;
            let verification = async {
                let readback = target::readback(&options.target, &options.tenant_id).await?;
                verify_readback(&verifying, &readback)?;
                Ok::<(), MigrationError>(())
            }
            .await;
            if let Err(error) = verification {
                let failed = verifying.transition(MigrationStatus::Failed)?;
                target::write_shared_migration_state(&options.target, &failed).await?;
                return Err(error);
            }
            for checkpoint in verifying.domains.values_mut() {
                checkpoint.verified = checkpoint.applied;
            }
            let verified = verifying.transition(MigrationStatus::Verified)?;
            target::write_shared_migration_state(&options.target, &verified).await?;
            Ok(verified)
        }
        status => Err(MigrationError::InvalidInput(format!(
            "verify requires an applied or verifying manifest, got {status:?}"
        ))),
    }
}

/// Temporary compatibility entry point.
///
/// Dry-run callers now receive the non-writing plan manifest. Apply callers
/// retain the old one-shot behavior while the standalone CLI moves to the
/// explicit lifecycle API; the wrapper treats the one-shot invocation as the
/// legacy offline-snapshot acknowledgement.
pub async fn run_migration(options: MigrationOptions) -> Result<MigrationReport, MigrationError> {
    let manifest = plan_migration(&options).await?;
    if options.dry_run {
        let mut report = MigrationReport::new(true);
        report.manifest = Some(manifest);
        return Ok(report);
    }
    let secrets = MigrationSecretInputs::from_legacy(&options);
    apply_migration(
        options,
        &manifest,
        secrets,
        ApplyAcknowledgements::offline_snapshot(),
    )
    .await
}

async fn run_converters(
    source: &source::V1Source,
    target: &mut target::RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    convert::users::run(source, target, options, report).await?;
    convert::projects::run(source, target, options, report).await?;
    convert::threads::run(source, target, options, report).await?;
    convert::automations::run(source, target, options, report).await?;
    convert::memory::run(source, target, options, report).await?;
    convert::jobs::run(source, target, options, report).await?;
    convert::secrets::run(source, target, options, report).await?;
    convert::extensions::run(source, target, options, report).await?;
    convert::identities::run(source, target, options, report).await?;
    convert::heartbeat::run(source, target, options, report).await?;
    convert::settings::run(source, target, options, report).await?;
    Ok(())
}

async fn validate_apply_preconditions(
    options: &MigrationOptions,
    manifest: &MigrationManifest,
    acknowledgements: ApplyAcknowledgements,
    is_resume: bool,
) -> Result<(), MigrationError> {
    if !acknowledgements.source_is_stopped || !acknowledgements.source_is_snapshot {
        return Err(MigrationError::InvalidInput(
            "apply requires both a stopped v1 source acknowledgement and a consistent snapshot acknowledgement"
                .to_string(),
        ));
    }
    if manifest.inventory.iter().any(|item| item.blocker.is_some()) {
        return Err(MigrationError::InvalidInput(
            "migration plan contains unresolved inventory blockers".to_string(),
        ));
    }
    if !is_resume {
        if manifest.status != MigrationStatus::Planned {
            return Err(MigrationError::InvalidInput(
                "apply requires a planned manifest; use resume for an existing run".to_string(),
            ));
        }
        if manifest.scope.target_empty == Some(false) {
            return Err(MigrationError::InvalidInput(
                "apply requires a target that was empty at planning time".to_string(),
            ));
        }
        if !target::target_is_empty(&options.target).await? {
            return Err(MigrationError::InvalidInput(
                "Reborn target is not empty; refusing to overwrite it".to_string(),
            ));
        }
    } else if !matches!(
        manifest.status,
        MigrationStatus::Applying | MigrationStatus::Failed | MigrationStatus::Applied
    ) {
        return Err(MigrationError::InvalidInput(
            "resume requires an applying, failed, or applied manifest".to_string(),
        ));
    }
    validate_manifest_source(options, manifest).await
}

fn record_applied_checkpoints(manifest: &mut MigrationManifest, stats: &MigrationStats) {
    let applied = [
        (Domain::User, stats.users),
        (Domain::Project, stats.projects),
        (Domain::Thread, stats.threads),
        (Domain::Message, stats.messages),
        (Domain::Routine, stats.routines),
        (Domain::Mission, stats.missions),
        (Domain::Trigger, stats.triggers),
        (Domain::Memory, stats.memory_documents),
        (Domain::Secret, stats.secrets),
        (Domain::Extension, stats.extensions),
        (Domain::Identity, stats.identities),
        (Domain::Heartbeat, stats.heartbeats),
    ];
    let completed_at = Utc::now();
    for (domain, count) in applied {
        let checkpoint = manifest.domains.entry(domain).or_default();
        checkpoint.applied = count as u64;
        checkpoint.completed_at = Some(completed_at);
    }
}

fn verify_readback(
    manifest: &MigrationManifest,
    readback: &target::TargetReadback,
) -> Result<(), MigrationError> {
    let expected = |domain: Domain| {
        manifest
            .domains
            .get(&domain)
            .map_or(0, |checkpoint| checkpoint.applied)
    };
    let checks = [
        ("users", expected(Domain::User), readback.users),
        ("projects", expected(Domain::Project), readback.projects),
        ("threads", expected(Domain::Thread), readback.threads),
        ("messages", expected(Domain::Message), readback.messages),
        ("triggers", expected(Domain::Trigger), readback.triggers),
        (
            "memory documents",
            expected(Domain::Memory),
            readback.memory_documents,
        ),
        ("secrets", expected(Domain::Secret), readback.secrets),
    ];
    for (domain, expected, actual) in checks {
        if actual != expected {
            return Err(MigrationError::InvalidInput(format!(
                "target verification failed for {domain}: expected {expected}, found {actual}"
            )));
        }
    }
    let expected_identities = expected(Domain::Identity);
    if readback.identity_records < expected_identities {
        return Err(MigrationError::InvalidInput(format!(
            "target verification failed for identities: expected at least {expected_identities} durable records, found {}",
            readback.identity_records
        )));
    }
    Ok(())
}

async fn validate_manifest_source(
    options: &MigrationOptions,
    manifest: &MigrationManifest,
) -> Result<(), MigrationError> {
    validate_distinct_stores(&options.source, &options.target)?;
    manifest.validate_plan_hash()?;
    if manifest.manifest_schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.migration_protocol_version != MIGRATION_PROTOCOL_VERSION
    {
        return Err(MigrationError::InvalidInput(format!(
            "unsupported migration manifest/protocol version {}/{}",
            manifest.manifest_schema_version, manifest.migration_protocol_version
        )));
    }
    let current_target = target_descriptor(&options.target);
    let current_source_home_fingerprint = options
        .source_home
        .as_deref()
        .map(canonicalish)
        .as_deref()
        .map(locator_hash);
    if source_descriptor(&options.source) != manifest.source
        || current_target.backend != manifest.target.backend
        || current_target.locator_fingerprint != manifest.target.locator_fingerprint
        || options.profile != manifest.scope.profile
        || options.tenant_id.to_string() != manifest.scope.tenant_id
        || options.agent_id.to_string() != manifest.scope.agent_id
        || current_source_home_fingerprint != manifest.scope.source_home_fingerprint
        || !manifest.scope.user_mapping.is_empty()
    {
        return Err(MigrationError::InvalidInput(
            "migration inputs do not match the sealed plan".to_string(),
        ));
    }
    let source = source::V1Source::open(&options.source).await?;
    #[cfg(feature = "postgres")]
    if let (Some(source_pool), TargetStore::Postgres { url }) =
        (source.handles.pg_pool.as_ref(), &options.target)
        && !target::postgres_stores_are_distinct(source_pool, url).await?
    {
        return Err(MigrationError::InvalidInput(
            "v1 source and Reborn target resolve to the same PostgreSQL database".to_string(),
        ));
    }
    let current_inventory = collect_inventory(options, &source).await?;
    if inventory_checksum(&current_inventory)? != manifest.source_inventory_checksum {
        return Err(MigrationError::InvalidInput(
            "v1 source inventory changed after planning; create a new snapshot and plan"
                .to_string(),
        ));
    }
    let current = source.fingerprint(&options.source).await?;
    if current != manifest.source_fingerprint {
        return Err(MigrationError::InvalidInput(
            "v1 source fingerprint changed after planning; create a new snapshot and plan"
                .to_string(),
        ));
    }
    Ok(())
}

async fn collect_inventory(
    options: &MigrationOptions,
    source: &source::V1Source,
) -> Result<Vec<InventoryEntry>, MigrationError> {
    let raw_tables = source.table_inventory().await?;
    let source_path = match &options.source {
        SourceDb::LibSql { path } => Some(path.as_path()),
        SourceDb::Postgres { .. } => None,
    };
    let target_path = match &options.target {
        TargetStore::LibSql { path } => Some(path.as_path()),
        TargetStore::Postgres { .. } => None,
    };
    let mut inventory = inventory::build_table_inventory(raw_tables);
    if options.source_home.is_none() {
        inventory.push(InventoryEntry {
            source_kind: InventorySourceKind::HomeDirectory,
            source_name: "v1_home".to_string(),
            domain: Domain::Setting,
            disposition: Disposition::Unsupported,
            count: 1,
            checksum: sha256_hex(b"v1-home-not-specified"),
            blocker: Some(
                "v1 home was not specified; complete persistent-home inventory cannot be proven"
                    .to_string(),
            ),
            warning: None,
        });
    }
    inventory.extend(inventory::build_home_inventory(
        options.source_home.as_deref(),
        source_path,
        target_path,
    ));
    Ok(inventory)
}

fn inventory_checksum(inventory: &[InventoryEntry]) -> Result<String, MigrationError> {
    Ok(sha256_hex(&serde_json::to_vec(inventory)?))
}

fn source_descriptor(source: &SourceDb) -> RedactedStoreDescriptor {
    match source {
        SourceDb::LibSql { path } => RedactedStoreDescriptor {
            backend: StoreBackend::Libsql,
            locator_fingerprint: locator_hash(&canonicalish(path)),
            exists: Some(path.exists()),
        },
        SourceDb::Postgres { url } => RedactedStoreDescriptor {
            backend: StoreBackend::Postgres,
            locator_fingerprint: locator_hash(Path::new(url.expose_secret())),
            exists: None,
        },
    }
}

fn target_descriptor(target: &TargetStore) -> RedactedStoreDescriptor {
    match target {
        TargetStore::LibSql { path } => RedactedStoreDescriptor {
            backend: StoreBackend::Libsql,
            locator_fingerprint: locator_hash(&canonicalish(path)),
            exists: Some(path.exists()),
        },
        TargetStore::Postgres { url } => RedactedStoreDescriptor {
            backend: StoreBackend::Postgres,
            locator_fingerprint: locator_hash(Path::new(url.expose_secret())),
            exists: None,
        },
    }
}

fn locator_hash(locator: &Path) -> String {
    sha256_hex(locator.as_os_str().as_encoded_bytes())
}

fn validate_distinct_stores(source: &SourceDb, target: &TargetStore) -> Result<(), MigrationError> {
    let same = match (source, target) {
        (SourceDb::LibSql { path: source }, TargetStore::LibSql { path: target }) => {
            canonicalish(source) == canonicalish(target)
        }
        (SourceDb::Postgres { url: source }, TargetStore::Postgres { url: target }) => {
            source.expose_secret() == target.expose_secret()
        }
        _ => false,
    };
    if same {
        return Err(MigrationError::InvalidInput(
            "v1 source and Reborn target must be different stores".to_string(),
        ));
    }
    Ok(())
}

fn canonicalish(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }

    // Resolve symlinks and platform aliases (for example `/var` vs
    // `/private/var` on macOS) without requiring the final target to exist.
    // Hashing the lexical path before creation and the canonical path after
    // creation would otherwise make a valid resume look like target drift.
    let mut ancestor = normalized.clone();
    let mut missing_suffix = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else {
            break;
        };
        missing_suffix.push(name.to_os_string());
        if !ancestor.pop() {
            break;
        }
    }
    let mut resolved = ancestor.canonicalize().unwrap_or(ancestor);
    for component in missing_suffix.into_iter().rev() {
        resolved.push(component);
    }
    resolved
}
