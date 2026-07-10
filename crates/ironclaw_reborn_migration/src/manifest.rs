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

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const MIGRATION_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Planned,
    Applying,
    Failed,
    Applied,
    Verifying,
    Verified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreBackend {
    Libsql,
    Postgres,
}

/// A store locator safe for reports and logs.
///
/// `locator_fingerprint` is a one-way hash of the path/URL. It is sufficient
/// for equality checks without serializing credentials, usernames, hosts, or
/// home-directory paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedStoreDescriptor {
    pub backend: StoreBackend,
    pub locator_fingerprint: String,
    pub exists: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFingerprint {
    pub algorithm: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Imported,
    SemanticallyConverted,
    ArchiveOnly,
    RequiresReauth,
    RequiresReinstall,
    IntentionallyReset,
    SkippedByOperator,
    Unsupported,
    UnsupportedUnknown,
    DerivedRebuilt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventorySourceKind {
    Table,
    HomeFile,
    HomeDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    pub source_kind: InventorySourceKind,
    pub source_name: String,
    pub domain: Domain,
    pub disposition: Disposition,
    pub count: u64,
    /// Digest of non-secret inventory metadata. Never a digest of plaintext
    /// secret material or bearer tokens.
    pub checksum: String,
    pub blocker: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainCheckpoint {
    pub planned: u64,
    pub applied: u64,
    pub verified: u64,
    pub conflicts: u64,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedScope {
    pub profile: String,
    pub tenant_id: String,
    pub agent_id: String,
    pub user_mapping: BTreeMap<String, String>,
    pub target_empty: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationManifest {
    pub manifest_schema_version: u32,
    pub migration_protocol_version: u32,
    pub tool_version: String,
    pub release_version: String,
    pub run_id: Uuid,
    pub status: MigrationStatus,
    pub source: RedactedStoreDescriptor,
    pub target: RedactedStoreDescriptor,
    pub source_schema_version: Option<String>,
    pub source_fingerprint: SourceFingerprint,
    pub source_inventory_checksum: String,
    pub plan_hash: String,
    pub scope: ResolvedScope,
    pub inventory: Vec<InventoryEntry>,
    pub domains: BTreeMap<Domain, DomainCheckpoint>,
    pub operator_acknowledgements: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MigrationManifest {
    pub(crate) fn seal(&mut self) -> Result<(), MigrationError> {
        self.plan_hash.clear();
        self.plan_hash = sha256_hex(&serde_json::to_vec(self)?);
        Ok(())
    }

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
        }
        Ok(())
    }
}
