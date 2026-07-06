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
    /// Where to write Reborn state.
    pub target: TargetStore,
    /// Reborn tenant that all migrated state belongs to.
    pub tenant_id: TenantId,
    /// Reborn agent that migrated threads/triggers/memory are scoped to.
    pub agent_id: AgentId,
    /// Secrets master key (v1 ciphertext re-encrypts under Reborn's ported
    /// AES-256-GCM scheme). Required only when migrating secrets.
    pub secret_master_key: Option<SecretString>,
    /// Report only; write nothing to the Reborn store.
    pub dry_run: bool,
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
