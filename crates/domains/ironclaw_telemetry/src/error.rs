use std::{
    error::Error as StdError,
    num::{ParseIntError, TryFromIntError},
};

use thiserror::Error;

/// Errors returned by the telemetry repositories. Backend sources remain
/// attached so callers can distinguish storage failure from invalid durable
/// data without exposing driver types in the repository contract.
#[derive(Debug, Error)]
pub enum TelemetryRepositoryError {
    #[error("telemetry scope does not match persisted record tenant")]
    ScopeMismatch,
    #[error("invalid telemetry scan request: {reason}")]
    InvalidScanRequest { reason: &'static str },
    #[error("invalid telemetry page request: {reason}")]
    InvalidPageRequest { reason: &'static str },
    #[error("invalid telemetry page cursor")]
    InvalidCursor,
    #[error("invalid telemetry page cursor encoding")]
    InvalidCursorEncoding {
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    #[error("invalid telemetry page cursor length")]
    InvalidCursorLength {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("invalid persisted telemetry timestamp in {field}")]
    InvalidTimestamp {
        field: &'static str,
        #[source]
        source: chrono::ParseError,
    },
    #[error("invalid persisted telemetry {field}")]
    InvalidPersistedField {
        field: &'static str,
        value: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    #[error("unknown persisted telemetry {field}")]
    UnknownEnum { field: &'static str, value: String },
    #[error("invalid persisted telemetry index projection")]
    InvalidProjection,
    #[error("telemetry counter overflow for {family} row")]
    CounterOverflow { family: &'static str },
    #[error("telemetry counter conversion failed for {family} row")]
    CounterConversion {
        family: &'static str,
        value: i64,
        #[source]
        source: TryFromIntError,
    },
    #[error(transparent)]
    Record(#[from] crate::records::RecordError),
    #[error("telemetry runtime admission failed while {operation}")]
    StorageAdmission {
        operation: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    #[error("telemetry storage operation failed while {operation}")]
    StorageOperation {
        operation: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    #[error("telemetry pool admission failed while {operation}")]
    StoragePoolAdmission {
        operation: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    #[error("telemetry JSON serialization failed while {operation}")]
    Serialization {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

impl TelemetryRepositoryError {
    pub(crate) fn invalid_cursor_encoding<E>(source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::InvalidCursorEncoding {
            source: Box::new(source),
        }
    }

    pub(crate) fn invalid_cursor_length(value: String, source: ParseIntError) -> Self {
        Self::InvalidCursorLength { value, source }
    }

    pub(crate) fn invalid_persisted_field<E>(field: &'static str, value: String, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::InvalidPersistedField {
            field,
            value,
            source: Box::new(source),
        }
    }
}
