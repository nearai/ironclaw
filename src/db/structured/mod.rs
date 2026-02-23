//! Structured collections: typed, queryable records stored as JSONB.
//!
//! Each collection has a schema (`CollectionSchema`) defining named fields
//! with types, required flags, and defaults. Records are validated against
//! the schema on insert/update, stored as JSONB, and queryable via filters
//! and aggregations.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DatabaseError;

// ==================== Field Types ====================

/// The type of a field in a collection schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Number,
    Date,
    Time,
    DateTime,
    Bool,
    Enum { values: Vec<String> },
}

/// Definition of a single field within a collection schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    #[serde(flatten)]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

// ==================== Collection Schema ====================

/// Schema for a structured collection, defining its name, description, and fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionSchema {
    pub collection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub fields: BTreeMap<String, FieldDef>,
}

// ==================== Record ====================

/// A single record in a structured collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: Uuid,
    pub user_id: String,
    pub collection: String,
    pub data: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// ==================== Filters ====================

/// Comparison operator for a filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Between,
    In,
    IsNull,
    IsNotNull,
}

/// A filter criterion for querying records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub field: String,
    pub op: FilterOp,
    pub value: serde_json::Value,
}

// ==================== Aggregation ====================

/// Aggregation operation type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggOp {
    Sum,
    Count,
    Avg,
    Min,
    Max,
}

/// An aggregation query against a collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aggregation {
    pub operation: AggOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    #[serde(default)]
    pub filters: Vec<Filter>,
}

// ==================== Validation ====================

/// Validation errors for structured collection operations.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("invalid collection name '{name}': {reason}")]
    InvalidName { name: String, reason: String },

    #[error("missing required field '{field}'")]
    MissingRequired { field: String },

    #[error("unknown field '{field}'")]
    UnknownField { field: String },

    #[error("type mismatch for field '{field}': expected {expected}, got {got}")]
    TypeMismatch {
        field: String,
        expected: String,
        got: String,
    },

    #[error("invalid enum value '{value}' for field '{field}'; allowed: {allowed:?}")]
    InvalidEnumValue {
        field: String,
        value: String,
        allowed: Vec<String>,
    },

    #[error("invalid date format for field '{field}': {reason}")]
    InvalidDateFormat { field: String, reason: String },

    #[error("invalid time format for field '{field}': {reason}")]
    InvalidTimeFormat { field: String, reason: String },

    #[error("invalid datetime format for field '{field}': {reason}")]
    InvalidDateTimeFormat { field: String, reason: String },
}

// ==================== Name Validation ====================

/// Validate that an identifier (collection name or field name) contains only
/// safe characters: ASCII alphanumeric and underscore, 1-64 characters,
/// must not start with an underscore.
///
/// This is used for both collection names and field names to prevent SQL
/// injection when identifiers are interpolated into queries.
fn validate_identifier(name: &str, kind: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::InvalidName {
            name: name.to_string(),
            reason: format!("{kind} must not be empty"),
        });
    }
    if name.len() > 64 {
        return Err(ValidationError::InvalidName {
            name: name.to_string(),
            reason: format!("{kind} must be at most 64 characters"),
        });
    }
    if name.starts_with('_') {
        return Err(ValidationError::InvalidName {
            name: name.to_string(),
            reason: format!("{kind} must not start with an underscore"),
        });
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(ValidationError::InvalidName {
            name: name.to_string(),
            reason: format!(
                "{kind} must contain only alphanumeric characters and underscores"
            ),
        });
    }
    Ok(())
}

/// Validate that a field name is safe for use in SQL queries.
///
/// Applies the same rules as collection names: alphanumeric + underscore,
/// 1-64 characters, must not start with an underscore.
pub fn validate_field_name(name: &str) -> Result<(), ValidationError> {
    validate_identifier(name, "field name")
}

impl CollectionSchema {
    /// Validate that a collection name is well-formed.
    ///
    /// Rules: alphanumeric + underscore only, max 64 characters, must not
    /// start with an underscore, must not be empty.
    pub fn validate_name(name: &str) -> Result<(), ValidationError> {
        validate_identifier(name, "collection name")
    }

