//! Versioned, redacted migration plan contract.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

use chrono::{DateTime, Utc};
use ironclaw_common::hashing::sha256_hex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::MigrationError;
use crate::report::Domain;

/// Current serialized manifest schema version.
pub const MANIFEST_SCHEMA_VERSION: u32 = 3;
/// Companion lifecycle protocol version required by the primary CLI.
pub const MIGRATION_PROTOCOL_VERSION: u32 = 1;

/// Persisted lifecycle state. Every state between `Applying` and `Verified` is
/// quarantined from live runtime activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    /// Source inventory is sealed and no target writes have begun.
    Planned,
    /// Apply or resume may be writing target records.
    Applying,
    /// Apply or verification failed and requires operator action.
    Failed,
    /// Conversion completed but structural verification has not.
    Applied,
    /// Structural durable-store verification is in progress.
    Verifying,
    /// Structural verification completed successfully.
    Verified,
}

/// Durable-store family named by a redacted descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreBackend {
    /// Embedded libSQL/SQLite store.
    Libsql,
    /// PostgreSQL store.
    Postgres,
}

/// A store locator safe for reports and logs.
///
/// `locator_fingerprint` is a one-way hash of a local path or credential-free
/// PostgreSQL locator components. It is sufficient for equality checks without
/// serializing credentials, usernames, hosts, or home-directory paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedStoreDescriptor {
    /// Store family.
    pub backend: StoreBackend,
    /// One-way, credential-free locator identity.
    pub locator_fingerprint: String,
    /// Local existence observation; `None` when planning cannot inspect it
    /// without opening a remote target.
    pub exists: Option<bool>,
}

/// Versioned digest that seals the source snapshot contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    /// Fingerprint algorithm identifier.
    pub algorithm: String,
    /// Hex-encoded digest.
    pub value: String,
}

/// Planned handling for one known source category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Copied into the equivalent Reborn concept.
    Imported,
    /// Converted into a different Reborn concept.
    SemanticallyConverted,
    /// Counted and checksummed only; payload is not copied.
    ArchiveOnly,
    /// Operator must issue new credentials after cutover.
    RequiresReauth,
    /// Operator must reinstall the artifact after cutover.
    RequiresReinstall,
    /// Transient state deliberately starts clean.
    IntentionallyReset,
    /// Excluded by an explicit operator-selected scope.
    SkippedByOperator,
    /// Known source data has no safe representation in this release.
    Unsupported,
    /// Unrecognized source data blocks apply.
    UnsupportedUnknown,
    /// Derived state is recomputed by Reborn.
    DerivedRebuilt,
}

/// Physical source category represented by an inventory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventorySourceKind {
    /// Database table.
    Table,
    /// Persistent v1-home file.
    HomeFile,
    /// Persistent v1-home directory.
    HomeDirectory,
}

/// Counted and classified source category in a sealed plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    /// Physical category kind.
    pub source_kind: InventorySourceKind,
    /// Stable table or home-artifact name.
    pub source_name: String,
    /// Logical conversion domain.
    pub domain: Domain,
    /// Required handling in this release.
    pub disposition: Disposition,
    /// Number of source records or path entries observed.
    pub count: u64,
    /// Digest of non-secret inventory metadata. Never a digest of plaintext
    /// secret material or bearer tokens.
    pub checksum: String,
    /// Apply-preventing issue discovered during inventory.
    pub blocker: Option<String>,
    /// Reviewable non-blocking caveat.
    pub warning: Option<String>,
}

/// Lifecycle accounting for one logical domain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainCheckpoint {
    /// Source records planned for the domain.
    pub planned: u64,
    /// Records converted successfully.
    pub applied: u64,
    /// Records covered by structural readback.
    pub verified: u64,
    /// Divergent records that prevented overwrite.
    pub conflicts: u64,
    /// Apply-preventing domain issues.
    pub blockers: Vec<String>,
    /// Non-blocking domain caveats.
    pub warnings: Vec<String>,
    /// Completion time for the latest successful domain phase.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Production profile and authority scope sealed into the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedScope {
    /// Effective Reborn profile.
    pub profile: String,
    /// Target tenant identity.
    pub tenant_id: String,
    /// Target agent identity.
    pub agent_id: String,
    /// One-way identity of the explicit v1 home, when supplied.
    pub source_home_fingerprint: Option<String>,
    /// Deterministic source-to-target user identity mapping.
    pub user_mapping: BTreeMap<String, String>,
    /// Planning-time local target emptiness; remote targets use `None`.
    pub target_empty: Option<bool>,
}

