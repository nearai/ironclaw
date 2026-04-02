//! Unified collection tool — experiment branch.
//!
//! When `COLLECTION_TOOL_MODE=unified`, each collection gets ONE tool
//! (`{owner}_{collection}`) with an `operation` enum parameter instead
//! of five separate tools per collection.
//!
//! This reduces tool count from 5N to N while keeping collection-specific
//! tool names so the LLM still knows which collection to target.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::broadcast;

use crate::agent::collection_events::CollectionWriteEvent;
use crate::context::JobContext;
use crate::db::Database;
use crate::db::structured::{
    AggOp, Aggregation, CollectionSchema, FieldType, Filter, append_history, init_history,
};
use crate::tools::tool::{Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str};

use super::collections::resolve_collection_scope;

/// Check whether unified collection tool mode is enabled.
pub fn is_unified_mode() -> bool {
    std::env::var("COLLECTION_TOOL_MODE")
        .map(|v| v == "unified")
        .unwrap_or(false)
}

/// Generate a single unified tool for a collection schema.
///
/// Returns a `Vec<Arc<dyn Tool>>` with exactly one element for API
/// compatibility with `generate_collection_tools`.
pub fn generate_unified_collection_tool(
    schema: &CollectionSchema,
    db: Arc<dyn Database>,
    collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
    owner_user_id: &str,
) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(UnifiedCollectionTool::new(
        schema.clone(),
        db,
        collection_write_tx,
        owner_user_id,
    ))]
}

/// A single tool that handles all CRUD operations for one collection.
///
/// The `operation` parameter selects which action to perform:
/// `query`, `add`, `update`, `delete`, or `summary`.
pub struct UnifiedCollectionTool {
    tool_name: String,
    tool_description: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
    collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
    owner_user_id: String,
}

impl UnifiedCollectionTool {
    pub fn new(
        schema: CollectionSchema,
        db: Arc<dyn Database>,
        collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
        owner_user_id: &str,
    ) -> Self {
        let scope = schema.source_scope.as_deref().unwrap_or(owner_user_id);
        let tool_name = format!("{}_{}", scope, schema.collection);

        let human_name = schema.collection.replace('_', " ");
        let desc = schema
            .description
            .as_deref()
            .unwrap_or("Structured data collection");

        let tool_description = format!(
            "Manage {human_name} records ({desc}). \
             Use operation='add' to create, 'query' to search/list, \
             'update' to modify by record_id, 'delete' to remove by record_id, \
             'summary' for aggregations (count/sum/avg/min/max)."
        );

        Self {
            tool_name,
            tool_description,
            schema,
            db,
            collection_write_tx,
            owner_user_id: owner_user_id.to_string(),
        }
    }

    fn owner_scope<'a>(&'a self, ctx: &'a JobContext) -> &'a str {
        self.schema.source_scope.as_deref().unwrap_or(&ctx.user_id)
    }

    // ---- Operation implementations ----

    async fn execute_add(
        &self,
        params: &serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let mut data = params
            .get("data")
            .cloned()
            .unwrap_or_else(|| json!({}));

        if !data.is_object() {
            return Err(ToolError::InvalidParameters(
                "'data' must be an object".to_string(),
            ));
        }

        // Inject _lineage for provenance tracking.
        if let serde_json::Value::Object(ref mut obj) = data {
            obj.insert(
                "_lineage".to_string(),
                json!({
                    "source": "conversation",
                    "created_by": "user",
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }),
            );
        }

        // Inject _history for audit trail.
        init_history(&mut data, "conversation");

        let data_for_event = data.clone();

        let id = self
            .db
            .insert_record(self.owner_scope(ctx), &self.schema.collection, data)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to insert record: {e}")))?;

        // Fire collection write triggers.
        if let Some(tx) = &self.collection_write_tx {
            let _ = tx.send(CollectionWriteEvent {
                user_id: self.owner_scope(ctx).to_string(),
                collection: self.schema.collection.clone(),
                record_id: id,
                operation: "insert".to_string(),
                data: data_for_event,
            });
        }

        Ok(ToolOutput::success(
            json!({
                "status": "created",
                "record_id": id.to_string(),
                "collection": self.schema.collection,
            }),
            start.elapsed(),
        ))
    }

