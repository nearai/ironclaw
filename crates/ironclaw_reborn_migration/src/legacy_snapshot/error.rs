//! Error type for the frozen v1 read path.

/// Errors from the frozen v1 reader. Every variant maps to a
/// [`crate::error::MigrationError`] at the call site via `.to_string()`, same
/// as the original `ironclaw::db`/`ironclaw::secrets` error types did.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LegacyError {
    #[error("failed to connect to legacy database: {0}")]
    Connect(String),

    /// The source database is missing a column/table this reader expects for
    /// the schema version it was frozen against. Deliberately fails loud
    /// instead of silently reading a partial row — this reader does not apply
    /// migrations (see `connect::ensure_schema_current`), so an out-of-date
    /// source database must be migrated by running the legacy `ironclaw`
    /// binary once before this tool runs.
    #[error(
        "legacy database is not at the schema version this migration tool expects: \
         table '{table}' is missing expected column '{column}'. Run the legacy `ironclaw` \
         binary once against this database (it applies its own migrations on startup) \
         before running the migration tool."
    )]
    SchemaMismatch { table: String, column: String },

    #[error("legacy query failed: {0}")]
    Query(String),

    #[error("legacy row decode failed ({what}): {field}")]
    Decode { what: String, field: String },
}