    /// Validate a full record against this schema, applying defaults for
    /// missing optional fields. Returns the validated (and possibly
    /// augmented) data on success.
    pub fn validate_record(
        &self,
        data: &serde_json::Value,
    ) -> Result<serde_json::Value, ValidationError> {
        let obj = data
            .as_object()
            .ok_or_else(|| ValidationError::TypeMismatch {
                field: "(root)".to_string(),
                expected: "object".to_string(),
                got: json_type_name(data).to_string(),
            })?;

        // Reject unknown fields.
        for key in obj.keys() {
            if !self.fields.contains_key(key) {
                return Err(ValidationError::UnknownField {
                    field: key.clone(),
                });
            }
        }

        let mut result = serde_json::Map::new();

        for (field_name, field_def) in &self.fields {
            match obj.get(field_name) {
                Some(value) if !value.is_null() => {
                    validate_field_value(field_name, &field_def.field_type, value)?;
                    result.insert(field_name.clone(), value.clone());
                }
                _ => {
                    // Field is absent or null.
                    if let Some(ref default_val) = field_def.default {
                        result.insert(field_name.clone(), default_val.clone());
                    } else if field_def.required {
                        return Err(ValidationError::MissingRequired {
                            field: field_name.clone(),
                        });
                    }
                    // Optional field with no default: omit from result.
                }
            }
        }

        Ok(serde_json::Value::Object(result))
    }

    /// Validate a partial update against this schema. Does not check for
    /// missing required fields (since this is a partial update), but does
    /// validate types and reject unknown fields.
    pub fn validate_partial(
        &self,
        data: &serde_json::Value,
    ) -> Result<serde_json::Value, ValidationError> {
        let obj = data
            .as_object()
            .ok_or_else(|| ValidationError::TypeMismatch {
                field: "(root)".to_string(),
                expected: "object".to_string(),
                got: json_type_name(data).to_string(),
            })?;

        // Reject unknown fields.
        for key in obj.keys() {
            if !self.fields.contains_key(key) {
                return Err(ValidationError::UnknownField {
                    field: key.clone(),
                });
            }
        }

        let mut result = serde_json::Map::new();

        for (field_name, value) in obj {
            if let Some(field_def) = self.fields.get(field_name) {
                if value.is_null() {
                    // Reject null for required fields — would leave the record
                    // in an invalid state after merge.
                    if field_def.required {
                        return Err(ValidationError::MissingRequired {
                            field: field_name.clone(),
                        });
                    }
                } else {
                    validate_field_value(field_name, &field_def.field_type, value)?;
                }
                result.insert(field_name.clone(), value.clone());
            }
        }

        Ok(serde_json::Value::Object(result))
    }
}

// ==================== Field Value Validation ====================