    async fn execute_update(
        &self,
        params: &serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid record_id: {e}")))?;

        let data = params
            .get("data")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let updates: serde_json::Map<String, serde_json::Value> = match data.as_object() {
            Some(obj) => obj.clone(),
            None => {
                return Err(ToolError::InvalidParameters(
                    "'data' must be an object".to_string(),
                ));
            }
        };

        if updates.is_empty() {
            return Err(ToolError::InvalidParameters(
                "No fields to update provided in 'data'".to_string(),
            ));
        }

        // Fetch existing record to get current _history, then append update entry.
        let existing = self
            .db
            .get_record(self.owner_scope(ctx), record_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch record: {e}")))?;

        let changed_fields = serde_json::Value::Object(updates.clone());
        let mut existing_data = existing.data;
        append_history(&mut existing_data, &changed_fields, "conversation");

        let mut final_updates = updates;
        if let Some(history) = existing_data.get("_history") {
            final_updates.insert("_history".to_string(), history.clone());
        }

        self.db
            .update_record(
                self.owner_scope(ctx),
                record_id,
                serde_json::Value::Object(final_updates),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to update record: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "updated",
                "record_id": record_id_str,
                "collection": self.schema.collection,
            }),
            start.elapsed(),
        ))
    }

    async fn execute_delete(
        &self,
        params: &serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid record_id: {e}")))?;

        self.db
            .delete_record(self.owner_scope(ctx), record_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to delete record: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "deleted",
                "record_id": record_id_str,
                "collection": self.schema.collection,
            }),
            start.elapsed(),
        ))
    }

    async fn execute_query(
        &self,
        params: &serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse filters
        let filters: Vec<Filter> = match params.get("filters") {
            Some(v) if v.is_array() => serde_json::from_value(v.clone())
                .map_err(|e| ToolError::InvalidParameters(format!("Invalid filters: {e}")))?,
            Some(v) if v.is_string() => {
                let s = v.as_str().unwrap_or("[]");
                if s == "{}" || s.trim().is_empty() {
                    Vec::new()
                } else {
                    serde_json::from_str(s).map_err(|e| {
                        ToolError::InvalidParameters(format!("Invalid filters string: {e}"))
                    })?
                }
            }
            _ => Vec::new(),
        };

        let order_by = params.get("order_by").and_then(|v| v.as_str());

        let limit = params
            .get("limit")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
            })
            .unwrap_or(50)
            .min(200) as usize;

        // Resolve scope
        let owner = if self.schema.source_scope.is_some() {
            self.owner_scope(ctx).to_string()
        } else {
            resolve_collection_scope(
                self.db.as_ref(),
                &ctx.user_id,
                &[],
                &self.schema.collection,
            )
            .await
            .unwrap_or_else(|| ctx.user_id.clone())
        };

        let records = self
            .db
            .query_records(&owner, &self.schema.collection, &filters, order_by, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to query records: {e}")))?;

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

    async fn execute_summary(
        &self,
        params: &serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let agg_op_str = params
            .get("agg_operation")
            .and_then(|v| v.as_str())
            .unwrap_or("count");

        let agg_op: AggOp = serde_json::from_value(json!(agg_op_str))
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid agg_operation: {e}")))?;

        let field = params
            .get("field")
            .and_then(|v| v.as_str())
            .map(String::from);
        let group_by = params
            .get("group_by")
            .and_then(|v| v.as_str())
            .map(String::from);

        let filters: Vec<Filter> = match params.get("filters") {
            Some(v) if v.is_array() => serde_json::from_value(v.clone())
                .map_err(|e| ToolError::InvalidParameters(format!("Invalid filters: {e}")))?,
            Some(v) if v.is_string() => {
                let s = v.as_str().unwrap_or("[]");
                if s == "{}" || s.trim().is_empty() {
                    Vec::new()
                } else {
                    serde_json::from_str(s).map_err(|e| {
                        ToolError::InvalidParameters(format!("Invalid filters string: {e}"))
                    })?
                }
            }
            _ => Vec::new(),
        };

        // Validate that sum/avg operations target numeric fields.
        if matches!(agg_op, AggOp::Sum | AggOp::Avg)
            && let Some(ref f) = field
            && let Some(def) = self.schema.fields.get(f)
            && !matches!(def.field_type, FieldType::Number)
        {
            return Err(ToolError::InvalidParameters(format!(
                "Cannot use {agg_op_str} on non-numeric field '{f}'"
            )));
        }

        let aggregation = Aggregation {
            operation: agg_op,
            field,
            group_by,
            filters,
        };

        let owner = if self.schema.source_scope.is_some() {
            self.owner_scope(ctx).to_string()
        } else {
            resolve_collection_scope(
                self.db.as_ref(),
                &ctx.user_id,
                &[],
                &self.schema.collection,
            )
            .await
            .unwrap_or_else(|| ctx.user_id.clone())
        };

        let result = self
            .db
            .aggregate(&owner, &self.schema.collection, &aggregation)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to aggregate: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "collection": self.schema.collection,
                "aggregation": result,
            }),
            start.elapsed(),
        ))
    }
}

