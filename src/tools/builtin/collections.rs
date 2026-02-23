//! Built-in tools for structured collections.
//!
//! Two categories:
//!
//! **Management tools** (static, one instance each):
//! - `collections_list` — List all registered collections
//! - `collections_register` — Register a new collection schema
//! - `collections_drop` — Drop a collection and all its records
//!
//! **Per-collection tools** (dynamically generated per schema):
//! - `{collection}_add` — Insert a record with typed fields
//! - `{collection}_update` — Update fields on an existing record
//! - `{collection}_delete` — Delete a record by ID
//! - `{collection}_query` — Query records with filters
//! - `{collection}_summary` — Aggregate records (sum, count, avg, min, max)
//!
//! At startup, all existing collection schemas are loaded and per-collection
//! tools are generated. When `collections_register` is called mid-session,
//! it also registers the new per-collection tools dynamically.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::context::JobContext;
use crate::db::structured::{
    AggOp, Aggregation, CollectionSchema, FieldType, Filter,
};
use crate::db::Database;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str};

// ==================== Schema → JSON Schema conversion ====================

/// Convert a `FieldType` to its JSON Schema representation.
fn field_type_to_json_schema(field_type: &FieldType) -> serde_json::Value {
    match field_type {
        FieldType::Text => json!({ "type": "string" }),
        FieldType::Number => json!({ "type": "number" }),
        FieldType::Date => json!({ "type": "string", "format": "date", "description": "Date in YYYY-MM-DD format" }),
        FieldType::Time => json!({ "type": "string", "description": "Time in HH:MM or HH:MM:SS format" }),
        FieldType::DateTime => json!({ "type": "string", "format": "date-time", "description": "ISO 8601 datetime (e.g. 2026-02-22T08:00:00Z)" }),
        FieldType::Bool => json!({ "type": "boolean" }),
        FieldType::Enum { values } => json!({ "type": "string", "enum": values }),
    }
}

/// Generate tool instances for a collection schema.
pub fn generate_collection_tools(
    schema: &CollectionSchema,
    db: Arc<dyn Database>,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(CollectionAddTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionUpdateTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionDeleteTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionQueryTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionSummaryTool::new(schema.clone(), db)),
    ]
}

// ==================== Management Tools ====================

/// Tool to list all registered structured collections.
pub struct CollectionListTool {
    db: Arc<dyn Database>,
}

impl CollectionListTool {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for CollectionListTool {
    fn name(&self) -> &str {
        "collections_list"
    }

    fn description(&self) -> &str {
        "List all registered structured data collections. Returns collection names, \
         descriptions, and field definitions. Use this to discover what structured \
         data is available before querying."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let schemas = self
            .db
            .list_collections(&ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to list collections: {e}")))?;

        let collections: Vec<serde_json::Value> = schemas
            .iter()
            .map(|s| {
                let fields: serde_json::Value = s
                    .fields
                    .iter()
                    .map(|(name, def)| {
                        (
                            name.clone(),
                            json!({
                                "type": field_type_to_json_schema(&def.field_type),
                                "required": def.required,
                            }),
                        )
                    })
                    .collect::<serde_json::Map<String, serde_json::Value>>()
                    .into();
                json!({
                    "collection": s.collection,
                    "description": s.description,
                    "fields": fields,
                })
            })
            .collect();

        Ok(ToolOutput::success(
            json!({
                "collections": collections,
                "count": collections.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool to register a new structured collection.
///
/// When called, this also dynamically registers per-collection tools
/// so the LLM can immediately start using them.
pub struct CollectionRegisterTool {
    db: Arc<dyn Database>,
    registry: Arc<ToolRegistry>,
}

impl CollectionRegisterTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self { db, registry }
    }
}

#[async_trait]
impl Tool for CollectionRegisterTool {
    fn name(&self) -> &str {
        "collections_register"
    }

    fn description(&self) -> &str {
        "Register a new structured data collection with a typed schema. \
         After registration, dedicated tools for adding, updating, deleting, \
         querying, and summarizing records become available immediately. \
         Use this when you need to track structured data like schedules, \
         lists, or any records with defined fields."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Name for the collection (alphanumeric + underscores, e.g. 'nanny_shifts', 'grocery_items')"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of what this collection tracks"
                },
                "fields": {
                    "type": "object",
                    "description": "Field definitions. Each key is a field name, value is an object with 'type' (text/number/date/time/datetime/bool/enum), optional 'required' (boolean), optional 'default', and for enum type: 'values' (array of allowed strings).",
                    "additionalProperties": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["text", "number", "date", "time", "datetime", "bool", "enum"]
                            },
                            "required": { "type": "boolean" },
                            "default": {},
                            "values": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Allowed values (only for enum type)"
                            }
                        },
                        "required": ["type"]
                    }
                }
            },
            "required": ["collection", "fields"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse the schema from parameters
        let schema: CollectionSchema = serde_json::from_value(params.clone()).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid collection schema: {e}"))
        })?;