/// Validate a single field value against its declared type.
pub fn validate_field_value(
    field: &str,
    field_type: &FieldType,
    value: &serde_json::Value,
) -> Result<(), ValidationError> {
    match field_type {
        FieldType::Text => {
            if !value.is_string() {
                return Err(ValidationError::TypeMismatch {
                    field: field.to_string(),
                    expected: "text".to_string(),
                    got: json_type_name(value).to_string(),
                });
            }
        }
        FieldType::Number => {
            if !value.is_number() {
                return Err(ValidationError::TypeMismatch {
                    field: field.to_string(),
                    expected: "number".to_string(),
                    got: json_type_name(value).to_string(),
                });
            }
        }
        FieldType::Date => {
            let s = value.as_str().ok_or_else(|| ValidationError::TypeMismatch {
                field: field.to_string(),
                expected: "date (string)".to_string(),
                got: json_type_name(value).to_string(),
            })?;
            NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
                ValidationError::InvalidDateFormat {
                    field: field.to_string(),
                    reason: e.to_string(),
                }
            })?;
        }
        FieldType::Time => {
            let s = value.as_str().ok_or_else(|| ValidationError::TypeMismatch {
                field: field.to_string(),
                expected: "time (string)".to_string(),
                got: json_type_name(value).to_string(),
            })?;
            NaiveTime::parse_from_str(s, "%H:%M:%S")
                .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M"))
                .map_err(|e| ValidationError::InvalidTimeFormat {
                    field: field.to_string(),
                    reason: e.to_string(),
                })?;
        }
        FieldType::DateTime => {
            let s = value.as_str().ok_or_else(|| ValidationError::TypeMismatch {
                field: field.to_string(),
                expected: "datetime (string)".to_string(),
                got: json_type_name(value).to_string(),
            })?;
            DateTime::<FixedOffset>::parse_from_rfc3339(s).map_err(|e| {
                ValidationError::InvalidDateTimeFormat {
                    field: field.to_string(),
                    reason: e.to_string(),
                }
            })?;
        }
        FieldType::Bool => {
            if !value.is_boolean() {
                return Err(ValidationError::TypeMismatch {
                    field: field.to_string(),
                    expected: "bool".to_string(),
                    got: json_type_name(value).to_string(),
                });
            }
        }
        FieldType::Enum { values } => {
            let s = value.as_str().ok_or_else(|| ValidationError::TypeMismatch {
                field: field.to_string(),
                expected: "enum (string)".to_string(),
                got: json_type_name(value).to_string(),
            })?;
            if !values.iter().any(|v| v == s) {
                return Err(ValidationError::InvalidEnumValue {
                    field: field.to_string(),
                    value: s.to_string(),
                    allowed: values.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Convert a JSON value to its text representation, matching PostgreSQL
/// `data->>'field'` semantics (which always returns text).
///
/// Used by both the PostgreSQL and libSQL backends for consistent
/// filter comparison and aggregation behavior.
pub fn json_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Return a human-readable name for a JSON value type.
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ==================== StructuredStore Trait ====================

/// Persistence trait for structured collections.
///
/// Implementations manage schema registration, record CRUD, filtering,
/// and aggregation. All operations are scoped by `user_id` to enforce
/// tenant isolation.
#[async_trait]
pub trait StructuredStore: Send + Sync {
    /// Register (or update) a collection schema for the given user.
    async fn register_collection(
        &self,
        user_id: &str,
        schema: &CollectionSchema,
    ) -> Result<(), DatabaseError>;

    /// Retrieve the schema for a specific collection.
    async fn get_collection_schema(
        &self,
        user_id: &str,
        collection: &str,
    ) -> Result<CollectionSchema, DatabaseError>;

    /// List all collection schemas for the given user.
    async fn list_collections(&self, user_id: &str)
        -> Result<Vec<CollectionSchema>, DatabaseError>;

    /// Drop a collection and all its records.
    async fn drop_collection(
        &self,
        user_id: &str,
        collection: &str,
    ) -> Result<(), DatabaseError>;

    /// Insert a new record into a collection. Returns the generated record ID.
    async fn insert_record(
        &self,
        user_id: &str,
        collection: &str,
        data: serde_json::Value,
    ) -> Result<Uuid, DatabaseError>;

    /// Retrieve a single record by ID.
    async fn get_record(&self, user_id: &str, record_id: Uuid)
        -> Result<Record, DatabaseError>;

    /// Update fields on an existing record (partial update / merge).
    async fn update_record(
        &self,
        user_id: &str,
        record_id: Uuid,
        updates: serde_json::Value,
    ) -> Result<(), DatabaseError>;

    /// Delete a single record by ID.
    async fn delete_record(&self, user_id: &str, record_id: Uuid)
        -> Result<(), DatabaseError>;

    /// Query records in a collection with optional filters, ordering, and limit.
    async fn query_records(
        &self,
        user_id: &str,
        collection: &str,
        filters: &[Filter],
        order_by: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Record>, DatabaseError>;

    /// Run an aggregation query against a collection.
    async fn aggregate(
        &self,
        user_id: &str,
        collection: &str,
        aggregation: &Aggregation,
    ) -> Result<serde_json::Value, DatabaseError>;
}
