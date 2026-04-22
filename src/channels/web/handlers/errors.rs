//! Shared error-conversion helpers for web handlers.
//!
//! `db_error_to_status` is the preferred entry point when mapping a
//! `DatabaseError` from a handler: it classifies known constraint violations
//! into specific 4xx responses so clients can distinguish validation
//! conflicts from retryable server failures. Fall back to
//! `internal_db_error` only when the caller does not have a typed
//! `DatabaseError` (e.g. conversion errors).

use axum::http::StatusCode;

use crate::error::DatabaseError;

/// Convert a `DatabaseError` into an HTTP `(status, body)` tuple.
///
/// Client-visible constraint violations are classified by SQL error code
/// (PostgreSQL `SqlState` / SQLite extended code) and mapped to 4xx:
/// - UNIQUE        → 409 Conflict
/// - FOREIGN KEY   → 422 Unprocessable Entity
/// - CHECK         → 422 Unprocessable Entity
/// - NOT NULL      → 400 Bad Request
///
/// Application-level `DatabaseError::Constraint(msg)` (e.g. the last-owner
/// guard in `pg_check_not_last_owner`) is also mapped to 409, preserving
/// the caller-supplied message.
///
/// All other `DatabaseError` variants are logged at `error!` level and
/// returned as 500 with a generic body.
pub(crate) fn db_error_to_status(err: DatabaseError) -> (StatusCode, String) {
    if err.is_unique_violation() {
        tracing::info!("Client-visible DB conflict (unique violation): {err}");
        return (
            StatusCode::CONFLICT,
            "A resource with the same unique key already exists".to_string(),
        );
    }
    if err.is_foreign_key_violation() {
        tracing::info!("Client-visible DB error (foreign key violation): {err}");
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Referenced resource does not exist".to_string(),
        );
    }
    if err.is_check_violation() {
        tracing::info!("Client-visible DB error (check violation): {err}");
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Value violates constraint".to_string(),
        );
    }
    if err.is_not_null_violation() {
        tracing::info!("Client-visible DB error (not null violation): {err}");
        return (
            StatusCode::BAD_REQUEST,
            "Required field is missing".to_string(),
        );
    }
    if let DatabaseError::Constraint(msg) = &err {
        tracing::info!("Application constraint violation: {err}");
        return (StatusCode::CONFLICT, msg.clone());
    }

    tracing::error!("Internal database error: {err}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal database error".to_string(),
    )
}

/// Fallback for callers that do not have a typed `DatabaseError`.
/// Prefer `db_error_to_status` when possible.
pub(crate) fn internal_db_error(e: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("Database error: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal database error".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_variant_maps_to_409_with_message() {
        let err = DatabaseError::Constraint("cannot demote the last owner".into());
        let (status, body) = db_error_to_status(err);
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body, "cannot demote the last owner");
    }

    #[test]
    fn generic_db_error_maps_to_500() {
        let err = DatabaseError::Pool("connection refused".into());
        let (status, _) = db_error_to_status(err);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn libsql_unique_violation_maps_to_409() {
        // SQLITE_CONSTRAINT_UNIQUE = 2067
        let err = DatabaseError::LibSql(libsql::Error::SqliteFailure(
            2067,
            "UNIQUE constraint failed: workspaces.slug".into(),
        ));
        let (status, _) = db_error_to_status(err);
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn libsql_foreign_key_violation_maps_to_422() {
        // SQLITE_CONSTRAINT_FOREIGNKEY = 787
        let err = DatabaseError::LibSql(libsql::Error::SqliteFailure(
            787,
            "FOREIGN KEY constraint failed".into(),
        ));
        let (status, _) = db_error_to_status(err);
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn libsql_not_null_violation_maps_to_400() {
        // SQLITE_CONSTRAINT_NOTNULL = 1299
        let err = DatabaseError::LibSql(libsql::Error::SqliteFailure(
            1299,
            "NOT NULL constraint failed".into(),
        ));
        let (status, _) = db_error_to_status(err);
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
