//! Typed operator migration inputs.

use std::path::PathBuf;

use secrecy::SecretString;

/// Where Reborn operator migrations read and write state.
#[derive(Debug, Clone)]
pub enum TargetStore {
    /// Local libSQL file (the `reborn-local-dev.db` shape).
    LibSql { path: PathBuf },
    /// PostgreSQL connection URL. Held as a `SecretString` (see [`SourceDb`]).
    Postgres { url: SecretString },
}
