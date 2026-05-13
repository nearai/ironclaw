//! Storage error mapping for the product workflow storage crate.
//!
//! Internal DB errors are converted to `ProductWorkflowError::Transient` so the
//! trait surface stays clean and webhook retries can replay the action.

use ironclaw_product_workflow::ProductWorkflowError;

#[allow(dead_code)]
pub(crate) fn transient(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: reason.into(),
    }
}

#[allow(dead_code)]
pub(crate) fn permanent(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::TurnSubmissionRejected {
        reason: reason.into(),
    }
}

#[cfg(feature = "libsql")]
pub(crate) fn libsql_error(err: ::libsql::Error) -> ProductWorkflowError {
    transient(format!("libsql: {err}"))
}

#[cfg(feature = "postgres")]
pub(crate) fn postgres_error(err: ::tokio_postgres::Error) -> ProductWorkflowError {
    transient(format!("postgres: {err}"))
}

#[cfg(feature = "postgres")]
pub(crate) fn pool_error(err: ::deadpool_postgres::PoolError) -> ProductWorkflowError {
    transient(format!("postgres pool: {err}"))
}