/// Versioned, redacted plan and lifecycle authority for one migration run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationManifest {
    /// Serialized manifest schema version.
    pub manifest_schema_version: u32,
    /// Companion protocol version.
    pub migration_protocol_version: u32,
    /// Migrator build version.
    pub tool_version: String,
    /// Release version that must match the primary CLI.
    pub release_version: String,
    /// Unique migration-run identity.
    pub run_id: Uuid,
    /// Current lifecycle state.
    pub status: MigrationStatus,
    /// Redacted source locator.
    pub source: RedactedStoreDescriptor,
    /// Redacted target locator.
    pub target: RedactedStoreDescriptor,
    /// v1 schema version, when the source records one.
    pub source_schema_version: Option<String>,
    /// Source snapshot content seal.
    pub source_fingerprint: SourceFingerprint,
    /// Digest over the classified inventory.
    pub source_inventory_checksum: String,
    /// Seal over the complete manifest with this field cleared.
    pub plan_hash: String,
    /// Production scope and identity mapping.
    pub scope: ResolvedScope,
    /// Complete classified source inventory.
    pub inventory: Vec<InventoryEntry>,
    /// Per-domain lifecycle accounting.
    pub domains: BTreeMap<Domain, DomainCheckpoint>,
    /// Offline assertions recorded at apply time.
    pub operator_acknowledgements: Vec<String>,
    /// Plan creation time.
    pub created_at: DateTime<Utc>,
    /// Latest lifecycle update time.
    pub updated_at: DateTime<Utc>,
}

impl MigrationManifest {
    pub(crate) fn seal(&mut self) -> Result<(), MigrationError> {
        self.plan_hash.clear();
        self.plan_hash = sha256_hex(&serde_json::to_vec(self)?);
        Ok(())
    }

    /// Validate that no sealed manifest field changed since the last seal.
    pub fn validate_plan_hash(&self) -> Result<(), MigrationError> {
        let expected = self.plan_hash.clone();
        let mut candidate = self.clone();
        candidate.seal()?;
        if candidate.plan_hash != expected {
            return Err(MigrationError::InvalidInput(
                "migration manifest plan hash does not match its contents".to_string(),
            ));
        }
        Ok(())
    }

    /// Return a resealed lifecycle copy after checking the state transition.
    /// Callers can atomically persist `Applying` before target writes and
    /// `Failed` on an error path, so a crash never leaves only an in-memory
    /// partial report.
    pub fn transition(&self, status: MigrationStatus) -> Result<Self, MigrationError> {
        let valid = matches!(
            (self.status, status),
            (MigrationStatus::Planned, MigrationStatus::Applying)
                | (MigrationStatus::Applying, MigrationStatus::Failed)
                | (MigrationStatus::Applying, MigrationStatus::Applied)
                | (MigrationStatus::Failed, MigrationStatus::Applying)
                | (MigrationStatus::Applied, MigrationStatus::Applying)
                | (MigrationStatus::Applied, MigrationStatus::Verifying)
                | (MigrationStatus::Verified, MigrationStatus::Verifying)
                | (MigrationStatus::Verifying, MigrationStatus::Failed)
                | (MigrationStatus::Verifying, MigrationStatus::Verified)
        );
        if !valid {
            return Err(MigrationError::InvalidInput(format!(
                "invalid migration status transition {:?} -> {:?}",
                self.status, status
            )));
        }
        let mut next = self.clone();
        next.status = status;
        next.updated_at = Utc::now();
        next.seal()?;
        Ok(next)
    }

    /// Serialize the redacted manifest as pretty JSON.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Persist a manifest via a same-directory temporary file.
    ///
    /// The default is no-clobber. On Unix, both the temporary and final file
    /// are owner-readable/writable only. `hard_link` provides an atomic
    /// create-if-absent operation without a check-then-rename race.
    pub fn write_atomic(&self, path: &Path, overwrite: bool) -> Result<(), MigrationError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("migration-manifest.json");
        let tmp_path = path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

        let mut open = OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            open.mode(0o600);
        }
        let mut file = open.open(&tmp_path)?;
        let json = self.to_json()?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);

        let result = if overwrite {
            std::fs::rename(&tmp_path, path)
        } else {
            std::fs::hard_link(&tmp_path, path).and_then(|()| std::fs::remove_file(&tmp_path))
        };
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }
        result?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}