#[async_trait]
impl Tool for UnifiedCollectionTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn owner_user_id(&self) -> Option<&str> {
        Some(&self.owner_user_id)
    }

    fn parameters_schema(&self) -> serde_json::Value {
        // Build field descriptions for the data object hint
        let fields_hint: String = self
            .schema
            .fields
            .iter()
            .map(|(fname, fdef)| {
                let type_str = match &fdef.field_type {
                    FieldType::Text => "string",
                    FieldType::Number => "number",
                    FieldType::Date => "date (YYYY-MM-DD)",
                    FieldType::Time => "time (HH:MM)",
                    FieldType::DateTime => "datetime (ISO 8601)",
                    FieldType::Bool => "boolean",
                    FieldType::Enum { .. } => "enum",
                };
                let req = if fdef.required { ", required" } else { "" };
                let extra = match &fdef.field_type {
                    FieldType::Enum { values } => format!(", values: {}", values.join("/")),
                    _ => String::new(),
                };
                format!("{fname} ({type_str}{req}{extra})")
            })
            .collect::<Vec<_>>()
            .join("; ");

        // Build field name list for summary
        let field_names: Vec<&str> = self.schema.fields.keys().map(|s| s.as_str()).collect();

        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["query", "add", "update", "delete", "summary"],
                    "description": "The operation to perform on this collection"
                },
                "data": {
                    "type": "object",
                    "description": format!("Record data for add/update. Fields: {fields_hint}")
                },
                "record_id": {
                    "type": "string",
                    "description": "Record ID (required for update and delete)"
                },
                "filters": {
                    "type": "array",
                    "description": "Filters for query/summary. Each: {field, op (eq/neq/gt/gte/lt/lte/is_null/is_not_null), value}",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": { "type": "string" },
                            "op": {
                                "type": "string",
                                "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "is_null", "is_not_null"]
                            },
                            "value": { "description": "Value to compare against" }
                        },
                        "required": ["field", "op"]
                    }
                },
                "order_by": {
                    "type": "string",
                    "description": "Field to order query results by"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results for query (default 50, max 200)",
                    "default": 50,
                    "minimum": 1,
                    "maximum": 200
                },
                "agg_operation": {
                    "type": "string",
                    "enum": ["sum", "count", "avg", "min", "max"],
                    "description": "Aggregation type (for operation=summary, default: count)"
                },
                "field": {
                    "type": "string",
                    "enum": field_names,
                    "description": "Field to aggregate (for summary)"
                },
                "group_by": {
                    "type": "string",
                    "description": "Group results by this field (for summary)"
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
        let operation = require_str(&params, "operation")?;

        match operation {
            "add" => self.execute_add(&params, ctx).await,
            "update" => self.execute_update(&params, ctx).await,
            "delete" => self.execute_delete(&params, ctx).await,
            "query" => self.execute_query(&params, ctx).await,
            "summary" => self.execute_summary(&params, ctx).await,
            other => Err(ToolError::InvalidParameters(format!(
                "Unknown operation '{other}'. Must be one of: query, add, update, delete, summary"
            ))),
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::structured::FieldDef;
    use std::collections::BTreeMap;

    // ==================== Helpers ====================

    /// Create a test database (file-backed libsql) and return it with a TempDir guard.
    async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
        crate::testing::test_db().await
    }

    fn test_ctx(user_id: &str) -> crate::context::JobContext {
        crate::context::JobContext {
            user_id: user_id.to_string(),
            ..Default::default()
        }
    }

    /// Grocery schema with text, enum, bool, and number fields.
    fn grocery_schema() -> CollectionSchema {
        serde_json::from_value(json!({
            "collection": "grocery_items",
            "description": "Tracks grocery items",
            "fields": {
                "name": { "type": "text", "required": true },
                "category": {
                    "type": "enum",
                    "values": ["produce", "dairy", "meat", "pantry", "frozen", "household", "other"]
                },
                "on_list": { "type": "bool", "default": true },
                "quantity": { "type": "number" },
                "notes": { "type": "text" }
            }
        }))
        .expect("grocery schema should parse")
    }

    /// Nanny shifts schema with date, datetime, enum, and number fields.
    fn nanny_schema() -> CollectionSchema {
        serde_json::from_value(json!({
            "collection": "nanny_shifts",
            "description": "Tracks nanny working shifts",
            "fields": {
                "date": { "type": "date", "required": true },
                "start_time": { "type": "date_time", "required": true },
                "end_time": { "type": "date_time" },
                "status": {
                    "type": "enum",
                    "values": ["in_progress", "completed"],
                    "default": "in_progress"
                },
                "hours": { "type": "number" },
                "notes": { "type": "text" }
            }
        }))
        .expect("nanny schema should parse")
    }

    // ==================== Unit Tests ====================

    #[test]
    fn is_unified_mode_reads_env() {
        // SAFETY: Tests run single-threaded for this env var; no other thread
        // reads COLLECTION_TOOL_MODE concurrently in unit tests.
        unsafe {
            // With no env var set, default is false.
            std::env::remove_var("COLLECTION_TOOL_MODE");
            assert!(!is_unified_mode());

            // Set to "unified" — should return true.
            std::env::set_var("COLLECTION_TOOL_MODE", "unified");
            assert!(is_unified_mode());

            // Set to something else — should return false.
            std::env::set_var("COLLECTION_TOOL_MODE", "classic");
            assert!(!is_unified_mode());

            // Clean up.
            std::env::remove_var("COLLECTION_TOOL_MODE");
        }
    }

    #[tokio::test]
    async fn tool_name_includes_owner_prefix() {
        let schema = CollectionSchema {
            collection: "grocery_items".to_string(),
            description: Some("Grocery shopping list".to_string()),
            fields: BTreeMap::new(),
            source_scope: None,
        };
        let (db, _dir) = setup().await;
        let tool = UnifiedCollectionTool::new(schema, db, None, "andrew");
        assert_eq!(tool.name(), "andrew_grocery_items");
    }

    #[tokio::test]
    async fn tool_name_with_source_scope() {
        let schema = CollectionSchema {
            collection: "tasks".to_string(),
            description: None,
            fields: BTreeMap::new(),
            source_scope: Some("household".to_string()),
        };
        let (db, _dir) = setup().await;
        let tool = UnifiedCollectionTool::new(schema, db, None, "andrew");
        // source_scope overrides owner in the tool name.
        assert_eq!(tool.name(), "household_tasks");
    }

    #[tokio::test]
    async fn parameters_schema_has_operation_enum() {
        let schema = CollectionSchema {
            collection: "test".to_string(),
            description: None,
            fields: BTreeMap::new(),
            source_scope: None,
        };
        let (db, _dir) = setup().await;
        let tool = UnifiedCollectionTool::new(schema, db, None, "owner");
        let params = tool.parameters_schema();

        // operation must be required
        let required = params["required"].as_array().unwrap();
        assert!(required.contains(&json!("operation")));

        // operation must be a string enum with all 5 values
        let op = &params["properties"]["operation"];
        assert_eq!(op["type"], "string");
        let ops = op["enum"].as_array().unwrap();
        assert_eq!(ops.len(), 5);
        assert!(ops.contains(&json!("query")));
        assert!(ops.contains(&json!("add")));
        assert!(ops.contains(&json!("update")));
        assert!(ops.contains(&json!("delete")));
        assert!(ops.contains(&json!("summary")));
    }

    #[tokio::test]
    async fn parameters_schema_has_all_optional_fields() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "amount".to_string(),
            FieldDef {
                field_type: FieldType::Number,
                required: false,
                default: None,
            },
        );
        let schema = CollectionSchema {
            collection: "test".to_string(),
            description: None,
            fields,
            source_scope: None,
        };
        let (db, _dir) = setup().await;
        let tool = UnifiedCollectionTool::new(schema, db, None, "owner");
        let params = tool.parameters_schema();
        let props = params["properties"].as_object().unwrap();

        // All expected optional fields must be present.
        assert!(props.contains_key("data"), "missing 'data' property");
        assert!(props.contains_key("record_id"), "missing 'record_id' property");
        assert!(props.contains_key("filters"), "missing 'filters' property");
        assert!(props.contains_key("field"), "missing 'field' property");
        assert!(props.contains_key("group_by"), "missing 'group_by' property");
        assert!(props.contains_key("agg_operation"), "missing 'agg_operation' property");
        assert!(props.contains_key("order_by"), "missing 'order_by' property");
        assert!(props.contains_key("limit"), "missing 'limit' property");

        // Only "operation" should be required.
        let required = params["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "operation");
    }

    #[tokio::test]
    async fn generate_unified_returns_one_tool() {
        let schema = CollectionSchema {
            collection: "items".to_string(),
            description: None,
            fields: BTreeMap::new(),
            source_scope: None,
        };
        let (db, _dir) = setup().await;
        let tools = generate_unified_collection_tool(&schema, db, None, "user1");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "user1_items");
    }

    // ==================== Integration Tests ====================

    #[tokio::test]
    async fn unified_add_inserts_record() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        let result = tool
            .execute(
                json!({
                    "operation": "add",
                    "data": { "name": "milk", "category": "dairy" }
                }),
                &ctx,
            )
            .await
            .expect("add should succeed");

        assert_eq!(result.result["status"], "created");
        let record_id = result.result["record_id"].as_str().unwrap();
        assert!(!record_id.is_empty());

        // Verify the record is in the DB.
        let id = uuid::Uuid::parse_str(record_id).unwrap();
        let record = db.get_record("andrew", id).await.expect("record should exist");
        assert_eq!(record.data["name"], "milk");
        assert_eq!(record.data["category"], "dairy");
    }

    #[tokio::test]
    async fn unified_query_returns_records() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Add two records.
        tool.execute(
            json!({ "operation": "add", "data": { "name": "milk" } }),
            &ctx,
        )
        .await
        .unwrap();
        tool.execute(
            json!({ "operation": "add", "data": { "name": "bread" } }),
            &ctx,
        )
        .await
        .unwrap();

        // Query all.
        let result = tool
            .execute(json!({ "operation": "query" }), &ctx)
            .await
            .expect("query should succeed");

        assert_eq!(result.result["count"], 2);
        let records = result.result["results"].as_array().unwrap();
        assert_eq!(records.len(), 2);
    }

    #[tokio::test]
    async fn unified_update_modifies_record() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Add a record.
        let add_result = tool
            .execute(
                json!({ "operation": "add", "data": { "name": "milk", "quantity": 1 } }),
                &ctx,
            )
            .await
            .unwrap();
        let record_id = add_result.result["record_id"].as_str().unwrap().to_string();

        // Update it.
        let update_result = tool
            .execute(
                json!({
                    "operation": "update",
                    "record_id": record_id,
                    "data": { "quantity": 3 }
                }),
                &ctx,
            )
            .await
            .expect("update should succeed");

        assert_eq!(update_result.result["status"], "updated");

        // Verify change persisted.
        let id = uuid::Uuid::parse_str(&record_id).unwrap();
        let record = db.get_record("andrew", id).await.unwrap();
        assert_eq!(record.data["quantity"], 3);
        // Original field preserved.
        assert_eq!(record.data["name"], "milk");
    }

    #[tokio::test]
    async fn unified_delete_removes_record() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Add and then delete.
        let add_result = tool
            .execute(
                json!({ "operation": "add", "data": { "name": "eggs" } }),
                &ctx,
            )
            .await
            .unwrap();
        let record_id = add_result.result["record_id"].as_str().unwrap().to_string();

        let delete_result = tool
            .execute(
                json!({ "operation": "delete", "record_id": record_id }),
                &ctx,
            )
            .await
            .expect("delete should succeed");

        assert_eq!(delete_result.result["status"], "deleted");

        // Verify it is gone.
        let id = uuid::Uuid::parse_str(&record_id).unwrap();
        assert!(db.get_record("andrew", id).await.is_err());
    }

    #[tokio::test]
    async fn unified_summary_counts_records() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Add 3 records.
        for name in ["milk", "bread", "eggs"] {
            tool.execute(
                json!({ "operation": "add", "data": { "name": name } }),
                &ctx,
            )
            .await
            .unwrap();
        }

        // Summary with count.
        let result = tool
            .execute(
                json!({
                    "operation": "summary",
                    "agg_operation": "count",
                    "field": "name"
                }),
                &ctx,
            )
            .await
            .expect("summary should succeed");

        let agg = &result.result["aggregation"];
        let count: f64 = agg
            .as_f64()
            .unwrap_or_else(|| agg.as_str().unwrap_or("0").parse().unwrap_or(0.0));
        assert!((count - 3.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn unified_summary_sums_numeric_field() {
        let (db, _dir) = setup().await;
        let schema = nanny_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Add shifts with hours.
        for hours in [8.0, 7.5, 9.0] {
            tool.execute(
                json!({
                    "operation": "add",
                    "data": {
                        "date": "2026-02-22",
                        "start_time": "2026-02-22T08:00:00Z",
                        "hours": hours,
                        "status": "completed"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        }

        // Sum hours.
        let result = tool
            .execute(
                json!({
                    "operation": "summary",
                    "agg_operation": "sum",
                    "field": "hours"
                }),
                &ctx,
            )
            .await
            .expect("summary sum should succeed");

        let agg = &result.result["aggregation"];
        let total: f64 = agg
            .as_f64()
            .unwrap_or_else(|| agg.as_str().unwrap_or("0").parse().unwrap_or(0.0));
        assert!((total - 24.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn unified_query_with_filters() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Add items in different categories.
        tool.execute(
            json!({ "operation": "add", "data": { "name": "milk", "category": "dairy" } }),
            &ctx,
        )
        .await
        .unwrap();
        tool.execute(
            json!({ "operation": "add", "data": { "name": "yogurt", "category": "dairy" } }),
            &ctx,
        )
        .await
        .unwrap();
        tool.execute(
            json!({ "operation": "add", "data": { "name": "bread", "category": "pantry" } }),
            &ctx,
        )
        .await
        .unwrap();

        // Query with filter for dairy only.
        let result = tool
            .execute(
                json!({
                    "operation": "query",
                    "filters": [{ "field": "category", "op": "eq", "value": "dairy" }]
                }),
                &ctx,
            )
            .await
            .expect("filtered query should succeed");

        assert_eq!(result.result["count"], 2);
        let records = result.result["results"].as_array().unwrap();
        for r in records {
            assert_eq!(r["data"]["category"], "dairy");
        }
    }

    #[tokio::test]
    async fn unified_invalid_operation_returns_error() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        let err = tool
            .execute(json!({ "operation": "invalid" }), &ctx)
            .await;

        assert!(err.is_err());
        let msg = format!("{}", err.unwrap_err());
        assert!(
            msg.contains("Unknown operation"),
            "error should mention unknown operation, got: {msg}"
        );
    }

    #[tokio::test]
    async fn unified_add_validates_against_schema() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        db.register_collection("andrew", &schema)
            .await
            .expect("register schema");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Add without the required "name" field — DB-level validation should reject.
        let err = tool
            .execute(
                json!({ "operation": "add", "data": { "category": "dairy" } }),
                &ctx,
            )
            .await;

        assert!(
            err.is_err(),
            "add without required field should fail, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn unified_update_requires_record_id() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Update without record_id.
        let err = tool
            .execute(
                json!({ "operation": "update", "data": { "name": "oats" } }),
                &ctx,
            )
            .await;

        assert!(err.is_err(), "update without record_id should fail");
    }

    #[tokio::test]
    async fn unified_delete_requires_record_id() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();
        let ctx = test_ctx("andrew");

        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        // Delete without record_id.
        let err = tool.execute(json!({ "operation": "delete" }), &ctx).await;

        assert!(err.is_err(), "delete without record_id should fail");
    }

    #[tokio::test]
    async fn unified_user_isolation() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();

        db.register_collection("andrew", &schema)
            .await
            .expect("register for andrew");
        db.register_collection("grace", &schema)
            .await
            .expect("register for grace");

        let andrew_tool =
            UnifiedCollectionTool::new(schema.clone(), Arc::clone(&db), None, "andrew");
        let grace_tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "grace");

        let andrew_ctx = test_ctx("andrew");
        let grace_ctx = test_ctx("grace");

        // Andrew adds 2 items.
        andrew_tool
            .execute(
                json!({ "operation": "add", "data": { "name": "waffles" } }),
                &andrew_ctx,
            )
            .await
            .unwrap();
        andrew_tool
            .execute(
                json!({ "operation": "add", "data": { "name": "milk" } }),
                &andrew_ctx,
            )
            .await
            .unwrap();

        // Grace adds 1 item.
        grace_tool
            .execute(
                json!({ "operation": "add", "data": { "name": "yogurt" } }),
                &grace_ctx,
            )
            .await
            .unwrap();

        // Andrew sees only his 2 records.
        let andrew_result = andrew_tool
            .execute(json!({ "operation": "query" }), &andrew_ctx)
            .await
            .unwrap();
        assert_eq!(andrew_result.result["count"], 2);

        // Grace sees only her 1 record.
        let grace_result = grace_tool
            .execute(json!({ "operation": "query" }), &grace_ctx)
            .await
            .unwrap();
        assert_eq!(grace_result.result["count"], 1);

        // Verify the data content is correct.
        let grace_records = grace_result.result["results"].as_array().unwrap();
        assert_eq!(grace_records[0]["data"]["name"], "yogurt");
    }

    /// A unified tool created for andrew (owner_user_id="andrew") should not
    /// leak data when executed with a JobContext for grace. The tool uses
    /// `owner_scope(ctx)` which resolves to `ctx.user_id` when source_scope
    /// is None, so grace's context should query grace's scope — not andrew's.
    #[tokio::test]
    async fn wrong_context_tool_isolation() {
        let (db, _dir) = setup().await;
        let schema = grocery_schema();

        // Register collection for andrew only — grace has NO collection.
        db.register_collection("andrew", &schema)
            .await
            .expect("register for andrew");

        // Create tool owned by andrew.
        let tool = UnifiedCollectionTool::new(schema, Arc::clone(&db), None, "andrew");

        let andrew_ctx = test_ctx("andrew");

        // Andrew adds records through the tool.
        tool.execute(
            json!({ "operation": "add", "data": { "name": "steak" } }),
            &andrew_ctx,
        )
        .await
        .expect("andrew add steak");

        tool.execute(
            json!({ "operation": "add", "data": { "name": "wine" } }),
            &andrew_ctx,
        )
        .await
        .expect("andrew add wine");

        // Now execute the SAME tool with grace's context.
        // Since source_scope is None, owner_scope(ctx) returns ctx.user_id = "grace".
        // Grace has no collection registered, but query_records will just return
        // empty for a non-existent collection/user pair rather than andrew's data.
        let grace_ctx = test_ctx("grace");
        let grace_result = tool
            .execute(json!({ "operation": "query" }), &grace_ctx)
            .await
            .expect("query with grace ctx should succeed");

        // Grace must see 0 records — not andrew's 2 records.
        assert_eq!(
            grace_result.result["count"], 0,
            "grace should not see andrew's records when using andrew's tool with grace's context"
        );
    }
}
