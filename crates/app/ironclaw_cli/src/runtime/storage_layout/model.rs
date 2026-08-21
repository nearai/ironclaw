use super::*;

pub(super) const LAYOUT_MANIFEST_FILE: &str = "layout.toml";
pub(super) const MIGRATION_RECORD_FILE: &str = "layout-migration.toml";
pub(super) const MIGRATION_LOCK_FILE: &str = ".reborn-storage-migration.lock";
pub(super) const MIGRATION_RECORD_SCHEMA_VERSION: u32 = 1;
pub(super) const DB_FILE: &str = "reborn-local-dev.db";
pub(super) const MASTER_KEY_FILE: &str = ironclaw_composition::STANDALONE_SECRETS_MASTER_KEY_PATH;
pub(super) const LIBSQL_DB_UNIT: &[&str] = &[
    DB_FILE,
    "reborn-local-dev.db-wal",
    "reborn-local-dev.db-shm",
    "reborn-local-dev.db-journal",
];
pub(super) const SYSTEM_CONTENT_DIRS: &[&str] = &["extensions", "prompts", "skills"];

// Keep the discovery module's in-flight split mechanically isolated while
// migration code uses the neutral config-owned source directly.
pub(super) use ironclaw_config::LegacyStorageSource as LegacySourceKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyCandidate {
    pub(super) kind: LegacyStorageSource,
    pub(super) source_root: PathBuf,
    pub(super) db_files: Vec<String>,
    pub(super) has_master_key: bool,
    pub(super) has_system_content: bool,
    pub(super) has_legacy_skills: bool,
}

impl LegacyCandidate {
    pub(super) fn is_embedded(&self) -> bool {
        self.kind.requirement().durable_state == DurableStateKind::EmbeddedLibSql
    }
}

/// Typed startup decision before any legacy migration work begins.
#[derive(Debug)]
pub(crate) enum StartupLayoutAdmission {
    Ready(RebornStoragePaths),
    MigrationRequired(Vec<LegacyCandidate>),
}

/// Operator control over boot-time legacy layout migration. Migration is
/// automatic by default; `manual` defers it to an operator-scheduled restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageMigrationPolicy {
    Automatic,
    Manual,
}

impl StorageMigrationPolicy {
    pub(crate) const ENV: &'static str = "IRONCLAW_REBORN_STORAGE_MIGRATION";
    pub(crate) const MANUAL: &'static str = "manual";
    pub(crate) const AUTOMATIC: &'static str = "automatic";

    pub(crate) fn from_environment_value(value: Option<&str>) -> anyhow::Result<Self> {
        match value {
            None => Ok(Self::Automatic),
            Some(Self::AUTOMATIC) => Ok(Self::Automatic),
            Some(Self::MANUAL) => Ok(Self::Manual),
            Some(other) => bail!(
                "{} must be `{}` or `{}` (got `{other}`)",
                Self::ENV,
                Self::AUTOMATIC,
                Self::MANUAL
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum MigrationPhase {
    InProgress,
    Complete,
}

/// Durable provenance for the one-time legacy layout migration. Published to
/// `runtime/layout-migration.toml` before the first rename and retained after
/// completion so an operator can always see which source was chosen and which
/// populated candidates were deliberately left in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MigrationRecord {
    pub(super) schema_version: u32,
    pub(super) phase: MigrationPhase,
    pub(super) source: LegacyStorageSource,
    pub(super) source_root: PathBuf,
    /// Exact manifest selected and admitted before the first source rename.
    /// A completed migration can publish only this value after a crash in the
    /// final record-to-manifest window; it is never reconstructed from a later
    /// startup request.
    pub(super) target_manifest: LayoutManifest,
    pub(super) has_legacy_skills: bool,
    #[serde(default)]
    pub(super) ignored: Vec<IgnoredCandidate>,
}

/// A populated legacy candidate that lost the recency selection. Its data is
/// left byte-for-byte untouched at `source_root`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IgnoredCandidate {
    pub(super) source: LegacyStorageSource,
    pub(super) source_root: PathBuf,
    pub(super) last_used_epoch_secs: u64,
}
