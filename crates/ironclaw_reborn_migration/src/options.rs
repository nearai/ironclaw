//! Typed inputs to a migration run.

use std::path::PathBuf;

use ironclaw_host_api::{AgentId, TenantId};
use secrecy::SecretString;

/// Everything a migration run needs: where the v1 state is, where Reborn state
/// should be written, and the Reborn scope dimensions that v1 never had (tenant,
/// agent) so single-user v1 rows land in the right Reborn cell.
#[derive(Clone)]
pub struct MigrationOptions {
    /// Backend + connection details for the v1 source database.
    pub source: SourceDb,
    /// Explicit v1 home whose persistent artifacts must be inventoried.
    /// When absent, planning records a blocker rather than guessing from the
    /// database snapshot location.
    pub source_home: Option<PathBuf>,
    /// Where to write Reborn state.
    pub target: TargetStore,
    /// Effective production Reborn profile selected by composition.
    pub profile: String,
    /// Reborn tenant that all migrated state belongs to.
    pub tenant_id: TenantId,
    /// Reborn agent that migrated threads/triggers/memory are scoped to.
    pub agent_id: AgentId,
    /// Legacy single-key compatibility input used only by `run_migration`.
    /// New lifecycle callers must use [`MigrationSecretInputs`] so v1
    /// decryption and Reborn encryption resolve independently.
    pub secret_master_key: Option<SecretString>,
    /// Report only; write nothing to the Reborn store.
    pub dry_run: bool,
}

/// Secret material used by the two sides of an apply operation.
///
/// v1 ciphertext and Reborn ciphertext do not have to use the same master key.
/// Keeping the values in a separate, non-`Debug` input also makes it harder for
/// a lifecycle request or serialized manifest to accidentally disclose them.
#[derive(Clone, Default)]
pub struct MigrationSecretInputs {
    /// Key used only while decrypting values read from the v1 snapshot.
    pub source_master_key: Option<SecretString>,
    /// Key used only while encrypting values written to Reborn.
    pub target_master_key: Option<SecretString>,
}

impl MigrationSecretInputs {
    /// Build the split input from the legacy single-key option.
    ///
    /// This exists only for the temporary [`crate::run_migration`]
    /// compatibility wrapper. New callers should resolve both sides
    /// independently and construct this type directly.
    pub fn from_legacy(options: &MigrationOptions) -> Self {
        Self {
            source_master_key: options.secret_master_key.clone(),
            target_master_key: options.secret_master_key.clone(),
        }
    }
}

/// Operator assertions required before the first-release offline apply path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyAcknowledgements {
    /// The source v1 process and every other writer have been stopped.
    pub source_is_stopped: bool,
    /// The selected source is an operator-created, consistent snapshot.
    pub source_is_snapshot: bool,
}

impl ApplyAcknowledgements {
    pub const fn offline_snapshot() -> Self {
        Self {
            source_is_stopped: true,
            source_is_snapshot: true,
        }
    }
}

/// v1 source database selector. Mirrors `ironclaw::config::DatabaseConfig`
/// enough to open a read connection via `ironclaw::db::connect_with_handles`.
#[derive(Debug, Clone)]
pub enum SourceDb {
    /// libSQL/SQLite file on disk.
    LibSql { path: PathBuf },
    /// PostgreSQL connection URL. Held as a `SecretString` because the URL
    /// typically embeds `user:password@host`; `secrecy` redacts it under the
    /// derived `Debug`.
    Postgres { url: SecretString },
}

/// Where Reborn state is written. The `RootFilesystem` KV substrate (threads,
/// memory, secrets, extensions, identity) and the triggers DB share the same
/// underlying backend handle.
#[derive(Debug, Clone)]
pub enum TargetStore {
    /// Local libSQL file (the `reborn-local-dev.db` shape).
    LibSql { path: PathBuf },
    /// PostgreSQL connection URL. Held as a `SecretString` (see [`SourceDb`]).
    Postgres { url: SecretString },
}
