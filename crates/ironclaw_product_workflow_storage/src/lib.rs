//! Durable product workflow [`IdempotencyLedger`] storage adapters.

mod filesystem_ledger;
mod scoped_lifecycle;

pub use filesystem_ledger::RebornFilesystemIdempotencyLedger;
#[cfg(feature = "libsql")]
pub use filesystem_ledger::RebornLibSqlIdempotencyLedger;
#[cfg(feature = "postgres")]
pub use filesystem_ledger::RebornPostgresIdempotencyLedger;

pub use scoped_lifecycle::FilesystemScopedLifecycleInstallationStore;
#[cfg(feature = "libsql")]
pub use scoped_lifecycle::RebornLibSqlScopedLifecycleInstallationStore;
#[cfg(feature = "postgres")]
pub use scoped_lifecycle::RebornPostgresScopedLifecycleInstallationStore;