        // Validate name
        CollectionSchema::validate_name(&schema.collection).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid collection name: {e}"))
        })?;

        // Register in database
        self.db
            .register_collection(&ctx.user_id, &schema)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to register collection: {e}"))
            })?;

        // Generate and register per-collection tools dynamically
        let tools = generate_collection_tools(&schema, Arc::clone(&self.db));
        let tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        for tool in tools {
            self.registry.register(tool).await;
        }

        Ok(ToolOutput::success(
            json!({
                "status": "registered",
                "collection": schema.collection,
                "tools_created": tool_names,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(5, 50))
    }
}

/// Tool to drop a structured collection and all its records.
pub struct CollectionDropTool {
    db: Arc<dyn Database>,
    registry: Arc<ToolRegistry>,
}

impl CollectionDropTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self { db, registry }
    }
}

#[async_trait]
impl Tool for CollectionDropTool {
    fn name(&self) -> &str {
        "collections_drop"
    }

    fn description(&self) -> &str {
        "Drop a structured data collection and permanently delete all its records. \
         This action cannot be undone. The associated tools for this collection \
         will also be removed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Name of the collection to drop"
                }
            },
            "required": ["collection"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let collection = require_str(&params, "collection")?;

        // Verify the collection exists
        self.db
            .get_collection_schema(&ctx.user_id, collection)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Collection not found: {e}"))
            })?;

        // Drop from database (cascades to records)
        self.db
            .drop_collection(&ctx.user_id, collection)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to drop collection: {e}"))
            })?;

        // Unregister per-collection tools
        let tool_suffixes = ["_add", "_update", "_delete", "_query", "_summary"];
        let mut removed = Vec::new();
        for suffix in &tool_suffixes {
            let tool_name = format!("{collection}{suffix}");
            if self.registry.unregister(&tool_name).await.is_some() {
                removed.push(tool_name);
            }
        }

        Ok(ToolOutput::success(
            json!({
                "status": "dropped",
                "collection": collection,
                "tools_removed": removed,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(5, 50))
    }
}

// ==================== Per-Collection Tools ====================

/// Tool for adding a record to a specific collection.
///
/// Parameters are derived from the collection's schema so the LLM
/// sees typed fields (not a generic "data" blob).
pub struct CollectionAddTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionAddTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_add", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionAddTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        // Can't return dynamic string from &str, so use a static prefix.
        // The tool name already encodes the collection.
        "Add a new record to this collection. Fields are validated against the schema."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for (field_name, field_def) in &self.schema.fields {
            let mut prop = field_type_to_json_schema(&field_def.field_type);
            if let Some(ref default) = field_def.default
                && let Some(obj) = prop.as_object_mut()
            {
                obj.insert("default".to_string(), default.clone());
            }
            properties.insert(field_name.clone(), prop);
            if field_def.required {
                required.push(json!(field_name));
            }
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let id = self
            .db
            .insert_record(&ctx.user_id, &self.schema.collection, params)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Insert failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "created",
                "record_id": id.to_string(),
                "collection": self.schema.collection,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for updating a record in a specific collection.
pub struct CollectionUpdateTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionUpdateTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_update", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionUpdateTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Update an existing record. Provide the record_id and only the fields you want to change."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();

        // record_id is always required
        properties.insert(
            "record_id".to_string(),
            json!({
                "type": "string",
                "description": "The ID of the record to update"
            }),
        );

        // All collection fields are optional for updates
        for (field_name, field_def) in &self.schema.fields {
            properties.insert(field_name.clone(), field_type_to_json_schema(&field_def.field_type));
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": ["record_id"],
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(&params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid record_id: {e}"))
        })?;

        // Extract only the collection fields (not record_id) for the update
        let mut updates = serde_json::Map::new();
        if let Some(obj) = params.as_object() {
            for (key, value) in obj {
                if key != "record_id" {
                    updates.insert(key.clone(), value.clone());
                }
            }
        }

        if updates.is_empty() {
            return Err(ToolError::InvalidParameters(
                "No fields to update provided".to_string(),
            ));
        }

        self.db
            .update_record(
                &ctx.user_id,
                record_id,
                serde_json::Value::Object(updates),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Update failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "updated",
                "record_id": record_id_str,
                "collection": self.schema.collection,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for deleting a record from a specific collection.
pub struct CollectionDeleteTool {
    tool_name: String,
    collection_name: String,
    db: Arc<dyn Database>,
}

impl CollectionDeleteTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_delete", schema.collection);
        Self {
            tool_name,
            collection_name: schema.collection,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionDeleteTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Delete a record by its ID. This action cannot be undone."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "record_id": {
                    "type": "string",
                    "description": "The ID of the record to delete"
                }
            },
            "required": ["record_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(&params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid record_id: {e}"))
        })?;

        self.db
            .delete_record(&ctx.user_id, record_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Delete failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "deleted",
                "record_id": record_id_str,
                "collection": self.collection_name,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for querying records from a specific collection.
pub struct CollectionQueryTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionQueryTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_query", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionQueryTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Query records with optional filters, ordering, and limit. \
         Returns matching records sorted by the specified field or by creation date."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let field_names: Vec<&str> = self.schema.fields.keys().map(|s| s.as_str()).collect();

        json!({
            "type": "object",
            "properties": {
                "filters": {
                    "type": "array",
                    "description": "Optional filters to apply",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": {
                                "type": "string",
                                "enum": field_names,
                                "description": "Field to filter on"
                            },
                            "op": {
                                "type": "string",
                                "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "is_null", "is_not_null"],
                                "description": "Filter operation"
                            },
                            "value": {
                                "description": "Value to compare against"
                            }
                        },
                        "required": ["field", "op"]
                    }
                },
                "order_by": {
                    "type": "string",
                    "enum": field_names,
                    "description": "Field to order results by (default: creation date descending)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 50, max: 200)",
                    "default": 50,
                    "minimum": 1,
                    "maximum": 200
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse filters
        let filters: Vec<Filter> = if let Some(filters_val) = params.get("filters") {
            serde_json::from_value(filters_val.clone()).map_err(|e| {
                ToolError::InvalidParameters(format!("Invalid filters: {e}"))
            })?
        } else {
            Vec::new()
        };

        let order_by = params.get("order_by").and_then(|v| v.as_str());
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50)
            .min(200) as usize;

        let records = self
            .db
            .query_records(
                &ctx.user_id,
                &self.schema.collection,
                &filters,
                order_by,
                limit,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Query failed: {e}")))?;

        let results: Vec<serde_json::Value> = records
            .iter()
            .map(|r| {
                json!({
                    "id": r.id.to_string(),
                    "data": r.data,
                    "created_at": r.created_at.to_rfc3339(),
                    "updated_at": r.updated_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(ToolOutput::success(
            json!({
                "collection": self.schema.collection,
                "results": results,
                "count": results.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for running aggregation queries on a specific collection.
pub struct CollectionSummaryTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionSummaryTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_summary", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionSummaryTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Summarize records with aggregation operations like sum, count, average, \
         min, or max. Optionally group results by a field and filter before aggregating."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let field_names: Vec<&str> = self.schema.fields.keys().map(|s| s.as_str()).collect();
        let numeric_fields: Vec<&str> = self
            .schema
            .fields
            .iter()
            .filter(|(_, def)| matches!(def.field_type, FieldType::Number))
            .map(|(name, _)| name.as_str())
            .collect();

        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["sum", "count", "avg", "min", "max"],
                    "description": "The aggregation operation to perform"
                },
                "field": {
                    "type": "string",
                    "enum": if numeric_fields.is_empty() { field_names.clone() } else { numeric_fields },
                    "description": "The field to aggregate (required for sum/avg/min/max, optional for count)"
                },
                "group_by": {
                    "type": "string",
                    "enum": field_names,
                    "description": "Optional field to group results by"
                },
                "filters": {
                    "type": "array",
                    "description": "Optional filters to apply before aggregating",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": {
                                "type": "string",
                                "enum": field_names,
                                "description": "Field to filter on"
                            },
                            "op": {
                                "type": "string",
                                "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "is_null", "is_not_null"],
                                "description": "Filter operation"
                            },
                            "value": {
                                "description": "Value to compare against"
                            }
                        },
                        "required": ["field", "op"]
                    }
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let op_str = require_str(&params, "operation")?;
        let operation: AggOp = serde_json::from_value(json!(op_str)).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid operation: {e}"))
        })?;

        let field = params.get("field").and_then(|v| v.as_str()).map(String::from);
        let group_by = params
            .get("group_by")
            .and_then(|v| v.as_str())
            .map(String::from);

        let filters: Vec<Filter> = if let Some(filters_val) = params.get("filters") {
            serde_json::from_value(filters_val.clone()).map_err(|e| {
                ToolError::InvalidParameters(format!("Invalid filters: {e}"))
            })?
        } else {
            Vec::new()
        };

        let aggregation = Aggregation {
            operation,
            field,
            group_by,
            filters,
        };

        let result = self
            .db
            .aggregate(&ctx.user_id, &self.schema.collection, &aggregation)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Aggregation failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "collection": self.schema.collection,
                "aggregation": result,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_type_to_json_schema_text() {
        let schema = field_type_to_json_schema(&FieldType::Text);
        assert_eq!(schema["type"], "string");
    }

    #[test]
    fn field_type_to_json_schema_number() {
        let schema = field_type_to_json_schema(&FieldType::Number);
        assert_eq!(schema["type"], "number");
    }

    #[test]
    fn field_type_to_json_schema_bool() {
        let schema = field_type_to_json_schema(&FieldType::Bool);
        assert_eq!(schema["type"], "boolean");
    }

    #[test]
    fn field_type_to_json_schema_enum() {
        let schema = field_type_to_json_schema(&FieldType::Enum {
            values: vec!["a".to_string(), "b".to_string()],
        });
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["enum"], json!(["a", "b"]));
    }

    #[test]
    fn field_type_to_json_schema_date() {
        let schema = field_type_to_json_schema(&FieldType::Date);
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["format"], "date");
    }

    #[test]
    fn field_type_to_json_schema_datetime() {
        let schema = field_type_to_json_schema(&FieldType::DateTime);
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["format"], "date-time");
    }

    #[test]
    fn field_type_to_json_schema_time() {
        let schema = field_type_to_json_schema(&FieldType::Time);
        assert_eq!(schema["type"], "string");
    }
}
